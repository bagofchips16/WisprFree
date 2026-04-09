#![windows_subsystem = "windows"]

//! WisprFree – Local, private voice-to-text for Windows.
//!
//! Hold **Ctrl+Space** and speak → text appears wherever your cursor is.
//! No cloud, no API, no internet required.
//!
//! ## Architecture
//!
//! ```text
//! ┌─────────────────────────────────────────────────┐
//! │  main thread  (Win32 message loop)              │
//! │   ├─ keyboard hook (Ctrl+Space press/release)   │
//! │   ├─ tray icon + menu                           │
//! │   └─ orchestration: start/stop/inject           │
//! ├─────────────────────────────────────────────────┤
//! │  cpal audio callback (OS thread)                │
//! │   └─ pushes f32 samples into shared buffer      │
//! ├─────────────────────────────────────────────────┤
//! │  transcription thread  (spawned per utterance)  │
//! │   └─ whisper inference → post-process → inject  │
//! └─────────────────────────────────────────────────┘
//! ```

mod audio;
mod autostart;
mod config;
mod dashboard;
mod dictionary;
mod history;
mod hotkey;
mod overlay;
mod paster;
mod punctuation;
mod shortcut;
mod snippets;
mod transcriber;
mod tray;

use anyhow::{Context, Result};
use crossbeam_channel::{select, Receiver, Sender};
use std::sync::Arc;
use tempfile::NamedTempFile;
use windows::Win32::UI::WindowsAndMessaging::{
    DispatchMessageW, GetMessageW, TranslateMessage, MSG,
};

/// Minimum recording duration in seconds. Anything shorter is likely
/// an accidental tap and would produce garbage transcription.
const MIN_RECORDING_SECS: f32 = 0.5;

fn main() {
    // Write errors to a log file for debugging (windows_subsystem = "windows" hides stderr)
    let log_path = std::env::temp_dir().join("wisprfree_debug.log");
    if let Err(e) = run() {
        let msg = format!("WisprFree failed to start:\n\n{e:#}");
        let _ = std::fs::write(&log_path, &msg);
        eprintln!("fatal: {e:#}");
        // Show a message box so the user sees the error even without a console.
        let wide: Vec<u16> = msg.encode_utf16().chain(std::iter::once(0)).collect();
        let title: Vec<u16> = "WisprFree Error"
            .encode_utf16()
            .chain(std::iter::once(0))
            .collect();
        unsafe {
            windows::Win32::UI::WindowsAndMessaging::MessageBoxW(
                windows::Win32::Foundation::HWND::default(),
                windows::core::PCWSTR(wide.as_ptr()),
                windows::core::PCWSTR(title.as_ptr()),
                windows::Win32::UI::WindowsAndMessaging::MB_OK
                    | windows::Win32::UI::WindowsAndMessaging::MB_ICONERROR,
            );
        }
        std::process::exit(1);
    }
    // If we get here, app ran and exited normally — write success marker
    let _ = std::fs::write(&log_path, "WisprFree exited normally");
}

fn run() -> Result<()> {
    // ── Single-instance guard ─────────────────────────────────────────
    let _mutex = single_instance_lock()?;

    // ── Load configuration ────────────────────────────────────────────
    let cfg = config::load().context("failed to load config")?;

    // Init logger
    env_logger::Builder::new()
        .filter_level(
            cfg.general
                .log_level
                .parse()
                .unwrap_or(log::LevelFilter::Info),
        )
        .format_timestamp_millis()
        .init();

    log::info!("WisprFree v{} starting", env!("CARGO_PKG_VERSION"));

    // ── Create Start Menu shortcut (so users can search "WisprFree") ──
    shortcut::ensure_shortcut();

    // ── Resolve model path & load Whisper ─────────────────────────────
    let model_path = config::resolve_model_path(&cfg.whisper.model_path);
    let transcriber = Arc::new(
        transcriber::Transcriber::new(
            &model_path,
            &cfg.whisper.language,
            cfg.whisper.use_gpu,
            cfg.whisper.threads,
        )
        .context("failed to initialise Whisper")?,
    );

    // ── Load snippet library & personal dictionary ────────────────────
    let config_dir = config::config_dir()?;
    let mut snippet_lib =
        snippets::SnippetLibrary::load(config_dir.join("snippets.toml"))?;
    let mut personal_dict =
        dictionary::PersonalDictionary::load(config_dir.join("dictionary.toml"))?;

    // ── Set up audio capture ──────────────────────────────────────────
    let (audio_err_tx, audio_err_rx): (Sender<String>, Receiver<String>) =
        crossbeam_channel::unbounded();
    let capture = audio::AudioCapture::new(&cfg.audio.device_name, audio_err_tx)
        .context("failed to open audio device")?;
    let audio_shared = Arc::clone(&capture.shared);

    // ── Set up hotkey hook ────────────────────────────────────────────
    let (hk_tx, hk_rx): (Sender<hotkey::HotkeyEvent>, Receiver<hotkey::HotkeyEvent>) =
        crossbeam_channel::unbounded();
    hotkey::install(cfg.hotkey.vk_code, cfg.hotkey.ctrl, hk_tx)?;

    // ── Set up system tray ────────────────────────────────────────────
    let (tray_tx, tray_rx): (Sender<tray::TrayCommand>, Receiver<tray::TrayCommand>) =
        crossbeam_channel::unbounded();
    let _tray = tray::Tray::new(tray_tx)?;

    // ── Orchestration thread ──────────────────────────────────────────
    // Processes hotkey events and tray commands while the main thread
    // runs the Win32 message loop (required for the keyboard hook).
    let injection_method = cfg.injection.method.clone();
    let clipboard_delay = cfg.injection.clipboard_restore_delay_ms;

    let orch_capture = audio_shared;
    let orch_transcriber = Arc::clone(&transcriber);
    let orch_overlay = overlay::Overlay::new().context("failed to create overlay")?;

    // Start the dashboard HTTP server in the background and open it
    dashboard::start();
    // Give the server a moment to bind, then open in browser
    std::thread::spawn(|| {
        std::thread::sleep(std::time::Duration::from_millis(500));
        dashboard::open_in_browser();
    });

    // Show "Ready" notification so the user knows the app has started
    orch_overlay.set_state(overlay::OverlayState::Ready);

    std::thread::Builder::new().name("orchestrator".into()).spawn(move || {
        let mut recording_start: Option<std::time::Instant> = None;

        loop {
            select! {
                recv(hk_rx) -> msg => {
                    match msg {
                        Ok(hotkey::HotkeyEvent::PushDown) => {
                            log::info!("⏺  recording…");
                            recording_start = Some(std::time::Instant::now());
                            orch_overlay.set_state(overlay::OverlayState::Recording);
                            orch_capture.start_recording();
                        }
                        Ok(hotkey::HotkeyEvent::PushUp) => {
                            let duration = recording_start
                                .map(|s| s.elapsed().as_secs_f32())
                                .unwrap_or(0.0);
                            recording_start = None;

                            if duration < MIN_RECORDING_SECS {
                                log::info!("recording too short ({:.1}s < {:.1}s), skipping", duration, MIN_RECORDING_SECS);
                                let _ = orch_capture.stop_recording();
                                orch_overlay.set_state(overlay::OverlayState::Hidden);
                                continue;
                            }

                            orch_overlay.set_state(overlay::OverlayState::Processing);

                            log::info!("⏹  processing… ({:.1}s recorded)", duration);
                            match orch_capture.stop_recording() {
                                Ok(samples) if samples.is_empty() => {
                                    log::warn!("no audio captured");
                                    orch_overlay.set_state(overlay::OverlayState::Hidden);
                                    show_notification("WisprFree", "No audio captured. Check your microphone.");
                                }
                                Ok(samples) => {
                                    // Write samples to a temp WAV file
                                    let tmp = NamedTempFile::new()
                                        .expect("failed to create temp file");
                                    let wav_path = tmp.path().with_extension("wav");
                                    if let Err(e) = audio::write_wav(&samples, &wav_path) {
                                        log::error!("WAV write failed: {e:#}");
                                        continue;
                                    }
                                    match orch_transcriber.transcribe_file(&wav_path) {
                                        Ok(raw_text) if raw_text.is_empty() => {
                                            log::warn!("whisper returned empty text");
                                        }
                                        Ok(raw_text) => {
                                            // Post-processing pipeline
                                            let text = punctuation::auto_punctuate(&raw_text);
                                            let text = personal_dict.correct(&text);
                                            let text = snippet_lib.expand(&text);
                                            log::info!("💬  \"{}\"", text);

                                            if let Err(e) = paster::inject(
                                                &text,
                                                &injection_method,
                                                clipboard_delay,
                                            ) {
                                                log::error!("injection failed: {e:#}");
                                                show_notification("WisprFree", &format!("Text injection failed: {e}"));
                                            }

                                            // Log to history for dashboard analytics
                                            if let Err(e) = history::append(&text, duration, 0.0) {
                                                log::warn!("history log failed: {e:#}");
                                            }

                                            orch_overlay.set_state(overlay::OverlayState::Done);
                                        }
                                        Err(e) => {
                                            log::error!("transcription failed: {e:#}");
                                            orch_overlay.set_state(overlay::OverlayState::Hidden);
                                            show_notification("WisprFree", &format!("Transcription failed: {e}"));
                                        }
                                    }
                                    // Clean up temp WAV
                                    let _ = std::fs::remove_file(&wav_path);
                                }
                                Err(e) => log::error!("audio capture error: {e:#}"),
                            }
                        }
                        Err(_) => break, // channel closed
                    }
                }

                recv(tray_rx) -> msg => {
                    match msg {
                        Ok(tray::TrayCommand::Quit) => {
                            log::info!("quit requested");
                            // Post WM_QUIT to break the message loop
                            unsafe {
                                windows::Win32::UI::WindowsAndMessaging::PostQuitMessage(0);
                            }
                            break;
                        }
                        Ok(tray::TrayCommand::ReloadConfig) => {
                            log::info!("reloading config…");
                            if let Err(e) = snippet_lib.reload() {
                                log::error!("snippet reload: {e:#}");
                            }
                            if let Err(e) = personal_dict.reload() {
                                log::error!("dictionary reload: {e:#}");
                            }
                        }
                        Ok(tray::TrayCommand::OpenConfigFolder) => {
                            let dir = config::config_dir().unwrap_or_default();
                            let _ = std::process::Command::new("explorer")
                                .arg(dir.as_os_str())
                                .spawn();
                        }
                        Ok(tray::TrayCommand::OpenDashboard) => {
                            dashboard::open_in_browser();
                        }
                        Ok(tray::TrayCommand::ToggleAutostart) => {
                            let new_state = autostart::toggle();
                            log::info!("autostart toggled → {}", if new_state { "enabled" } else { "disabled" });
                        }
                        Ok(tray::TrayCommand::About) => {
                            let msg = format!(
                                "WisprFree v{}\n\n\
                                 Local voice-to-text for Windows.\n\
                                 Hold Ctrl+Space and speak.\n\n\
                                 No cloud, no API, no internet.\n\
                                 Nothing ever leaves your PC.",
                                env!("CARGO_PKG_VERSION")
                            );
                            let wide: Vec<u16> =
                                msg.encode_utf16().chain(std::iter::once(0)).collect();
                            let title: Vec<u16> = "About WisprFree"
                                .encode_utf16()
                                .chain(std::iter::once(0))
                                .collect();
                            unsafe {
                                windows::Win32::UI::WindowsAndMessaging::MessageBoxW(
                                    windows::Win32::Foundation::HWND::default(),
                                    windows::core::PCWSTR(wide.as_ptr()),
                                    windows::core::PCWSTR(title.as_ptr()),
                                    windows::Win32::UI::WindowsAndMessaging::MB_OK
                                        | windows::Win32::UI::WindowsAndMessaging::MB_ICONINFORMATION,
                                );
                            }
                        }
                        Err(_) => break,
                    }
                }

                recv(audio_err_rx) -> msg => {
                    if let Ok(err) = msg {
                        log::error!("audio: {err}");
                    }
                }
            }
        }
    });

    // ── Main thread: Win32 message loop ───────────────────────────────
    // Required for the low-level keyboard hook to receive callbacks.
    log::info!("ready – hold Ctrl+Space to dictate");
    unsafe {
        let mut msg = MSG::default();
        while GetMessageW(&mut msg, None, 0, 0).as_bool() {
            let _ = TranslateMessage(&msg);
            DispatchMessageW(&msg);
        }
    }

    let _capture = capture; // keep cpal Stream alive on main thread

    hotkey::uninstall();
    log::info!("WisprFree exiting");
    Ok(())
}

// ── Single-instance guard ─────────────────────────────────────────────

/// Creates a named mutex. If another instance already holds it, bail.
fn single_instance_lock() -> Result<windows::Win32::Foundation::HANDLE> {
    use windows::Win32::System::Threading::CreateMutexW;
    use windows::core::PCWSTR;

    let name: Vec<u16> = "WisprFree_SingleInstance\0"
        .encode_utf16()
        .collect();

    let handle = unsafe {
        CreateMutexW(None, true, PCWSTR(name.as_ptr()))
            .context("CreateMutexW failed")?
    };

    // ERROR_ALREADY_EXISTS = 183
    if unsafe { windows::Win32::Foundation::GetLastError() }.0 == 183 {
        // Ask user if they want to replace the running instance (common when updating)
        let msg = "WisprFree is already running.\n\n\
                   Do you want to close the old version and start this one?\n\n\
                   Click YES to restart, or NO to keep the current instance.";
        let wide: Vec<u16> = msg.encode_utf16().chain(std::iter::once(0)).collect();
        let title: Vec<u16> = "WisprFree".encode_utf16().chain(std::iter::once(0)).collect();
        let result = unsafe {
            windows::Win32::UI::WindowsAndMessaging::MessageBoxW(
                windows::Win32::Foundation::HWND::default(),
                windows::core::PCWSTR(wide.as_ptr()),
                windows::core::PCWSTR(title.as_ptr()),
                windows::Win32::UI::WindowsAndMessaging::MB_YESNO
                    | windows::Win32::UI::WindowsAndMessaging::MB_ICONQUESTION,
            )
        };

        // IDYES = 6
        if result.0 == 6 {
            // Kill the old instance and retry the mutex
            kill_old_instance();
            std::thread::sleep(std::time::Duration::from_millis(1500));

            // Try acquiring the mutex again
            let handle = unsafe {
                CreateMutexW(None, true, PCWSTR(name.as_ptr()))
                    .context("CreateMutexW failed on retry")?
            };
            if unsafe { windows::Win32::Foundation::GetLastError() }.0 == 183 {
                // Still couldn't acquire — something else is holding it
                let msg2 = "Could not replace the running instance.\n\
                            Please manually close WisprFree from the system tray\n\
                            (right-click the green icon → Quit), then try again.";
                let wide2: Vec<u16> = msg2.encode_utf16().chain(std::iter::once(0)).collect();
                let title2: Vec<u16> = "WisprFree".encode_utf16().chain(std::iter::once(0)).collect();
                unsafe {
                    windows::Win32::UI::WindowsAndMessaging::MessageBoxW(
                        windows::Win32::Foundation::HWND::default(),
                        windows::core::PCWSTR(wide2.as_ptr()),
                        windows::core::PCWSTR(title2.as_ptr()),
                        windows::Win32::UI::WindowsAndMessaging::MB_OK
                            | windows::Win32::UI::WindowsAndMessaging::MB_ICONWARNING,
                    );
                }
                std::process::exit(1);
            }
            return Ok(handle);
        }

        std::process::exit(0);
    }

    Ok(handle)
}

/// Kill any other running wisprfree.exe processes (but not ourselves).
fn kill_old_instance() {
    use std::os::windows::process::CommandExt;
    use windows::Win32::System::Threading::{
        OpenProcess, TerminateProcess, PROCESS_TERMINATE,
    };
    use windows::Win32::System::Diagnostics::ToolHelp::{
        CreateToolhelp32Snapshot, Process32FirstW, Process32NextW,
        PROCESSENTRY32W, TH32CS_SNAPPROCESS,
    };
    use windows::Win32::Foundation::CloseHandle;

    let our_pid = std::process::id();

    let snapshot = unsafe { CreateToolhelp32Snapshot(TH32CS_SNAPPROCESS, 0) };
    let snapshot = match snapshot {
        Ok(h) => h,
        Err(e) => {
            log::error!("CreateToolhelp32Snapshot failed: {e}");
            // Fallback: try taskkill directly
            let _ = std::process::Command::new("taskkill")
                .args(["/F", "/IM", "wisprfree.exe"])
                .creation_flags(0x08000000)
                .output();
            std::thread::sleep(std::time::Duration::from_millis(500));
            return;
        }
    };

    let mut entry = PROCESSENTRY32W {
        dwSize: std::mem::size_of::<PROCESSENTRY32W>() as u32,
        ..Default::default()
    };

    let mut found = unsafe { Process32FirstW(snapshot, &mut entry) }.is_ok();
    while found {
        let exe_name: String = entry
            .szExeFile
            .iter()
            .take_while(|&&c| c != 0)
            .map(|&c| char::from(c as u8))
            .collect();

        if exe_name.eq_ignore_ascii_case("wisprfree.exe") && entry.th32ProcessID != our_pid {
            log::info!("killing old wisprfree pid={}", entry.th32ProcessID);
            if let Ok(proc) =
                unsafe { OpenProcess(PROCESS_TERMINATE, false, entry.th32ProcessID) }
            {
                let _ = unsafe { TerminateProcess(proc, 1) };
                let _ = unsafe { CloseHandle(proc) };
            }
        }
        found = unsafe { Process32NextW(snapshot, &mut entry) }.is_ok();
    }
    let _ = unsafe { CloseHandle(snapshot) };

    std::thread::sleep(std::time::Duration::from_millis(500));
    log::info!("old instance cleanup done (our pid={})", our_pid);
}

// ── Notifications ─────────────────────────────────────────────────────

/// Show a simple Windows message box notification from any thread.
fn show_notification(title: &str, message: &str) {
    let wide_msg: Vec<u16> = message.encode_utf16().chain(std::iter::once(0)).collect();
    let wide_title: Vec<u16> = title.encode_utf16().chain(std::iter::once(0)).collect();
    // Spawn a thread so we don't block the orchestrator
    std::thread::spawn(move || {
        unsafe {
            windows::Win32::UI::WindowsAndMessaging::MessageBoxW(
                windows::Win32::Foundation::HWND::default(),
                windows::core::PCWSTR(wide_msg.as_ptr()),
                windows::core::PCWSTR(wide_title.as_ptr()),
                windows::Win32::UI::WindowsAndMessaging::MB_OK
                    | windows::Win32::UI::WindowsAndMessaging::MB_ICONWARNING,
            );
        }
    });
}
