# WisprFree – Product Requirements Document

## Vision
A single-exe, fully local, push-to-talk voice-to-text tool for Windows that
works in **any** application, requires zero setup, zero internet, and zero
accounts.

## Core Requirements

### R1 – Push-to-Talk Dictation
- **Activation**: Hold Ctrl+Space (configurable) to begin recording.
- **Deactivation**: Release to stop recording, transcribe, and inject text.
- **Scope**: Works in any window that accepts keyboard input.

### R2 – Sub-second Latency
- Transcription must complete in < 1 second for utterances up to 15 seconds
  on a modern quad-core CPU (Intel 10th gen / Ryzen 3000+).
- Use whisper.cpp with the `base.en` model by default.

### R3 – Fully Local / Private
- All processing happens on-device.
- No network connections are made at any point.
- No telemetry, analytics, crash reports, or update checks.

### R4 – Snippet Library
- Users define trigger → replacement pairs in `snippets.toml`.
- If the entire transcription matches a trigger, the replacement is pasted.
- Inline triggers within longer dictation are also expanded.

### R5 – Personal Dictionary
- Users define misheard → correct pairs in `dictionary.toml`.
- Applied as a post-processing step after transcription.

### R6 – Auto-Punctuation
- Capitalize first letter.
- Capitalize after sentence-ending punctuation (. ! ?).
- Add a trailing period if missing.
- Normalize whitespace around punctuation.

### R7 – Single Executable
- Ship as one `.exe` (plus model file).
- No Python, Node.js, or other runtime.
- No installer required – download and run.

## Non-Functional Requirements

| NFR | Target |
|-----|--------|
| Binary size (without model) | < 10 MB |
| Memory usage (idle) | < 50 MB |
| Memory usage (recording) | < 300 MB |
| Startup time | < 3 seconds |
| Supported OS | Windows 10 1903+ / Windows 11 |

## Technology Stack

| Component | Choice | Rationale |
|-----------|--------|-----------|
| Language | Rust | Single binary, no runtime, safe concurrency |
| Speech engine | whisper.cpp via whisper-rs | Best local quality, GGML quantisation |
| Audio capture | cpal | Cross-platform, mature |
| Resampling | rubato | High-quality FFT resampler |
| Windows API | windows crate | Official Microsoft bindings |
| Tray icon | tray-icon + muda | Lightweight, maintained |
| Config format | TOML | Human-friendly, Rust-native |
