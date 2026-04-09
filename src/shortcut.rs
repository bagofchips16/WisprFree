//! Creates a Start Menu shortcut so users can find WisprFree via Windows Search.
//!
//! On first run, creates:
//!   `%APPDATA%\Microsoft\Windows\Start Menu\Programs\WisprFree.lnk`

use std::os::windows::process::CommandExt;

/// Ensure a Start Menu shortcut exists pointing to the current exe.
pub fn ensure_shortcut() {
    if let Err(e) = create_shortcut() {
        log::warn!("could not create Start Menu shortcut: {e}");
    }
}

fn create_shortcut() -> anyhow::Result<()> {
    let start_menu = std::env::var("APPDATA")
        .map(|a| std::path::PathBuf::from(a).join(r"Microsoft\Windows\Start Menu\Programs"))?;

    let lnk_path = start_menu.join("WisprFree.lnk");

    // Skip if shortcut already exists and points to the right exe
    let exe_path = std::env::current_exe()?;
    if lnk_path.exists() {
        // Shortcut exists — good enough (we won't re-validate the target every time)
        return Ok(());
    }

    let exe_str = exe_path.to_string_lossy();
    let work_dir = exe_path
        .parent()
        .map(|p| p.to_string_lossy().to_string())
        .unwrap_or_default();
    let lnk_str = lnk_path.to_string_lossy();

    // Use PowerShell to create a .lnk shortcut (no COM dependency needed)
    let ps_script = format!(
        "$ws = New-Object -ComObject WScript.Shell; \
         $s = $ws.CreateShortcut('{lnk}'); \
         $s.TargetPath = '{exe}'; \
         $s.WorkingDirectory = '{dir}'; \
         $s.Description = 'WisprFree - Local voice-to-text'; \
         $s.Save()",
        lnk = lnk_str.replace('\'', "''"),
        exe = exe_str.replace('\'', "''"),
        dir = work_dir.replace('\'', "''"),
    );

    std::process::Command::new("powershell")
        .args(["-NoProfile", "-NonInteractive", "-Command", &ps_script])
        .creation_flags(0x08000000) // CREATE_NO_WINDOW
        .output()?;

    log::info!("created Start Menu shortcut: {}", lnk_path.display());
    Ok(())
}
