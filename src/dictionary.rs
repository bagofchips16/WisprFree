//! Personal dictionary – teach WisprFree names and terms it consistently
//! mishears.
//!
//! Stored in `%APPDATA%\WisprFree\dictionary.toml`:
//!
//! ```toml
//! [[entry]]
//! wrong = "whisperfree"
//! correct = "WisprFree"
//!
//! [[entry]]
//! wrong = "john doe"
//! correct = "Jon Doe"
//! ```

use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};
use std::fs;
use std::path::PathBuf;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DictEntry {
    pub wrong: String,
    pub correct: String,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct DictFile {
    #[serde(default)]
    pub entry: Vec<DictEntry>,
}

pub struct PersonalDictionary {
    entries: Vec<(String, String)>, // (lower-case wrong, correct)
    path: PathBuf,
}

impl PersonalDictionary {
    /// Load from TOML (creates a template if missing).
    pub fn load(path: PathBuf) -> Result<Self> {
        if !path.exists() {
            let template = DictFile {
                entry: vec![DictEntry {
                    wrong: "whisperfree".into(),
                    correct: "WisprFree".into(),
                }],
            };
            if let Some(parent) = path.parent() {
                fs::create_dir_all(parent)?;
            }
            fs::write(&path, toml::to_string_pretty(&template)?)?;
            log::info!("created template dictionary at {}", path.display());
        }

        let text = fs::read_to_string(&path)
            .with_context(|| format!("failed to read {}", path.display()))?;
        let file: DictFile = toml::from_str(&text)
            .with_context(|| format!("invalid TOML in {}", path.display()))?;

        let entries: Vec<(String, String)> = file
            .entry
            .iter()
            .map(|e| (e.wrong.to_lowercase(), e.correct.clone()))
            .collect();

        log::info!("loaded {} dictionary entries from {}", entries.len(), path.display());
        Ok(Self { entries, path })
    }

    /// Apply corrections to transcribed text (case-insensitive match,
    /// preserving surrounding text).
    pub fn correct(&self, text: &str) -> String {
        let mut result = text.to_string();

        // Sort longest-first to avoid partial replacements.
        let mut sorted = self.entries.clone();
        sorted.sort_by(|a, b| b.0.len().cmp(&a.0.len()));

        for (wrong, correct) in &sorted {
            // Replace all occurrences, case-insensitive.
            let lower = result.to_lowercase();
            let mut new = String::with_capacity(result.len());
            let mut search_start = 0;

            while let Some(pos) = lower[search_start..].find(wrong.as_str()) {
                let abs_pos = search_start + pos;
                new.push_str(&result[search_start..abs_pos]);
                new.push_str(correct);
                search_start = abs_pos + wrong.len();
            }
            new.push_str(&result[search_start..]);
            result = new;
        }

        result
    }

    /// Reload from disk.
    pub fn reload(&mut self) -> Result<()> {
        let new = Self::load(self.path.clone())?;
        self.entries = new.entries;
        Ok(())
    }
}
