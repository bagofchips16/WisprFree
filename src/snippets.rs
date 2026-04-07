//! Snippet library – say a trigger word, get a full phrase.
//!
//! Snippets are stored in `%APPDATA%\WisprFree\snippets.toml`:
//!
//! ```toml
//! [[snippet]]
//! trigger = "my email"
//! replacement = "alice@example.com"
//!
//! [[snippet]]
//! trigger = "my address"
//! replacement = "123 Main Street, Springfield, IL 62704"
//! ```

use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::fs;
use std::path::PathBuf;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SnippetEntry {
    pub trigger: String,
    pub replacement: String,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct SnippetFile {
    #[serde(default)]
    pub snippet: Vec<SnippetEntry>,
}

/// In-memory snippet lookup (case-insensitive).
pub struct SnippetLibrary {
    /// lower-case trigger → replacement
    map: HashMap<String, String>,
    path: PathBuf,
}

impl SnippetLibrary {
    /// Load from the given TOML file (creates a template if missing).
    pub fn load(path: PathBuf) -> Result<Self> {
        if !path.exists() {
            let template = SnippetFile {
                snippet: vec![
                    SnippetEntry {
                        trigger: "my email".into(),
                        replacement: "you@example.com".into(),
                    },
                    SnippetEntry {
                        trigger: "my address".into(),
                        replacement: "123 Main Street, Anytown, USA".into(),
                    },
                ],
            };
            if let Some(parent) = path.parent() {
                fs::create_dir_all(parent)?;
            }
            fs::write(&path, toml::to_string_pretty(&template)?)?;
            log::info!("created template snippets file at {}", path.display());
        }

        let text = fs::read_to_string(&path)
            .with_context(|| format!("failed to read {}", path.display()))?;
        let file: SnippetFile = toml::from_str(&text)
            .with_context(|| format!("invalid TOML in {}", path.display()))?;

        let mut map = HashMap::new();
        for s in &file.snippet {
            map.insert(s.trigger.to_lowercase(), s.replacement.clone());
        }

        log::info!("loaded {} snippets from {}", map.len(), path.display());
        Ok(Self { map, path })
    }

    /// Expand any snippet triggers found in `text`.
    ///
    /// If the *entire* transcription (trimmed, case-insensitive) matches a
    /// trigger, the whole text is replaced.  Otherwise, individual trigger
    /// phrases are substituted inline.
    pub fn expand(&self, text: &str) -> String {
        let trimmed = text.trim();
        let lower = trimmed.to_lowercase();

        // Exact whole-text match first.
        if let Some(replacement) = self.map.get(&lower) {
            log::debug!("snippet exact match: \"{}\" → \"{}\"", trimmed, replacement);
            return replacement.clone();
        }

        // Inline substring replacement (longest match first to avoid partials).
        let mut result = text.to_string();
        let mut triggers: Vec<(&String, &String)> = self.map.iter().collect();
        triggers.sort_by(|a, b| b.0.len().cmp(&a.0.len()));

        for (trigger, replacement) in triggers {
            // Case-insensitive inline replace
            let lower_result = result.to_lowercase();
            if let Some(pos) = lower_result.find(trigger.as_str()) {
                result = format!(
                    "{}{}{}",
                    &result[..pos],
                    replacement,
                    &result[pos + trigger.len()..]
                );
            }
        }
        result
    }

    /// Reload from disk.
    pub fn reload(&mut self) -> Result<()> {
        let new = Self::load(self.path.clone())?;
        self.map = new.map;
        Ok(())
    }
}
