//! Floating overlay indicator for recording/processing state.
//!
//! Shows a small pill-shaped window at the top-center of the screen:
//! - **Red** while recording
//! - **Green** briefly when transcription completes
//! - Hidden when idle

use anyhow::{Context, Result};
use std::sync::mpsc;
use windows::core::PCWSTR;
use windows::Win32::Foundation::{HWND, LPARAM, LRESULT, WPARAM};
use windows::Win32::Graphics::Gdi::{
    BeginPaint, CreateSolidBrush, EndPaint, FillRect, InvalidateRect, PAINTSTRUCT,
};
use windows::Win32::UI::WindowsAndMessaging::*;

/// Commands sent to the overlay thread.
#[derive(Debug, Clone, Copy)]
pub enum OverlayState {
    Recording,
    Processing,
    Done,
    Hidden,
}

/// Handle to control the overlay from other threads.
pub struct Overlay {
    tx: mpsc::Sender<OverlayState>,
}

impl Overlay {
    /// Spawn the overlay window on a dedicated thread.
    pub fn new() -> Result<Self> {
        let (tx, rx) = mpsc::channel::<OverlayState>();

        std::thread::Builder::new()
            .name("overlay".into())
            .spawn(move || {
                if let Err(e) = overlay_thread(rx) {
                    log::error!("overlay thread failed: {e:#}");
                }
            })
            .context("failed to spawn overlay thread")?;

        Ok(Self { tx })
    }

    /// Update the overlay state.
    pub fn set_state(&self, state: OverlayState) {
        let _ = self.tx.send(state);
    }
}

// ── Win32 overlay window ──────────────────────────────────────────────

const PILL_WIDTH: i32 = 160;
const PILL_HEIGHT: i32 = 32;
const WM_OVERLAY_UPDATE: u32 = WM_USER + 1;

/// Colors (BGR format for Win32).
const COLOR_RED: u32 = 0x003030E0;     // recording
const COLOR_YELLOW: u32 = 0x0020C8E8;  // processing
const COLOR_GREEN: u32 = 0x0040B840;   // done

/// Global state for the WndProc callback.
static CURRENT_STATE: std::sync::atomic::AtomicU8 = std::sync::atomic::AtomicU8::new(0);

fn state_to_u8(s: OverlayState) -> u8 {
    match s {
        OverlayState::Recording => 1,
        OverlayState::Processing => 2,
        OverlayState::Done => 3,
        OverlayState::Hidden => 0,
    }
}

fn state_color(val: u8) -> u32 {
    match val {
        1 => COLOR_RED,
        2 => COLOR_YELLOW,
        3 => COLOR_GREEN,
        _ => 0,
    }
}

fn state_text(val: u8) -> &'static str {
    match val {
        1 => "  \u{25CF}  Recording...",
        2 => "  \u{23F3}  Processing...",
        3 => "  \u{2713}  Done",
        _ => "",
    }
}

fn overlay_thread(rx: mpsc::Receiver<OverlayState>) -> Result<()> {
    unsafe {
        let class_name: Vec<u16> = "WisprOverlay\0".encode_utf16().collect();

        let wc = WNDCLASSEXW {
            cbSize: std::mem::size_of::<WNDCLASSEXW>() as u32,
            style: CS_HREDRAW | CS_VREDRAW,
            lpfnWndProc: Some(overlay_wnd_proc),
            hInstance: windows::Win32::System::LibraryLoader::GetModuleHandleW(None)
                .unwrap_or_default()
                .into(),
            lpszClassName: PCWSTR(class_name.as_ptr()),
            hCursor: LoadCursorW(None, IDC_ARROW).ok().unwrap_or_default(),
            ..Default::default()
        };
        RegisterClassExW(&wc);

        // Position at top-center of primary monitor
        let screen_w = GetSystemMetrics(SM_CXSCREEN);
        let x = (screen_w - PILL_WIDTH) / 2;
        let y = 8;

        let hwnd = CreateWindowExW(
            WS_EX_LAYERED | WS_EX_TOPMOST | WS_EX_TOOLWINDOW | WS_EX_NOACTIVATE,
            PCWSTR(class_name.as_ptr()),
            PCWSTR::null(),
            WS_POPUP,
            x,
            y,
            PILL_WIDTH,
            PILL_HEIGHT,
            None,
            None,
            wc.hInstance,
            None,
        )?;

        // Set window transparency (220/255 = ~86% opaque)
        SetLayeredWindowAttributes(hwnd, None, 220, LWA_ALPHA)?;

        // Start hidden
        let _ = ShowWindow(hwnd, SW_HIDE);

        // Process messages + channel commands
        let mut msg = MSG::default();
        loop {
            // Check for overlay state changes (non-blocking)
            while let Ok(state) = rx.try_recv() {
                let val = state_to_u8(state);
                CURRENT_STATE.store(val, std::sync::atomic::Ordering::Relaxed);

                if val == 0 {
                    let _ = ShowWindow(hwnd, SW_HIDE);
                } else {
                    let _ = ShowWindow(hwnd, SW_SHOWNOACTIVATE);
                    let _ = InvalidateRect(hwnd, None, true);

                    // Auto-hide "Done" after 1 second
                    if val == 3 {
                        SetTimer(hwnd, 1, 1000, None);
                    }
                }
            }

            // Process Win32 messages (non-blocking peek)
            while PeekMessageW(&mut msg, None, 0, 0, PM_REMOVE).as_bool() {
                if msg.message == WM_QUIT {
                    return Ok(());
                }
                let _ = TranslateMessage(&msg);
                DispatchMessageW(&msg);
            }

            // Small sleep to avoid busy-waiting
            std::thread::sleep(std::time::Duration::from_millis(16));
        }
    }
}

unsafe extern "system" fn overlay_wnd_proc(
    hwnd: HWND,
    msg: u32,
    wparam: WPARAM,
    lparam: LPARAM,
) -> LRESULT {
    match msg {
        WM_PAINT => {
            let mut ps = PAINTSTRUCT::default();
            let hdc = BeginPaint(hwnd, &mut ps);

            let state_val = CURRENT_STATE.load(std::sync::atomic::Ordering::Relaxed);
            let color = state_color(state_val);
            let text = state_text(state_val);

            // Fill background
            let brush = CreateSolidBrush(windows::Win32::Foundation::COLORREF(color));
            let mut rect = ps.rcPaint;
            FillRect(hdc, &rect, brush);
            let _ = windows::Win32::Graphics::Gdi::DeleteObject(brush);

            // Draw text
            if !text.is_empty() {
                let _ = windows::Win32::Graphics::Gdi::SetBkMode(
                    hdc,
                    windows::Win32::Graphics::Gdi::TRANSPARENT,
                );
                let _ = windows::Win32::Graphics::Gdi::SetTextColor(
                    hdc,
                    windows::Win32::Foundation::COLORREF(0x00FFFFFF), // white
                );

                let wide: Vec<u16> = text.encode_utf16().collect();
                rect.left += 4;
                let _ = windows::Win32::Graphics::Gdi::DrawTextW(
                    hdc,
                    &mut wide.clone(),
                    &mut rect,
                    windows::Win32::Graphics::Gdi::DT_SINGLELINE
                        | windows::Win32::Graphics::Gdi::DT_VCENTER,
                );
            }

            let _ = EndPaint(hwnd, &ps);
            LRESULT(0)
        }
        WM_TIMER => {
            // Auto-hide after "Done" timeout
            KillTimer(hwnd, 1).ok();
            CURRENT_STATE.store(0, std::sync::atomic::Ordering::Relaxed);
            let _ = ShowWindow(hwnd, SW_HIDE);
            LRESULT(0)
        }
        // Make the window click-through
        WM_NCHITTEST => LRESULT(-1), // HTTRANSPARENT
        _ => DefWindowProcW(hwnd, msg, wparam, lparam),
    }
}
