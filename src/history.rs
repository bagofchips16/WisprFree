//! Transcription history — append-only JSONL log with analytics.
//!
//! Each transcription is stored as a single JSON line in
//! `%APPDATA%\WisprFree\history.jsonl`.

use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};
use std::fs::{self, OpenOptions};
use std::io::{BufRead, BufReader, Write};
use std::path::PathBuf;

/// A single transcription record.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Entry {
    /// ISO-8601 timestamp.
    pub timestamp: String,
    /// The transcribed (post-processed) text.
    pub text: String,
    /// Number of words.
    pub word_count: usize,
    /// How long the user held the key (seconds).
    pub recording_secs: f32,
    /// Whisper inference time (seconds).
    pub transcription_secs: f32,
}

/// Aggregate analytics computed from history.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Analytics {
    pub total_words: usize,
    pub total_entries: usize,
    pub avg_wpm: f64,
    pub streak_days: usize,
    pub today_words: usize,
    pub today_entries: usize,
    /// Per-day word counts (last 30 days), newest first.
    pub daily: Vec<DayStat>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DayStat {
    pub date: String,
    pub words: usize,
    pub entries: usize,
}

/// Full payload sent to the dashboard.
#[derive(Debug, Serialize)]
pub struct DashboardData {
    pub analytics: Analytics,
    pub entries: Vec<Entry>,
}

fn history_path() -> Result<PathBuf> {
    let dir = crate::config::config_dir()?;
    Ok(dir.join("history.jsonl"))
}

/// Append a transcription entry to the log.
pub fn append(text: &str, recording_secs: f32, transcription_secs: f32) -> Result<()> {
    let path = history_path()?;
    // Ensure parent dir exists
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)?;
    }

    let entry = Entry {
        timestamp: chrono_now(),
        text: text.to_string(),
        word_count: text.split_whitespace().count(),
        recording_secs,
        transcription_secs,
    };

    let mut line = serde_json::to_string(&entry).context("serialize entry")?;
    line.push('\n');

    let mut file = OpenOptions::new()
        .create(true)
        .append(true)
        .open(&path)
        .context("open history file")?;
    file.write_all(line.as_bytes())?;
    Ok(())
}

/// Load all entries from the history file.
pub fn load_all() -> Result<Vec<Entry>> {
    let path = history_path()?;
    if !path.exists() {
        return Ok(Vec::new());
    }
    let file = fs::File::open(&path).context("open history")?;
    let reader = BufReader::new(file);
    let mut entries = Vec::new();
    for line in reader.lines() {
        let line = line?;
        let trimmed = line.trim();
        if trimmed.is_empty() {
            continue;
        }
        match serde_json::from_str::<Entry>(trimmed) {
            Ok(e) => entries.push(e),
            Err(err) => log::warn!("skipping malformed history line: {err}"),
        }
    }
    Ok(entries)
}

/// Compute analytics from the full history.
pub fn get_dashboard_data() -> Result<DashboardData> {
    let entries = load_all()?;
    let total_entries = entries.len();
    let total_words: usize = entries.iter().map(|e| e.word_count).sum();

    // Average WPM: total_words / total_recording_minutes
    let total_rec_mins: f64 = entries.iter().map(|e| e.recording_secs as f64).sum::<f64>() / 60.0;
    let avg_wpm = if total_rec_mins > 0.0 {
        total_words as f64 / total_rec_mins
    } else {
        0.0
    };

    // Today
    let today_str = today_date_str();
    let today_words: usize = entries
        .iter()
        .filter(|e| e.timestamp.starts_with(&today_str))
        .map(|e| e.word_count)
        .sum();
    let today_entries: usize = entries
        .iter()
        .filter(|e| e.timestamp.starts_with(&today_str))
        .count();

    // Per-day stats (last 30 days)
    let mut day_map: std::collections::BTreeMap<String, (usize, usize)> =
        std::collections::BTreeMap::new();
    for e in &entries {
        if e.timestamp.len() >= 10 {
            let day = &e.timestamp[..10];
            let stat = day_map.entry(day.to_string()).or_insert((0, 0));
            stat.0 += e.word_count;
            stat.1 += 1;
        }
    }
    let daily: Vec<DayStat> = day_map
        .iter()
        .rev()
        .take(30)
        .map(|(d, (w, c))| DayStat {
            date: d.clone(),
            words: *w,
            entries: *c,
        })
        .collect();

    // Streak: consecutive days ending today (or yesterday) with activity
    let streak_days = compute_streak(&day_map);

    let analytics = Analytics {
        total_words,
        total_entries,
        avg_wpm,
        streak_days,
        today_words,
        today_entries,
        daily,
    };

    // Return entries newest-first, limited to 200 for dashboard performance
    let mut recent = entries;
    recent.reverse();
    recent.truncate(200);

    Ok(DashboardData {
        analytics,
        entries: recent,
    })
}

fn compute_streak(day_map: &std::collections::BTreeMap<String, (usize, usize)>) -> usize {
    if day_map.is_empty() {
        return 0;
    }

    // Get sorted dates
    let dates: Vec<&String> = day_map.keys().collect();
    let today = today_date_str();

    // Streak must include today or yesterday
    let last = dates.last().map(|s| s.as_str()).unwrap_or("");
    let yesterday = yesterday_date_str();
    if last != today && last != yesterday {
        return 0;
    }

    let mut streak = 0usize;
    // Walk backwards from the most recent day
    let mut current = if last == today {
        parse_date(&today)
    } else {
        parse_date(&yesterday)
    };

    loop {
        let key = format_date(current);
        if day_map.contains_key(&key) {
            streak += 1;
            // Go to previous day
            current = prev_day(current);
        } else {
            break;
        }
    }

    streak
}

// ── Minimal date helpers (avoid pulling in chrono crate) ─────────────

fn chrono_now() -> String {
    // Use Windows SYSTEMTIME for wall-clock time
    use windows::Win32::System::SystemInformation::GetLocalTime;
    let st = unsafe { GetLocalTime() };
    format!(
        "{:04}-{:02}-{:02}T{:02}:{:02}:{:02}",
        st.wYear, st.wMonth, st.wDay, st.wHour, st.wMinute, st.wSecond
    )
}

fn today_date_str() -> String {
    use windows::Win32::System::SystemInformation::GetLocalTime;
    let st = unsafe { GetLocalTime() };
    format!("{:04}-{:02}-{:02}", st.wYear, st.wMonth, st.wDay)
}

fn yesterday_date_str() -> String {
    let today = parse_date(&today_date_str());
    let y = prev_day(today);
    format_date(y)
}

/// (year, month, day) tuple
type DateTuple = (i32, u32, u32);

fn parse_date(s: &str) -> DateTuple {
    let parts: Vec<&str> = s.split('-').collect();
    (
        parts[0].parse().unwrap_or(2026),
        parts[1].parse().unwrap_or(1),
        parts[2].parse().unwrap_or(1),
    )
}

fn format_date(d: DateTuple) -> String {
    format!("{:04}-{:02}-{:02}", d.0, d.1, d.2)
}

fn days_in_month(year: i32, month: u32) -> u32 {
    match month {
        1 | 3 | 5 | 7 | 8 | 10 | 12 => 31,
        4 | 6 | 9 | 11 => 30,
        2 => {
            if (year % 4 == 0 && year % 100 != 0) || year % 400 == 0 {
                29
            } else {
                28
            }
        }
        _ => 30,
    }
}

fn prev_day(d: DateTuple) -> DateTuple {
    let (y, m, day) = d;
    if day > 1 {
        (y, m, day - 1)
    } else if m > 1 {
        (y, m - 1, days_in_month(y, m - 1))
    } else {
        (y - 1, 12, 31)
    }
}
