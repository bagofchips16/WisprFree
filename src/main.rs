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
mod config;
mod dictionary;
mod hotkey;
mod injector;
mod punctuation;
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

fn main() {
    if let Err(e) = run() {
        eprintln!("fatal: {e:#}");
        // Show a message box so the user sees the error even without a console.
        let msg = format!("WisprFree failed to start:\n\n{e:#}");
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
}

fn run() -> Result<()> {
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

    std::thread::spawn(move || {
        loop {
            select! {
                recv(hk_rx) -> msg => {
                    match msg {
                        Ok(hotkey::HotkeyEvent::PushDown) => {
                            log::info!("⏺  recording…");
                            orch_capture.start_recording();
                        }
                        Ok(hotkey::HotkeyEvent::PushUp) => {
                            log::info!("⏹  processing…");
                            match orch_capture.stop_recording() {
                                Ok(samples) if samples.is_empty() => {
                                    log::warn!("no audio captured");
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

                                            if let Err(e) = injector::inject(
                                                &text,
                                                &injection_method,
                                                clipboard_delay,
                                            ) {
                                                log::error!("injection failed: {e:#}");
                                            }
                                        }
                                        Err(e) => log::error!("transcription failed: {e:#}"),
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
