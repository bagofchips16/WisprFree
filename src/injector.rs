//! Injects transcribed text at the current cursor position.
//!
//! Two strategies:
//! 1. **Clipboard** (default) – copies text to clipboard, sends Ctrl+V,
//!    then restores the previous clipboard contents.
//! 2. **SendInput** – simulates individual `WM_CHAR` key events for each
//!    character.  More compatible with some games/terminals but slower.

use anyhow::{Context, Result};
use std::thread;
use std::time::Duration;
use windows::Win32::Foundation::{HANDLE, HGLOBAL, HWND};
use windows::Win32::System::DataExchange::{
    CloseClipboard, EmptyClipboard, GetClipboardData, OpenClipboard, SetClipboardData,
};
use windows::Win32::System::Memory::{GlobalAlloc, GlobalLock, GlobalUnlock, GMEM_MOVEABLE};
use windows::Win32::UI::Input::KeyboardAndMouse::{
    SendInput, INPUT, INPUT_0, INPUT_KEYBOARD, KEYBDINPUT, KEYBD_EVENT_FLAGS,
    KEYEVENTF_KEYUP, KEYEVENTF_UNICODE, VK_CONTROL, VK_V,
};

/// CF_UNICODETEXT clipboard format constant.
const CF_UNICODETEXT: u32 = 13;

/// Inject `text` at the cursor using the chosen method.
pub fn inject(text: &str, method: &str, restore_delay_ms: u64) -> Result<()> {
    if text.is_empty() {
        return Ok(());
    }
    match method {
        "sendinput" => inject_via_sendinput(text),
        _ => inject_via_clipboard(text, restore_delay_ms),
    }
}

// ── Clipboard strategy ────────────────────────────────────────────────

fn inject_via_clipboard(text: &str, restore_delay_ms: u64) -> Result<()> {
    // 1. Save current clipboard
    let old_clipboard = get_clipboard_text();

    // 2. Release any held modifier keys first (Ctrl from hotkey)
    release_modifiers();
    thread::sleep(Duration::from_millis(50));

    // 3. Set our text
    set_clipboard_text(text)?;

    // 4. Small delay to let clipboard settle
    thread::sleep(Duration::from_millis(30));

    // 5. Simulate Ctrl+V
    send_ctrl_v()?;

    // 6. Wait for the target app to process the paste
    thread::sleep(Duration::from_millis(restore_delay_ms.max(100)));

    // 7. Restore old clipboard
    if let Some(old) = old_clipboard {
        let _ = set_clipboard_text(&old);
    }

    log::debug!("injected {} chars via clipboard", text.len());
    Ok(())
}

fn get_clipboard_text() -> Option<String> {
    unsafe {
        if OpenClipboard(HWND::default()).is_err() {
            return None;
        }
        let result = (|| {
            let handle: HANDLE = GetClipboardData(CF_UNICODETEXT).ok()?;
            // HANDLE from GetClipboardData is actually an HGLOBAL
            let hglobal: HGLOBAL = std::mem::transmute(handle);
            let ptr = GlobalLock(hglobal) as *const u16;
            if ptr.is_null() {
                return None;
            }
            let mut len = 0;
            while *ptr.add(len) != 0 {
                len += 1;
            }
            let slice = std::slice::from_raw_parts(ptr, len);
            let text = String::from_utf16_lossy(slice);
            let _ = GlobalUnlock(hglobal);
            Some(text)
        })();
        let _ = CloseClipboard();
        result
    }
}

fn set_clipboard_text(text: &str) -> Result<()> {
    let wide: Vec<u16> = text.encode_utf16().chain(std::iter::once(0)).collect();
    let byte_len = wide.len() * 2;

    unsafe {
        OpenClipboard(HWND::default()).context("OpenClipboard failed")?;
        let _ = EmptyClipboard();

        let hmem: HGLOBAL = GlobalAlloc(GMEM_MOVEABLE, byte_len).context("GlobalAlloc failed")?;
        let ptr = GlobalLock(hmem) as *mut u16;
        if ptr.is_null() {
            let _ = CloseClipboard();
            anyhow::bail!("GlobalLock returned null");
        }
        std::ptr::copy_nonoverlapping(wide.as_ptr(), ptr, wide.len());
        let _ = GlobalUnlock(hmem);

        // SetClipboardData takes HANDLE; HGLOBAL has the same repr
        let handle: HANDLE = std::mem::transmute(hmem);
        SetClipboardData(CF_UNICODETEXT, handle)
            .context("SetClipboardData failed")?;
        let _ = CloseClipboard();
    }
    Ok(())
}

fn send_ctrl_v() -> Result<()> {
    let inputs = [
        make_key_input(VK_CONTROL.0, false),
        make_key_input(VK_V.0, false),
        make_key_input(VK_V.0, true),
        make_key_input(VK_CONTROL.0, true),
    ];
    unsafe {
        let sent = SendInput(&inputs, std::mem::size_of::<INPUT>() as i32);
        if sent != inputs.len() as u32 {
            anyhow::bail!("SendInput returned {sent}, expected {}", inputs.len());
        }
    }
    Ok(())
}

// ── SendInput strategy ────────────────────────────────────────────────

fn inject_via_sendinput(text: &str) -> Result<()> {
    let mut inputs: Vec<INPUT> = Vec::with_capacity(text.len() * 2);

    for ch in text.encode_utf16() {
        // Key down
        inputs.push(INPUT {
            r#type: INPUT_KEYBOARD,
            Anonymous: INPUT_0 {
                ki: KEYBDINPUT {
                    wVk: windows::Win32::UI::Input::KeyboardAndMouse::VIRTUAL_KEY(0),
                    wScan: ch,
                    dwFlags: KEYEVENTF_UNICODE,
                    time: 0,
                    dwExtraInfo: 0,
                },
            },
        });
        // Key up
        inputs.push(INPUT {
            r#type: INPUT_KEYBOARD,
            Anonymous: INPUT_0 {
                ki: KEYBDINPUT {
                    wVk: windows::Win32::UI::Input::KeyboardAndMouse::VIRTUAL_KEY(0),
                    wScan: ch,
                    dwFlags: KEYEVENTF_UNICODE | KEYEVENTF_KEYUP,
                    time: 0,
                    dwExtraInfo: 0,
                },
            },
        });
    }

    unsafe {
        let sent = SendInput(&inputs, std::mem::size_of::<INPUT>() as i32);
        if (sent as usize) != inputs.len() {
            anyhow::bail!("SendInput returned {sent}, expected {}", inputs.len());
        }
    }
    log::debug!("injected {} chars via SendInput", text.len());
    Ok(())
}

// ── shared helpers ────────────────────────────────────────────────────

/// Release Ctrl, Alt, Shift modifier keys so they don't interfere with paste.
fn release_modifiers() {
    let modifiers = [
        VK_CONTROL.0,
        windows::Win32::UI::Input::KeyboardAndMouse::VK_LCONTROL.0,
        windows::Win32::UI::Input::KeyboardAndMouse::VK_RCONTROL.0,
        windows::Win32::UI::Input::KeyboardAndMouse::VK_SHIFT.0,
        windows::Win32::UI::Input::KeyboardAndMouse::VK_MENU.0,
    ];
    let inputs: Vec<INPUT> = modifiers
        .iter()
        .map(|&vk| make_key_input(vk, true))
        .collect();
    unsafe {
        let _ = SendInput(&inputs, std::mem::size_of::<INPUT>() as i32);
    }
}

fn make_key_input(vk: u16, key_up: bool) -> INPUT {
    let flags = if key_up {
        KEYEVENTF_KEYUP
    } else {
        KEYBD_EVENT_FLAGS(0)
    };
    INPUT {
        r#type: INPUT_KEYBOARD,
        Anonymous: INPUT_0 {
            ki: KEYBDINPUT {
                wVk: windows::Win32::UI::Input::KeyboardAndMouse::VIRTUAL_KEY(vk),
                wScan: 0,
                dwFlags: flags,
                time: 0,
                dwExtraInfo: 0,
            },
        },
    }
}
