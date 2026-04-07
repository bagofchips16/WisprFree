# WisprFree

**Local, private voice-to-text for Windows.**  
Hold **Ctrl+Space** and speak → text appears wherever your cursor is.  
Works in any app. No cloud, no API, no internet required.

---

## Features

| Feature | Description |
|---------|-------------|
| **Push-to-talk** | Hold Ctrl+Space, speak, release → text is typed at the cursor |
| **Sub-second latency** | Whisper.cpp inference, optimised for modern hardware |
| **100% local** | Nothing leaves your PC. No account, no subscription, ever |
| **Snippet library** | Say a trigger word, get a full phrase pasted (email, address, canned replies) |
| **Personal dictionary** | Teach it names and terms Whisper consistently mishears |
| **Auto-punctuation** | Capitalisation, periods, and basic cleanup applied automatically |
| **Single .exe** | No Python, no install wizard, no runtime dependencies |

---

## Quick Start

### 1. Download the latest release

Download `WisprFree-v0.1.3-windows-x64.zip` from  
[**GitHub Releases**](https://github.com/bagofchips16/WisprFree/releases/latest)

Extract the zip. You'll get:

```
WisprFree/
  wisprfree.exe
  whisper-cli.exe
  ggml.dll
  ggml-base.dll
  ggml-cpu.dll
  whisper.dll
  models/
    ggml-base.en.bin
```

### 2. Run

Double-click `wisprfree.exe`. A green icon appears in the system tray.

> **Windows SmartScreen warning?**  
> Because WisprFree is a new, open-source app without a paid code-signing certificate,
> Windows may show a "Windows protected your PC" dialog.  
> Click **"More info"** → **"Run anyway"**.  
> This only happens the first time. The app is [fully open-source](https://github.com/bagofchips16/WisprFree) — you can inspect every line of code.

> **Microsoft Defender flags the file?**  
> Some antivirus software may flag WisprFree because it uses a low-level keyboard hook
> (to detect Ctrl+Space) and clipboard access (to paste text). These are standard techniques
> used by all voice-to-text and text-expansion tools.  
> You can add an exclusion: **Windows Security → Virus & threat protection → Manage settings → Exclusions → Add exclusion** → select the WisprFree folder.

### 3. Dictate

1. Click into any text field (Notepad, browser, Slack, VS Code, etc.)
2. **Hold Ctrl+Space**
3. Speak naturally
4. **Release Ctrl+Space**
5. Your transcribed text is inserted at the cursor

---

## Configuration

On first run, config files are created in `%APPDATA%\WisprFree\`:

| File | Purpose |
|------|---------|
| `config.toml` | General settings, model path, hotkey, injection method |
| `snippets.toml` | Trigger word → replacement phrase mappings |
| `dictionary.toml` | Misheard word → correct word mappings |

### config.toml (defaults)

```toml
[general]
show_notifications = true
log_level = "info"

[audio]
device_name = ""          # empty = system default mic
buffer_ms = 100

[whisper]
model_path = "models/ggml-base.en.bin"
language = "en"
use_gpu = false
threads = 0               # 0 = auto-detect

[hotkey]
vk_code = 32              # 0x20 = Spacebar
ctrl = true
alt = false
shift = false

[injection]
method = "clipboard"      # clipboard-based paste
clipboard_restore_delay_ms = 150
```

### snippets.toml

```toml
[[snippet]]
trigger = "my email"
replacement = "alice@example.com"

[[snippet]]
trigger = "my address"
replacement = "123 Main Street, Springfield, IL 62704"

[[snippet]]
trigger = "kind regards"
replacement = "Kind regards,\nAlice Smith\nSenior Engineer"
```

### dictionary.toml

```toml
[[entry]]
wrong = "whisperfree"
correct = "WisprFree"

[[entry]]
wrong = "john doe"
correct = "Jon Doe"
```

Right-click the tray icon → **Reload config** to pick up changes without restarting.

---

## Building from Source

### Prerequisites

- [Rust toolchain](https://rustup.rs/) (stable, MSVC target)
- Visual Studio C++ Build Tools
- [whisper.cpp CLI binary](https://github.com/ggerganov/whisper.cpp/releases) (`whisper-cli.exe` + DLLs, placed next to the built exe)

### Build

```powershell
git clone https://github.com/bagofchips16/WisprFree.git
cd wisprfree
cargo build --release
```

The binary is at `target/release/wisprfree.exe` (~2.5 MB). You'll also need to place `whisper-cli.exe`, its DLLs (`ggml.dll`, `ggml-base.dll`, `ggml-cpu.dll`, `whisper.dll`), and a model file (`models/ggml-base.en.bin`) next to the exe.

### Run in development

```powershell
# Set log level to debug
$env:RUST_LOG = "debug"
cargo run
```

---

## Architecture

```
┌─────────────────────────────────────────────────────┐
│  main thread  (Win32 message loop)                  │
│   ├─ WH_KEYBOARD_LL hook  (Ctrl+Space detection)   │
│   ├─ system tray icon + context menu                │
│   └─ orchestration: start / stop / inject           │
├─────────────────────────────────────────────────────┤
│  cpal audio callback  (OS-managed thread)           │
│   └─ captures mic → mono f32 → shared ring buffer   │
├─────────────────────────────────────────────────────┤
│  transcription  (per-utterance, on orchestrator)    │
│   └─ whisper.cpp → punctuation → dictionary → snip  │
│   └─ inject text via clipboard paste              │
└─────────────────────────────────────────────────────┘
```

### Processing pipeline

1. **Ctrl+Space down** → `audio::start_recording()`
2. **Ctrl+Space up** → `audio::stop_recording()` → resample to 16 kHz
3. `transcriber::transcribe()` → raw text
4. `punctuation::auto_punctuate()` → capitalised & punctuated
5. `dictionary::correct()` → personal corrections
6. `snippets::expand()` → trigger word expansion
7. `paster::inject()` → paste at cursor

---

## Troubleshooting

| Symptom | Fix |
|---------|-----|
| "whisper model not found" | Download a `.bin` model and place it in `models/` |
| No audio captured | Check Windows sound settings → default microphone |
| Text not appearing | Ensure your cursor is in a text field; check clipboard_restore_delay_ms |
| High latency | Use `ggml-tiny.en.bin` model; set `threads = 0` |
| Wrong words | Add corrections to `dictionary.toml`, then reload |

---

## Privacy

WisprFree is **completely local**:

- Audio is captured, processed, and discarded in memory
- No network connections are made, ever
- No telemetry, analytics, or crash reporting
- No account creation or license keys
- The Whisper model runs entirely on your CPU/GPU

---

## License

MIT
