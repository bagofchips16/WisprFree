//! Manage "Start with Windows" via the current-user Run registry key.

use windows::core::PCWSTR;
use windows::Win32::System::Registry::{
    RegCloseKey, RegDeleteValueW, RegOpenKeyExW, RegQueryValueExW, RegSetValueExW,
    HKEY, HKEY_CURRENT_USER, KEY_READ, KEY_WRITE, REG_SZ,
};

const RUN_KEY: &str = r"Software\Microsoft\Windows\CurrentVersion\Run";
const VALUE_NAME: &str = "WisprFree";

/// Check whether WisprFree is set to start with Windows.
pub fn is_enabled() -> bool {
    unsafe {
        let subkey = to_wide(RUN_KEY);
        let mut hkey = HKEY::default();
        let res = RegOpenKeyExW(
            HKEY_CURRENT_USER,
            PCWSTR(subkey.as_ptr()),
            0,
            KEY_READ,
            &mut hkey,
        );
        if res.is_err() {
            return false;
        }

        let name = to_wide(VALUE_NAME);
        let exists = RegQueryValueExW(
            hkey,
            PCWSTR(name.as_ptr()),
            None,
            None,
            None,
            None,
        )
        .is_ok();

        let _ = RegCloseKey(hkey);
        exists
    }
}

/// Enable auto-start by writing the current exe path to the Run key.
pub fn enable() -> bool {
    let exe_path = match std::env::current_exe() {
        Ok(p) => p.to_string_lossy().to_string(),
        Err(_) => return false,
    };

    unsafe {
        let subkey = to_wide(RUN_KEY);
        let mut hkey = HKEY::default();
        let res = RegOpenKeyExW(
            HKEY_CURRENT_USER,
            PCWSTR(subkey.as_ptr()),
            0,
            KEY_WRITE,
            &mut hkey,
        );
        if res.is_err() {
            return false;
        }

        let name = to_wide(VALUE_NAME);
        let value = to_wide(&exe_path);
        let value_bytes: &[u8] = std::slice::from_raw_parts(
            value.as_ptr() as *const u8,
            value.len() * 2,
        );

        let ok = RegSetValueExW(
            hkey,
            PCWSTR(name.as_ptr()),
            0,
            REG_SZ,
            Some(value_bytes),
        )
        .is_ok();

        let _ = RegCloseKey(hkey);
        ok
    }
}

/// Disable auto-start by removing the value from the Run key.
pub fn disable() -> bool {
    unsafe {
        let subkey = to_wide(RUN_KEY);
        let mut hkey = HKEY::default();
        let res = RegOpenKeyExW(
            HKEY_CURRENT_USER,
            PCWSTR(subkey.as_ptr()),
            0,
            KEY_WRITE,
            &mut hkey,
        );
        if res.is_err() {
            return false;
        }

        let name = to_wide(VALUE_NAME);
        let ok = RegDeleteValueW(hkey, PCWSTR(name.as_ptr())).is_ok();

        let _ = RegCloseKey(hkey);
        ok
    }
}

/// Toggle auto-start. Returns the new state.
pub fn toggle() -> bool {
    if is_enabled() {
        disable();
        false
    } else {
        enable();
        true
    }
}

fn to_wide(s: &str) -> Vec<u16> {
    s.encode_utf16().chain(std::iter::once(0)).collect()
}
