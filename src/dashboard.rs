//! Tiny HTTP dashboard server — serves analytics on `localhost:9876`.
//!
//! Runs on a background thread; no external dependencies (uses `std::net`).
//! The single-page app is embedded as a const HTML string.

use std::io::{Read, Write};
use std::net::{TcpListener, TcpStream};
use std::os::windows::process::CommandExt;

const PORT: u16 = 9876;

/// Start the dashboard HTTP server on a background thread.
pub fn start() {
    std::thread::Builder::new()
        .name("dashboard-http".into())
        .spawn(|| {
            if let Err(e) = run_server() {
                log::error!("dashboard server failed: {e:#}");
            }
        })
        .expect("spawn dashboard thread");
    log::info!("dashboard server started on http://localhost:{PORT}");
}

fn run_server() -> anyhow::Result<()> {
    let listener = TcpListener::bind(format!("127.0.0.1:{PORT}"))?;

    for stream in listener.incoming() {
        match stream {
            Ok(stream) => {
                // Handle each request on a short-lived thread to avoid blocking
                std::thread::spawn(move || {
                    if let Err(e) = handle_request(stream) {
                        log::debug!("dashboard request error: {e:#}");
                    }
                });
            }
            Err(e) => log::debug!("dashboard accept error: {e}"),
        }
    }
    Ok(())
}

fn handle_request(mut stream: TcpStream) -> anyhow::Result<()> {
    // Read the request (we only need the first line)
    let mut buf = [0u8; 4096];
    let n = stream.read(&mut buf)?;
    let request = String::from_utf8_lossy(&buf[..n]);
    let first_line = request.lines().next().unwrap_or("");

    if first_line.starts_with("GET /api/data") {
        // Serve analytics JSON
        let data = crate::history::get_dashboard_data()?;
        let json = serde_json::to_string(&data)?;
        let response = format!(
            "HTTP/1.1 200 OK\r\n\
             Content-Type: application/json\r\n\
             Content-Length: {}\r\n\
             Access-Control-Allow-Origin: *\r\n\
             Cache-Control: no-cache\r\n\
             Connection: close\r\n\
             \r\n\
             {}",
            json.len(),
            json
        );
        stream.write_all(response.as_bytes())?;
    } else if first_line.starts_with("GET /") || first_line.starts_with("GET / ") {
        // Serve the dashboard HTML
        let html = DASHBOARD_HTML;
        let response = format!(
            "HTTP/1.1 200 OK\r\n\
             Content-Type: text/html; charset=utf-8\r\n\
             Content-Length: {}\r\n\
             Connection: close\r\n\
             \r\n\
             {}",
            html.len(),
            html
        );
        stream.write_all(response.as_bytes())?;
    } else {
        let response = "HTTP/1.1 404 Not Found\r\nContent-Length: 0\r\nConnection: close\r\n\r\n";
        stream.write_all(response.as_bytes())?;
    }
    stream.flush()?;
    Ok(())
}

/// Open the dashboard as a standalone app window (no browser chrome).
/// Tries Edge --app mode first (looks like a native app), falls back to default browser.
pub fn open_in_browser() {
    let url = format!("http://localhost:{PORT}");

    // Try Microsoft Edge in app mode (standalone window, no tabs/address bar)
    let edge_paths = [
        r"C:\Program Files (x86)\Microsoft\Edge\Application\msedge.exe",
        r"C:\Program Files\Microsoft\Edge\Application\msedge.exe",
    ];

    for edge in &edge_paths {
        if std::path::Path::new(edge).exists() {
            // Use a dedicated user-data-dir so Edge opens a true app window
            // even if a normal Edge browser session is already running
            let app_data = std::env::var("LOCALAPPDATA").unwrap_or_default();
            let user_dir = std::path::PathBuf::from(app_data).join("WisprFree\\EdgeApp");
            let _ = std::process::Command::new(edge)
                .arg(format!("--app={url}"))
                .arg(format!("--user-data-dir={}", user_dir.display()))
                .arg("--no-first-run")
                .arg("--disable-default-apps")
                .creation_flags(0x08000000)
                .spawn();
            return;
        }
    }

    // Fallback: default browser
    let _ = std::process::Command::new("cmd")
        .args(["/C", &format!("start {url}")])
        .creation_flags(0x08000000)
        .spawn();
}

const DASHBOARD_HTML: &str = r##"<!DOCTYPE html>
<html lang="en">
<head>
<meta charset="UTF-8">
<meta name="viewport" content="width=device-width, initial-scale=1.0">
<title>WisprFree Dashboard</title>
<style>
  :root {
    --bg: #0f0f0f;
    --surface: #1a1a1a;
    --surface2: #242424;
    --border: #333;
    --text: #e8e8e8;
    --text2: #999;
    --accent: #2ecc71;
    --accent2: #27ae60;
    --red: #e74c3c;
    --yellow: #f39c12;
  }
  * { margin: 0; padding: 0; box-sizing: border-box; }
  body {
    font-family: -apple-system, BlinkMacSystemFont, 'Segoe UI', Roboto, sans-serif;
    background: var(--bg);
    color: var(--text);
    min-height: 100vh;
    padding: 0;
  }

  /* ── Header ─────────────────────────────────── */
  header {
    background: var(--surface);
    border-bottom: 1px solid var(--border);
    padding: 20px 32px;
    display: flex;
    align-items: center;
    justify-content: space-between;
  }
  header h1 {
    font-size: 22px;
    font-weight: 600;
    letter-spacing: -0.5px;
  }
  header h1 span {
    color: var(--accent);
  }
  .header-right {
    display: flex;
    align-items: center;
    gap: 16px;
  }
  .status-dot {
    width: 8px; height: 8px;
    background: var(--accent);
    border-radius: 50%;
    animation: pulse 2s infinite;
  }
  @keyframes pulse {
    0%, 100% { opacity: 1; }
    50% { opacity: 0.4; }
  }
  .status-text { color: var(--text2); font-size: 13px; }
  .refresh-btn {
    background: var(--surface2);
    border: 1px solid var(--border);
    color: var(--text);
    padding: 6px 14px;
    border-radius: 6px;
    cursor: pointer;
    font-size: 13px;
    transition: background 0.2s;
  }
  .refresh-btn:hover { background: var(--border); }

  /* ── Main layout ────────────────────────────── */
  .container { max-width: 1100px; margin: 0 auto; padding: 28px 32px; }

  /* ── Stat cards ─────────────────────────────── */
  .stats {
    display: grid;
    grid-template-columns: repeat(4, 1fr);
    gap: 16px;
    margin-bottom: 28px;
  }
  .stat-card {
    background: var(--surface);
    border: 1px solid var(--border);
    border-radius: 12px;
    padding: 20px 24px;
  }
  .stat-label {
    font-size: 12px;
    text-transform: uppercase;
    letter-spacing: 1px;
    color: var(--text2);
    margin-bottom: 8px;
  }
  .stat-value {
    font-size: 36px;
    font-weight: 700;
    letter-spacing: -1px;
    line-height: 1;
  }
  .stat-unit {
    font-size: 16px;
    font-weight: 400;
    color: var(--text2);
    margin-left: 4px;
  }
  .stat-sub {
    font-size: 13px;
    color: var(--text2);
    margin-top: 6px;
  }
  .accent-value { color: var(--accent); }

  /* ── Chart section ──────────────────────────── */
  .chart-section {
    background: var(--surface);
    border: 1px solid var(--border);
    border-radius: 12px;
    padding: 24px;
    margin-bottom: 28px;
  }
  .chart-title {
    font-size: 15px;
    font-weight: 600;
    margin-bottom: 16px;
  }
  .chart {
    display: flex;
    align-items: flex-end;
    gap: 4px;
    height: 120px;
    padding-top: 8px;
  }
  .chart-bar-wrap {
    flex: 1;
    display: flex;
    flex-direction: column;
    align-items: center;
    height: 100%;
    justify-content: flex-end;
  }
  .chart-bar {
    width: 100%;
    max-width: 32px;
    background: var(--accent);
    border-radius: 4px 4px 0 0;
    min-height: 2px;
    transition: height 0.3s;
    position: relative;
  }
  .chart-bar:hover {
    background: var(--accent2);
  }
  .chart-bar-label {
    font-size: 10px;
    color: var(--text2);
    margin-top: 6px;
    white-space: nowrap;
  }
  .chart-tooltip {
    display: none;
    position: absolute;
    bottom: calc(100% + 6px);
    left: 50%;
    transform: translateX(-50%);
    background: var(--surface2);
    border: 1px solid var(--border);
    padding: 4px 8px;
    border-radius: 4px;
    font-size: 11px;
    white-space: nowrap;
    color: var(--text);
    z-index: 10;
  }
  .chart-bar:hover .chart-tooltip { display: block; }

  /* ── Sections row ───────────────────────────── */
  .sections {
    display: grid;
    grid-template-columns: 1fr;
    gap: 16px;
  }

  /* ── History log ────────────────────────────── */
  .log-section {
    background: var(--surface);
    border: 1px solid var(--border);
    border-radius: 12px;
    padding: 24px;
  }
  .log-title {
    font-size: 15px;
    font-weight: 600;
    margin-bottom: 16px;
    display: flex;
    justify-content: space-between;
    align-items: center;
  }
  .log-count { font-size: 13px; color: var(--text2); font-weight: 400; }

  .log-entry {
    padding: 12px 0;
    border-bottom: 1px solid var(--border);
    display: flex;
    gap: 16px;
    align-items: flex-start;
  }
  .log-entry:last-child { border-bottom: none; }
  .log-time {
    font-size: 13px;
    color: var(--text2);
    white-space: nowrap;
    min-width: 80px;
    flex-shrink: 0;
    padding-top: 1px;
  }
  .log-text {
    font-size: 14px;
    line-height: 1.5;
    flex: 1;
    word-break: break-word;
  }
  .log-meta {
    font-size: 12px;
    color: var(--text2);
    white-space: nowrap;
    flex-shrink: 0;
    padding-top: 2px;
  }

  /* ── Empty state ────────────────────────────── */
  .empty-state {
    text-align: center;
    padding: 60px 20px;
    color: var(--text2);
  }
  .empty-state h2 {
    font-size: 18px;
    margin-bottom: 8px;
    color: var(--text);
  }
  .empty-state p { font-size: 14px; }

  /* ── Today highlight ────────────────────────── */
  .today-section {
    display: flex;
    align-items: center;
    gap: 6px;
    font-size: 13px;
    color: var(--text2);
    margin-bottom: 10px;
    padding: 8px 12px;
    background: var(--surface2);
    border-radius: 8px;
  }
  .today-section strong { color: var(--text); }

  /* ── Responsive ─────────────────────────────── */
  @media (max-width: 700px) {
    .stats { grid-template-columns: repeat(2, 1fr); }
    header { padding: 16px 20px; }
    .container { padding: 20px; }
  }
</style>
</head>
<body>
  <header>
    <h1><span>Wispr</span>Free Dashboard</h1>
    <div class="header-right">
      <div class="status-dot"></div>
      <span class="status-text">Live</span>
      <button class="refresh-btn" onclick="loadData()">↻ Refresh</button>
    </div>
  </header>

  <div class="container" id="app">
    <div class="empty-state">
      <h2>Loading...</h2>
      <p>Connecting to WisprFree</p>
    </div>
  </div>

<script>
async function loadData() {
  try {
    const resp = await fetch('/api/data');
    const data = await resp.json();
    render(data);
  } catch (e) {
    document.getElementById('app').innerHTML = `
      <div class="empty-state">
        <h2>Cannot connect</h2>
        <p>Make sure WisprFree is running, then refresh.</p>
      </div>`;
  }
}

function render(data) {
  const { analytics: a, entries } = data;
  const app = document.getElementById('app');

  // Format numbers
  const fmtNum = n => n.toLocaleString();
  const wpm = Math.round(a.avg_wpm);
  const streak = a.streak_days;

  // Chart data (reverse to chronological order)
  const daily = [...a.daily].reverse();
  const maxWords = Math.max(...daily.map(d => d.words), 1);

  let chartBars = '';
  if (daily.length > 0) {
    chartBars = daily.map(d => {
      const h = Math.max(2, (d.words / maxWords) * 100);
      const label = d.date.slice(5); // MM-DD
      return `<div class="chart-bar-wrap">
        <div class="chart-bar" style="height:${h}%">
          <div class="chart-tooltip">${d.date}: ${fmtNum(d.words)} words, ${d.entries} entries</div>
        </div>
        <div class="chart-bar-label">${label}</div>
      </div>`;
    }).join('');
  }

  // Log entries
  let logHtml = '';
  if (entries.length === 0) {
    logHtml = `<div class="empty-state">
      <h2>No transcriptions yet</h2>
      <p>Hold Ctrl+Space and speak to get started!</p>
    </div>`;
  } else {
    logHtml = entries.map(e => {
      const time = e.timestamp.slice(11, 16); // HH:MM
      const date = e.timestamp.slice(0, 10);
      const wc = e.word_count;
      const dur = e.recording_secs.toFixed(1);
      return `<div class="log-entry">
        <div class="log-time">${date}<br>${time}</div>
        <div class="log-text">${escapeHtml(e.text)}</div>
        <div class="log-meta">${wc} words · ${dur}s</div>
      </div>`;
    }).join('');
  }

  app.innerHTML = `
    <div class="stats">
      <div class="stat-card">
        <div class="stat-label">Total Words</div>
        <div class="stat-value accent-value">${fmtNum(a.total_words)}</div>
        <div class="stat-sub">${fmtNum(a.total_entries)} transcriptions</div>
      </div>
      <div class="stat-card">
        <div class="stat-label">Average Speed</div>
        <div class="stat-value">${wpm}<span class="stat-unit">wpm</span></div>
        <div class="stat-sub">words per minute</div>
      </div>
      <div class="stat-card">
        <div class="stat-label">Day Streak</div>
        <div class="stat-value accent-value">${streak}<span class="stat-unit">days</span></div>
        <div class="stat-sub">consecutive active days</div>
      </div>
      <div class="stat-card">
        <div class="stat-label">Today</div>
        <div class="stat-value">${fmtNum(a.today_words)}<span class="stat-unit">words</span></div>
        <div class="stat-sub">${a.today_entries} transcriptions today</div>
      </div>
    </div>

    ${daily.length > 1 ? `
    <div class="chart-section">
      <div class="chart-title">Daily Activity (last ${daily.length} days)</div>
      <div class="chart">${chartBars}</div>
    </div>` : ''}

    <div class="sections">
      <div class="log-section">
        <div class="log-title">
          <span>Transcription Log</span>
          <span class="log-count">${entries.length} recent entries</span>
        </div>
        ${logHtml}
      </div>
    </div>`;
}

function escapeHtml(s) {
  const d = document.createElement('div');
  d.textContent = s;
  return d.innerHTML;
}

// Load on start and auto-refresh every 30 seconds
loadData();
setInterval(loadData, 30000);
</script>
</body>
</html>"##;
