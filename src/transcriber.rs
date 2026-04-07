//! Whisper-based speech-to-text transcription (fully local).
//!
//! Uses the whisper.cpp CLI binary (`whisper-cli.exe`) as a subprocess,
//! avoiding C-binding compatibility issues entirely.

use anyhow::{bail, Context, Result};
use std::path::{Path, PathBuf};
use std::process::Command;

/// Wraps a path to the whisper CLI binary and model, invoking it per utterance.
pub struct Transcriber {
    cli_path: PathBuf,
    model_path: PathBuf,
    language: String,
    threads: u32,
}

impl Transcriber {
    /// Locate the whisper CLI binary and validate the model exists.
    ///
    /// Looks for `whisper-cli.exe` next to the running executable,
    /// or in a `bin/` subdirectory.
    pub fn new(model_path: &Path, language: &str, _use_gpu: bool, threads: u32) -> Result<Self> {
        if !model_path.exists() {
            bail!(
                "whisper model not found at {}\n\
                 Download a model from https://huggingface.co/ggerganov/whisper.cpp/tree/main \
                 and place it in the models/ directory next to the executable.",
                model_path.display()
            );
        }

        let cli_path = find_whisper_cli()
            .context("whisper-cli.exe not found.\n\
                      Download it from https://github.com/ggerganov/whisper.cpp/releases \
                      and place it next to wisprfree.exe (or in a bin/ subfolder).")?;

        log::info!(
            "whisper CLI: {} | model: {} | threads={}",
            cli_path.display(),
            model_path.display(),
            threads
        );

        let effective_threads = if threads == 0 {
            std::thread::available_parallelism()
                .map(|p| p.get() as u32)
                .unwrap_or(4)
        } else {
            threads
        };

        Ok(Self {
            cli_path,
            model_path: model_path.to_path_buf(),
            language: language.to_string(),
            threads: effective_threads,
        })
    }

    /// Transcribe a 16 kHz mono WAV file → text.
    pub fn transcribe_file(&self, wav_path: &Path) -> Result<String> {
        let start = std::time::Instant::now();

        let mut cmd = Command::new(&self.cli_path);
        cmd.arg("-m").arg(&self.model_path)
            .arg("-f").arg(wav_path)
            .arg("--no-timestamps")
            .arg("--no-prints")
            .arg("-t").arg(self.threads.max(1).to_string())
            .arg("--prompt").arg("Hello, how are you? This is a voice dictation of natural English speech.");

        if !self.language.is_empty() {
            cmd.arg("-l").arg(&self.language);
        }

        let output = cmd
            .output()
            .with_context(|| format!("failed to run {}", self.cli_path.display()))?;

        if !output.status.success() {
            let stderr = String::from_utf8_lossy(&output.stderr);
            bail!("whisper-cli failed (exit {}): {}", output.status, stderr.trim());
        }

        let text = String::from_utf8_lossy(&output.stdout).trim().to_string();
        let elapsed = start.elapsed();

        log::info!(
            "transcribed in {:.0}ms ({} chars): \"{}\"",
            elapsed.as_millis(),
            text.len(),
            truncate_for_log(&text, 80),
        );

        Ok(text)
    }
}

/// Search for `whisper-cli.exe` (or `main.exe` from older builds) adjacent to
/// our executable or in common subdirectories.
fn find_whisper_cli() -> Option<PathBuf> {
    let exe_dir = std::env::current_exe()
        .ok()?
        .parent()?
        .to_path_buf();

    let candidates = [
        exe_dir.join("whisper-cli.exe"),
        exe_dir.join("bin").join("whisper-cli.exe"),
        exe_dir.join("whisper.exe"),
        exe_dir.join("bin").join("whisper.exe"),
        exe_dir.join("main.exe"),
        exe_dir.join("bin").join("main.exe"),
    ];

    for c in &candidates {
        if c.exists() {
            return Some(c.clone());
        }
    }

    // Fall back to PATH
    if let Ok(output) = Command::new("where").arg("whisper-cli.exe").output() {
        if output.status.success() {
            let path = String::from_utf8_lossy(&output.stdout);
            let first = path.lines().next()?;
            let p = PathBuf::from(first.trim());
            if p.exists() {
                return Some(p);
            }
        }
    }

    None
}

fn truncate_for_log(s: &str, max: usize) -> String {
    if s.len() <= max {
        s.to_string()
    } else {
        format!("{}…", &s[..max])
    }
}
