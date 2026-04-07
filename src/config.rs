//! Configuration management for WisprFree.
//!
//! Loads and persists user settings from a TOML file stored in:
//!   `%APPDATA%\WisprFree\config.toml`

use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};
use std::fs;
use std::path::PathBuf;

/// Top-level configuration.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct AppConfig {
    pub general: GeneralConfig,
    pub audio: AudioConfig,
    pub whisper: WhisperConfig,
    pub hotkey: HotkeyConfig,
    pub injection: InjectionConfig,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct GeneralConfig {
    /// Show a desktop notification after each transcription.
    pub show_notifications: bool,
    /// Log level: "error", "warn", "info", "debug", "trace".
    pub log_level: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct AudioConfig {
    /// Name substring of the preferred input device (empty = system default).
    pub device_name: String,
    /// Internal buffer duration in milliseconds before Whisper gets data.
    pub buffer_ms: u32,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct WhisperConfig {
    /// Path to the GGML model file. Relative paths resolve from the exe dir.
    pub model_path: String,
    /// Language code (e.g. "en"). Empty string = auto-detect.
    pub language: String,
    /// Use GPU acceleration when available.
    pub use_gpu: bool,
    /// Number of threads for Whisper inference (0 = auto).
    pub threads: u32,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct HotkeyConfig {
    /// Virtual-key code for the activation key (default 0x20 = Space).
    pub vk_code: u32,
    /// Require Ctrl modifier.
    pub ctrl: bool,
    /// Require Alt modifier.
    pub alt: bool,
    /// Require Shift modifier.
    pub shift: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct InjectionConfig {
    /// "clipboard" (Ctrl-V) or "sendinput" (simulated keystrokes).
    pub method: String,
    /// Delay in ms between restoring the old clipboard and finishing.
    pub clipboard_restore_delay_ms: u64,
}

// ── Defaults ──────────────────────────────────────────────────────────

impl Default for AppConfig {
    fn default() -> Self {
        Self {
            general: GeneralConfig::default(),
            audio: AudioConfig::default(),
            whisper: WhisperConfig::default(),
            hotkey: HotkeyConfig::default(),
            injection: InjectionConfig::default(),
        }
    }
}

impl Default for GeneralConfig {
    fn default() -> Self {
        Self {
            show_notifications: true,
            log_level: "info".into(),
        }
    }
}

impl Default for AudioConfig {
    fn default() -> Self {
        Self {
            device_name: String::new(),
            buffer_ms: 100,
        }
    }
}

impl Default for WhisperConfig {
    fn default() -> Self {
        Self {
            model_path: "models/ggml-base.en.bin".into(),
            language: "en".into(),
            use_gpu: false,
            threads: 0,
        }
    }
}

impl Default for HotkeyConfig {
    fn default() -> Self {
        Self {
            vk_code: 0x20, // VK_SPACE
            ctrl: true,
            alt: false,
            shift: false,
        }
    }
}

impl Default for InjectionConfig {
    fn default() -> Self {
        Self {
            method: "clipboard".into(),
            clipboard_restore_delay_ms: 50,
        }
    }
}

// ── I/O helpers ───────────────────────────────────────────────────────

/// Returns `%APPDATA%\WisprFree`.
pub fn config_dir() -> Result<PathBuf> {
    let base = dirs::config_dir().context("cannot resolve %APPDATA%")?;
    Ok(base.join("WisprFree"))
}

/// Resolves the config TOML path.
pub fn config_path() -> Result<PathBuf> {
    Ok(config_dir()?.join("config.toml"))
}

/// Loads config from disk, falling back to defaults if the file is missing.
pub fn load() -> Result<AppConfig> {
    let path = config_path()?;
    if path.exists() {
        let text = fs::read_to_string(&path)
            .with_context(|| format!("failed to read {}", path.display()))?;
        let cfg: AppConfig =
            toml::from_str(&text).with_context(|| format!("invalid TOML in {}", path.display()))?;
        Ok(cfg)
    } else {
        let cfg = AppConfig::default();
        save(&cfg)?; // write defaults so the user has a template
        Ok(cfg)
    }
}

/// Persists config to disk.
pub fn save(cfg: &AppConfig) -> Result<()> {
    let path = config_path()?;
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)?;
    }
    let text = toml::to_string_pretty(cfg)?;
    fs::write(&path, text)?;
    Ok(())
}

/// Resolve a model path: if relative, look next to the running executable.
pub fn resolve_model_path(raw: &str) -> PathBuf {
    let p = PathBuf::from(raw);
    if p.is_absolute() {
        p
    } else {
        let exe_dir = std::env::current_exe()
            .ok()
            .and_then(|e| e.parent().map(|p| p.to_path_buf()))
            .unwrap_or_else(|| PathBuf::from("."));
        exe_dir.join(p)
    }
}
