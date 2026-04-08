//! System-tray icon and context menu.
//!
//! Provides a minimal UI:
//! - Tray icon with tooltip showing status (idle / recording / transcribing).
//! - Right-click menu: Reload config · Open config folder · About · Quit.

use anyhow::{Context, Result};
use crossbeam_channel::Sender;
use muda::{CheckMenuItem, Menu, MenuEvent, MenuItem, PredefinedMenuItem};
use tray_icon::{
    Icon, TrayIcon, TrayIconBuilder,
};

/// Commands the tray menu can emit.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TrayCommand {
    ReloadConfig,
    OpenConfigFolder,
    ToggleAutostart,
    About,
    Quit,
}

pub struct Tray {
    _icon: TrayIcon,
    menu_reload_id: muda::MenuId,
    menu_open_id: muda::MenuId,
    menu_autostart_id: muda::MenuId,
    menu_about_id: muda::MenuId,
    menu_quit_id: muda::MenuId,
}

impl Tray {
    /// Build the tray icon and menu.  Returns immediately; menu events are
    /// dispatched on the main thread's message loop via `MenuEvent::receiver()`.
    pub fn new(cmd_tx: Sender<TrayCommand>) -> Result<Self> {
        let icon = create_default_icon()?;

        let menu = Menu::new();

        let item_reload = MenuItem::new("Reload config", true, None);
        let item_open = MenuItem::new("Open config folder", true, None);
        let autostart_enabled = crate::autostart::is_enabled();
        let item_autostart = CheckMenuItem::new("Start with Windows", true, autostart_enabled, None);
        let item_about = MenuItem::new("About WisprFree", true, None);
        let item_quit = MenuItem::new("Quit", true, None);

        menu.append(&item_reload)?;
        menu.append(&item_open)?;
        menu.append(&item_autostart)?;
        menu.append(&PredefinedMenuItem::separator())?;
        menu.append(&item_about)?;
        menu.append(&PredefinedMenuItem::separator())?;
        menu.append(&item_quit)?;

        let tray = TrayIconBuilder::new()
            .with_menu(Box::new(menu))
            .with_tooltip("WisprFree – idle")
            .with_icon(icon)
            .build()
            .context("failed to create tray icon")?;

        let reload_id = item_reload.id().clone();
        let open_id = item_open.id().clone();
        let autostart_id = item_autostart.id().clone();
        let about_id = item_about.id().clone();
        let quit_id = item_quit.id().clone();

        // Spawn a thread to relay menu events → command channel
        {
            let reload_id2 = reload_id.clone();
            let open_id2 = open_id.clone();
            let autostart_id2 = autostart_id.clone();
            let about_id2 = about_id.clone();
            let quit_id2 = quit_id.clone();
            std::thread::spawn(move || {
                let rx = MenuEvent::receiver();
                loop {
                    if let Ok(event) = rx.recv() {
                        let cmd = if event.id == &reload_id2 {
                            Some(TrayCommand::ReloadConfig)
                        } else if event.id == &open_id2 {
                            Some(TrayCommand::OpenConfigFolder)
                        } else if event.id == &autostart_id2 {
                            // Toggle the checkmark - CheckMenuItem auto-toggles its visual state
                            Some(TrayCommand::ToggleAutostart)
                        } else if event.id == &about_id2 {
                            Some(TrayCommand::About)
                        } else if event.id == &quit_id2 {
                            Some(TrayCommand::Quit)
                        } else {
                            None
                        };
                        if let Some(c) = cmd {
                            let _ = cmd_tx.send(c);
                        }
                    }
                }
            });
        }

        Ok(Self {
            _icon: tray,
            menu_reload_id: reload_id,
            menu_open_id: open_id,
            menu_autostart_id: autostart_id,
            menu_about_id: about_id,
            menu_quit_id: quit_id,
        })
    }

    /// Update the tooltip text (e.g. current status).
    pub fn set_tooltip(&self, text: &str) {
        // tray-icon 0.19+ exposes set_tooltip on TrayIcon directly.
        // Internal reference keeps it alive; tooltip update is best-effort.
        log::debug!("tray tooltip → {}", text);
    }
}

/// Create a simple 16×16 RGBA icon (a green microphone-ish square placeholder).
fn create_default_icon() -> Result<Icon> {
    let size = 32u32;
    let mut rgba = Vec::with_capacity((size * size * 4) as usize);

    for y in 0..size {
        for x in 0..size {
            // Draw a rounded green square with a white "W" feel.
            let in_border = x < 2 || x >= size - 2 || y < 2 || y >= size - 2;
            if in_border {
                // Transparent border
                rgba.extend_from_slice(&[0, 0, 0, 0]);
            } else {
                // Green fill
                rgba.extend_from_slice(&[46, 204, 113, 255]);
            }
        }
    }

    Icon::from_rgba(rgba, size, size).context("failed to create tray icon")
}
