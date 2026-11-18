//! macOS: `ungive/mediaremote-adapter`.
//!
//! Since macOS 15.4 third-party apps can't read MediaRemote "Now Playing"
//! directly. The adapter runs a bundled Perl script through the system
//! `/usr/bin/perl`, which is allowed to load `MediaRemoteAdapter.framework`.
//! We run its `stream` command as a child process and merge the line-delimited
//! JSON it prints. If the adapter is missing or keeps failing we fall back to
//! polling Spotify / Music through AppleScript.

use super::{friendly_app_name, now_ms, MediaCommand, NowPlayingSource, RawNowPlaying, SnapshotSender};
use base64::Engine as _;
use serde_json::{Map, Value};
use std::io::{BufRead, BufReader};
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

const PERL: &str = "/usr/bin/perl";
const BACKOFF: [u64; 5] = [1, 2, 5, 15, 30];

pub struct AdapterSource {
    script: PathBuf,
    framework: PathBuf,
    /// Which app the fallback last saw ("Spotify" / "Music"), for commands.
    fallback_app: Arc<Mutex<Option<String>>>,
    using_fallback: Arc<Mutex<bool>>,
}

impl AdapterSource {
    pub fn new(resource_dir: Option<PathBuf>) -> Self {
        let base = resource_dir.unwrap_or_default().join("adapter");
        Self {
            script: base.join("mediaremote-adapter.pl"),
            framework: base.join("MediaRemoteAdapter.framework"),
            fallback_app: Arc::new(Mutex::new(None)),
            using_fallback: Arc::new(Mutex::new(false)),
        }
    }

    fn adapter_available(&self) -> bool {
        Path::new(PERL).exists() && self.script.exists() && self.framework.exists()
    }
}

impl NowPlayingSource for AdapterSource {
    fn start(&self, tx: SnapshotSender) {
        let script = self.script.clone();
        let framework = self.framework.clone();
        let available = self.adapter_available();
        let fallback_app = self.fallback_app.clone();
        let using_fallback = self.using_fallback.clone();
        std::thread::Builder::new()
            .name("mediaremote".into())
            .spawn(move || {
                if available {
                    let mut failures = 0usize;
                    loop {
                        let started = Instant::now();
                        match stream_once(&script, &framework, &tx) {
                            Ok(StreamEnd::ReceiverGone) => return,
                            Ok(StreamEnd::Exited(status)) => log::warn!("mediaremote-adapter exited: {status}"),
                            Err(e) => log::warn!("mediaremote-adapter failed: {e}"),
                        }
                        let _ = tx.send(None);
                        // A run that lasted a while resets the backoff.
                        if started.elapsed() > Duration::from_secs(60) {
                            failures = 0;
                        }
                        // Repeated instant failures: the adapter doesn't work on this system.
                        if failures >= 4 && started.elapsed() < Duration::from_secs(2) {
                            log::warn!("mediaremote-adapter keeps failing; falling back to AppleScript");
                            break;
                        }
                        let wait = BACKOFF[failures.min(BACKOFF.len() - 1)];
                        failures += 1;
                        std::thread::sleep(Duration::from_secs(wait));
                    }
                } else {
                    log::warn!("mediaremote-adapter not bundled; using AppleScript fallback");
                }
                *using_fallback.lock().unwrap() = true;
                applescript_loop(&tx, &fallback_app);
            })
            .expect("spawn mediaremote thread");
    }

    fn command(&self, cmd: MediaCommand) {
        if *self.using_fallback.lock().unwrap() {
            let app = self.fallback_app.lock().unwrap().clone();
            if let Some(app) = app {
                let verb = match cmd {
                    MediaCommand::TogglePlayPause => "playpause",
                    MediaCommand::Next => "next track",
                    MediaCommand::Previous => "previous track",
                };
                let _ = osascript(&format!("tell application \"{app}\" to {verb}"));
            }
            return;
        }
        // MRMediaRemoteCommand: 2 = toggle play/pause, 4 = next, 5 = previous.
        let id = match cmd {
            MediaCommand::TogglePlayPause => "2",
            MediaCommand::Next => "4",
            MediaCommand::Previous => "5",
        };
        let result = Command::new(PERL)
            .arg(&self.script)
            .arg(&self.framework)
            .arg("send")
            .arg(id)
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .status();
        if let Err(e) = result {
            log::warn!("mediaremote-adapter send failed: {e}");
        }
    }
}

enum StreamEnd {
    Exited(String),
    ReceiverGone,
}

fn stream_once(script: &Path, framework: &Path, tx: &SnapshotSender) -> std::io::Result<StreamEnd> {
    let mut child = Command::new(PERL)
        .arg(script)
        .arg(framework)
        .arg("stream")
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()?;
    let stdout = child.stdout.take().expect("piped stdout");
    let mut state = Map::new();
    let mut artwork: Option<(String, Arc<Vec<u8>>)> = None;

    for line in BufReader::new(stdout).lines() {
        let line = line?;
        let Ok(msg) = serde_json::from_str::<Value>(&line) else { continue };
        merge_message(&mut state, &msg);
        let snapshot = snapshot_from_state(&state, &mut artwork);
        if tx.send(snapshot).is_err() {
            let _ = child.kill();
            return Ok(StreamEnd::ReceiverGone);
        }
    }
    let status = child.wait()?;
    Ok(StreamEnd::Exited(status.to_string()))
}

/// Applies one adapter message to the merged state. Messages look like
/// `{"type":"data","diff":true,"payload":{...}}`; diffs only carry changed keys
/// (a `null` value removes a key). Bare payload objects are accepted too.
pub(crate) fn merge_message(state: &mut Map<String, Value>, msg: &Value) {
    let (payload, diff) = match msg.get("payload") {
        Some(p) => (p, msg.get("diff").and_then(Value::as_bool).unwrap_or(false)),
        None => (msg, false),
    };
    let Some(obj) = payload.as_object() else {
        if !diff {
            state.clear();
        }
        return;
    };
    if !diff {
        state.clear();
    }
    for (k, v) in obj {
        if v.is_null() {
            state.remove(k);
        } else {
            state.insert(k.clone(), v.clone());
        }
    }
}

pub(crate) fn snapshot_from_state(
    state: &Map<String, Value>,
    artwork: &mut Option<(String, Arc<Vec<u8>>)>,
) -> Option<RawNowPlaying> {
    let s = |k: &str| state.get(k).and_then(Value::as_str).unwrap_or("").to_string();
    let f = |k: &str| state.get(k).and_then(Value::as_f64);
    let title = s("title");
    if title.trim().is_empty() {
        return None;
    }
    let playing = state.get("playing").and_then(Value::as_bool).unwrap_or(false);
    let duration_ms = f("duration").map(|d| (d * 1000.0).max(0.0) as u64).unwrap_or(0);
    let rate = f("playbackRate").unwrap_or(if playing { 1.0 } else { 0.0 });
    let now = now_ms();

    let mut position_ms = f("elapsedTime").map(|e| (e * 1000.0).max(0.0)).unwrap_or(0.0);
    if playing {
        if let Some(ts) = state.get("timestamp").and_then(parse_timestamp_ms) {
            let elapsed = now as f64 - ts as f64;
            if elapsed > 0.0 {
                position_ms += elapsed * if rate > 0.0 { rate } else { 1.0 };
            }
        }
    }
    let mut position_ms = position_ms as u64;
    if duration_ms > 0 {
        position_ms = position_ms.min(duration_ms);
    }

    // Artwork is base64; only decode when it changes.
    let art_b64 = s("artworkData");
    let art = if art_b64.is_empty() {
        *artwork = None;
        None
    } else {
        match artwork {
            Some((prev, bytes)) if *prev == art_b64 => Some(bytes.clone()),
            _ => {
                let cleaned: String = art_b64.chars().filter(|c| !c.is_whitespace()).collect();
                match base64::engine::general_purpose::STANDARD.decode(cleaned.as_bytes()) {
                    Ok(bytes) => {
                        let bytes = Arc::new(bytes);
                        *artwork = Some((art_b64, bytes.clone()));
                        Some(bytes)
                    }
                    Err(_) => None,
                }
            }
        }
    };

    let bundle = {
        let parent = s("parentApplicationBundleIdentifier");
        if parent.is_empty() { s("bundleIdentifier") } else { parent }
    };
    let album = s("album");
    Some(RawNowPlaying {
        source_app: friendly_app_name(&bundle),
        source_id: bundle,
        title,
        artist: s("artist"),
        album: if album.trim().is_empty() { None } else { Some(album) },
        duration_ms,
        position_ms,
        position_at: now,
        playing,
        artwork: art,
    })
}

/// Accepts epoch seconds / milliseconds or an ISO-8601 / RFC 3339 string.
pub(crate) fn parse_timestamp_ms(v: &Value) -> Option<u64> {
    if let Some(n) = v.as_f64() {
        return Some(if n > 1e12 { n as u64 } else { (n * 1000.0) as u64 });
    }
    parse_rfc3339_ms(v.as_str()?)
}

/// Minimal RFC 3339 parser: `YYYY-MM-DDTHH:MM:SS(.fff)?(Z|±HH:MM)`.
pub(crate) fn parse_rfc3339_ms(s: &str) -> Option<u64> {
    let s = s.trim();
    let b = s.as_bytes();
    if b.len() < 19 {
        return None;
    }
    let num = |r: std::ops::Range<usize>| -> Option<i64> { s.get(r)?.parse().ok() };
    let (y, mo, d) = (num(0..4)?, num(5..7)?, num(8..10)?);
    let (h, mi, sec) = (num(11..13)?, num(14..16)?, num(17..19)?);
    let mut i = 19;
    let mut frac_ms = 0i64;
    if b.get(i) == Some(&b'.') {
        i += 1;
        let start = i;
        while i < b.len() && b[i].is_ascii_digit() {
            i += 1;
        }
        let digits = &s[start..i];
        let padded = format!("{:0<3}", &digits[..digits.len().min(3)]);
        frac_ms = padded.parse().ok()?;
    }
    let offset_min = match b.get(i) {
        None | Some(b'Z') | Some(b'z') => 0,
        Some(&c) if c == b'+' || c == b'-' => {
            let oh: i64 = s.get(i + 1..i + 3)?.parse().ok()?;
            let om: i64 = s.get(i + 4..i + 6).and_then(|x| x.parse().ok()).unwrap_or(0);
            let sign = if c == b'+' { 1 } else { -1 };
            sign * (oh * 60 + om)
        }
        _ => return None,
    };
    // Days since the epoch (Howard Hinnant's algorithm).
    let y2 = if mo <= 2 { y - 1 } else { y };
    let era = if y2 >= 0 { y2 } else { y2 - 399 } / 400;
    let yoe = y2 - era * 400;
    let mp = (mo + 9) % 12;
    let doy = (153 * mp + 2) / 5 + d - 1;
    let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy;
    let days = era * 146_097 + doe - 719_468;
    let secs = days * 86_400 + h * 3600 + mi * 60 + sec - offset_min * 60;
    if secs < 0 {
        return None;
    }
    Some((secs * 1000 + frac_ms) as u64)
}

// ------------------------------------------------------------------ AppleScript fallback

fn osascript(script: &str) -> Option<String> {
    let out = Command::new("/usr/bin/osascript")
        .arg("-e")
        .arg(script)
        .stdin(Stdio::null())
        .stderr(Stdio::null())
        .output()
        .ok()?;
    if !out.status.success() {
        return None;
    }
    Some(String::from_utf8_lossy(&out.stdout).trim().to_string())
}

fn is_running(process: &str) -> bool {
    Command::new("/usr/bin/pgrep")
        .arg("-x")
        .arg(process)
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status()
        .map(|s| s.success())
        .unwrap_or(false)
}

/// Polls Spotify and Music once a second. No artwork: Spotify only exposes a URL
/// and we don't make network calls other than LRCLIB.
fn applescript_loop(tx: &SnapshotSender, fallback_app: &Mutex<Option<String>>) {
    // Durations: Spotify reports ms, Music reports seconds.
    let apps: [(&str, &str, f64); 2] = [("Spotify", "Spotify", 1.0), ("Music", "Music", 1000.0)];
    loop {
        let mut best: Option<RawNowPlaying> = None;
        for (process, app, dur_scale) in apps {
            if !is_running(process) {
                continue;
            }
            // `run script` keeps the tell block from being compiled (and the app
            // launched) unless it is already running.
            let script = format!(
                r#"run script "tell application \"{app}\"
  set s to player state as string
  if s is \"stopped\" then return \"\"
  set t to current track
  return s & tab & (name of t) & tab & (artist of t) & tab & (album of t) & tab & (duration of t) & tab & (player position)
end tell""#
            );
            let Some(out) = osascript(&script) else { continue };
            let parts: Vec<&str> = out.split('\t').collect();
            if parts.len() < 6 || parts[1].is_empty() {
                continue;
            }
            let num = |x: &str| x.trim().replace(',', ".").parse::<f64>().unwrap_or(0.0);
            let playing = parts[0] == "playing";
            let snap = RawNowPlaying {
                source_app: app.to_string(),
                source_id: app.to_string(),
                title: parts[1].to_string(),
                artist: parts[2].to_string(),
                album: Some(parts[3].to_string()).filter(|a| !a.is_empty()),
                duration_ms: (num(parts[4]) * dur_scale) as u64,
                position_ms: (num(parts[5]) * 1000.0) as u64,
                position_at: now_ms(),
                playing,
                artwork: None,
            };
            let replace = match &best {
                None => true,
                Some(b) => !b.playing && playing,
            };
            if replace {
                best = Some(snap);
            }
        }
        *fallback_app.lock().unwrap() = best.as_ref().map(|b| b.source_app.clone());
        if tx.send(best).is_err() {
            return;
        }
        std::thread::sleep(Duration::from_secs(1));
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn rfc3339() {
        assert_eq!(parse_rfc3339_ms("1970-01-01T00:00:00Z"), Some(0));
        assert_eq!(parse_rfc3339_ms("2025-05-01T12:00:00.250Z"), Some(1_746_100_800_250));
        assert_eq!(parse_rfc3339_ms("2025-05-01T14:00:00+02:00"), Some(1_746_100_800_000));
        assert_eq!(parse_rfc3339_ms("nonsense"), None);
    }

    #[test]
    fn merges_diffs() {
        let mut st = Map::new();
        merge_message(&mut st, &json!({"type":"data","diff":false,"payload":{"title":"A","artist":"B","playing":true}}));
        merge_message(&mut st, &json!({"type":"data","diff":true,"payload":{"playing":false,"artist":null}}));
        assert_eq!(st.get("title"), Some(&json!("A")));
        assert_eq!(st.get("playing"), Some(&json!(false)));
        assert!(st.get("artist").is_none());
        merge_message(&mut st, &json!({"type":"data","diff":false,"payload":{}}));
        assert!(st.is_empty());
    }

    #[test]
    fn snapshot_extrapolates() {
        let mut st = Map::new();
        let ts = now_ms() - 2000;
        merge_message(
            &mut st,
            &json!({"payload":{"title":"T","artist":"A","playing":true,"duration":200.0,"elapsedTime":10.0,"timestamp":ts as f64 / 1000.0,"playbackRate":1.0,"bundleIdentifier":"com.spotify.client"}}),
        );
        let mut art = None;
        let s = snapshot_from_state(&st, &mut art).unwrap();
        assert!(s.position_ms >= 11_900 && s.position_ms <= 12_500, "{}", s.position_ms);
        assert_eq!(s.source_app, "Spotify");
        assert_eq!(s.duration_ms, 200_000);
    }
}
