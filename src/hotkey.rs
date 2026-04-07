//! Global keyboard hook for push-to-talk activation.
//!
//! Uses a Windows low-level keyboard hook (`WH_KEYBOARD_LL`) to detect
//! Ctrl+Space press and release events.  The hook callback posts custom
//! messages to the main thread's message loop.

use anyhow::{Context, Result};
use crossbeam_channel::Sender;
use once_cell::sync::OnceCell;
use std::sync::atomic::{AtomicBool, Ordering};
use windows::Win32::Foundation::{LPARAM, LRESULT, WPARAM};
use windows::Win32::UI::Input::KeyboardAndMouse::{GetAsyncKeyState, VK_CONTROL, VK_LCONTROL, VK_RCONTROL};
use windows::Win32::UI::WindowsAndMessaging::{
    CallNextHookEx, SetWindowsHookExW, UnhookWindowsHookEx, HHOOK, KBDLLHOOKSTRUCT,
    WH_KEYBOARD_LL, WM_KEYDOWN, WM_KEYUP, WM_SYSKEYDOWN, WM_SYSKEYUP,
};

/// Events emitted by the hotkey system.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum HotkeyEvent {
    /// User pressed the push-to-talk keys → start recording.
    PushDown,
    /// User released the push-to-talk keys → stop recording & transcribe.
    PushUp,
}

/// Configuration for which key combos to intercept.
struct HotkeyState {
    vk_code: u32,
    require_ctrl: bool,
    tx: Sender<HotkeyEvent>,
    is_down: AtomicBool,
}

/// Wrapper to make HHOOK Send+Sync (it's just a handle, safe to share).
struct SendHook(Option<HHOOK>);
unsafe impl Send for SendHook {}
unsafe impl Sync for SendHook {}

static STATE: OnceCell<HotkeyState> = OnceCell::new();
static HOOK_HANDLE: parking_lot::Mutex<SendHook> = parking_lot::Mutex::new(SendHook(None));

/// Install the low-level keyboard hook.  Must be called from a thread
/// that will run a Windows message pump (`GetMessage` loop).
///
/// `vk_code` – the virtual key code (default `VK_SPACE = 0x20`).
/// `require_ctrl` – whether Ctrl must be held.
pub fn install(vk_code: u32, require_ctrl: bool, tx: Sender<HotkeyEvent>) -> Result<()> {
    STATE
        .set(HotkeyState {
            vk_code,
            require_ctrl,
            tx,
            is_down: AtomicBool::new(false),
        })
        .ok()
        .context("hotkey hook already installed")?;

    let hook = unsafe {
        SetWindowsHookExW(WH_KEYBOARD_LL, Some(hook_proc), None, 0)
            .context("SetWindowsHookExW failed")?
    };

    *HOOK_HANDLE.lock() = SendHook(Some(hook));
    log::info!("keyboard hook installed (vk=0x{:02X}, ctrl={})", vk_code, require_ctrl);
    Ok(())
}

/// Remove the hook (call before exit).
pub fn uninstall() {
    if let Some(hook) = HOOK_HANDLE.lock().0.take() {
        unsafe {
            let _ = UnhookWindowsHookEx(hook);
        }
        log::info!("keyboard hook removed");
    }
}

/// The low-level keyboard procedure called by Windows.
unsafe extern "system" fn hook_proc(code: i32, wparam: WPARAM, lparam: LPARAM) -> LRESULT {
    if code >= 0 {
        if let Some(state) = STATE.get() {
            let kb = &*(lparam.0 as *const KBDLLHOOKSTRUCT);
            let vk = kb.vkCode;
            let msg = wparam.0 as u32;

            let is_target_key = vk == state.vk_code;
            let ctrl_ok = if state.require_ctrl {
                is_ctrl_down()
            } else {
                true
            };

            if is_target_key && ctrl_ok {
                match msg {
                    WM_KEYDOWN | WM_SYSKEYDOWN => {
                        // Only fire once (ignore auto-repeat)
                        if !state.is_down.swap(true, Ordering::SeqCst) {
                            let _ = state.tx.send(HotkeyEvent::PushDown);
                        }
                    }
                    WM_KEYUP | WM_SYSKEYUP => {
                        if state.is_down.swap(false, Ordering::SeqCst) {
                            let _ = state.tx.send(HotkeyEvent::PushUp);
                        }
                    }
                    _ => {}
                }
            }

            // If the target key goes up without ctrl, also reset state
            if is_target_key && (msg == WM_KEYUP || msg == WM_SYSKEYUP) {
                state.is_down.store(false, Ordering::SeqCst);
            }
        }
    }
    unsafe { CallNextHookEx(None, code, wparam, lparam) }
}

/// Check if either Ctrl key is currently held.
fn is_ctrl_down() -> bool {
    unsafe {
        GetAsyncKeyState(VK_CONTROL.0 as i32) < 0
            || GetAsyncKeyState(VK_LCONTROL.0 as i32) < 0
            || GetAsyncKeyState(VK_RCONTROL.0 as i32) < 0
    }
}
