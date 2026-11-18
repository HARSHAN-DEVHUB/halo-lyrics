//! "What's playing" sources. Each OS has one implementation behind
//! [`NowPlayingSource`]; both emit [`RawNowPlaying`] snapshots into a channel
//! that the hub (`hub.rs`) turns into `now-playing` events.

use serde::{Deserialize, Serialize};
use std::sync::mpsc::Sender;
use std::sync::Arc;
use std::time::{SystemTime, UNIX_EPOCH};

#[cfg(target_os = "macos")]
pub mod macos;
#[cfg(windows)]
pub mod windows;

/// Snapshot straight from the OS, before artwork processing.
#[derive(Clone, Debug, Default)]
pub struct RawNowPlaying {
    /// Friendly app name ("Spotify", "Chrome", "Music", ...).
    pub source_app: String,
    /// Stable id of the session (AUMID / bundle id) — not shown to users.
    pub source_id: String,
    pub title: String,
    pub artist: String,
    pub album: Option<String>,
    pub duration_ms: u64,
    /// Live position at `position_at` (already extrapolated by the source).
    pub position_ms: u64,
    /// Epoch ms.
    pub position_at: u64,
    pub playing: bool,
    /// Raw artwork bytes (any common image format). The same `Arc` is reused
    /// while the artwork is unchanged, so the hub can skip re-processing.
    pub artwork: Option<Arc<Vec<u8>>>,
}

/// Commands sent back to the OS media session.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum MediaCommand {
    TogglePlayPause,
    Next,
    Previous,
}

pub type SnapshotSender = Sender<Option<RawNowPlaying>>;

pub trait NowPlayingSource: Send + Sync {
    /// Start watching. Must return quickly; work happens on background threads.
    /// Sends `None` when nothing is available.
    fn start(&self, tx: SnapshotSender);
    /// Send a transport command to the current session.
    fn command(&self, cmd: MediaCommand);
}

/// The source for the current OS.
pub fn platform_source(resource_dir: Option<std::path::PathBuf>) -> Box<dyn NowPlayingSource> {
    #[cfg(windows)]
    {
        let _ = resource_dir;
        Box::new(windows::SmtcSource::new())
    }
    #[cfg(target_os = "macos")]
    {
        Box::new(macos::AdapterSource::new(resource_dir))
    }
    #[cfg(not(any(windows, target_os = "macos")))]
    {
        let _ = resource_dir;
        Box::new(NullSource)
    }
}

/// Fallback for unsupported platforms (keeps the crate compiling elsewhere).
#[cfg(not(any(windows, target_os = "macos")))]
pub struct NullSource;
#[cfg(not(any(windows, target_os = "macos")))]
impl NowPlayingSource for NullSource {
    fn start(&self, tx: SnapshotSender) {
        let _ = tx.send(None);
    }
    fn command(&self, _cmd: MediaCommand) {}
}

pub fn now_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_millis() as u64)
        .unwrap_or(0)
}

/// Turns ids like "Spotify.exe", "SpotifyAB.SpotifyMusic_zpdnekdrzrea0!Spotify",
/// "com.spotify.client" or "MSEdge" into a display name.
pub fn friendly_app_name(id: &str) -> String {
    let lower = id.to_lowercase();
    let known: &[(&str, &str)] = &[
        ("spotify", "Spotify"),
        ("applemusic", "Apple Music"),
        ("com.apple.music", "Music"),
        ("itunes", "iTunes"),
        ("tidal", "Tidal"),
        ("deezer", "Deezer"),
        ("amazon", "Amazon Music"),
        ("vlc", "VLC"),
        ("foobar", "foobar2000"),
        ("musicbee", "MusicBee"),
        ("aimp", "AIMP"),
        ("zunemusic", "Media Player"),
        ("microsoft.media", "Media Player"),
        ("msedge", "Edge"),
        ("chrome", "Chrome"),
        ("firefox", "Firefox"),
        ("308046b0af4a39cb", "Firefox"),
        ("opera", "Opera"),
        ("brave", "Brave"),
        ("vivaldi", "Vivaldi"),
        ("thebrowser", "Arc"),
        ("safari", "Safari"),
        ("youtube", "YouTube Music"),
    ];
    for (needle, name) in known {
        if lower.contains(needle) {
            return (*name).to_string();
        }
    }
    // "Something.exe" → "Something"; "com.vendor.App" → "App"
    let base = id.rsplit(['\\', '/']).next().unwrap_or(id);
    let base = base.trim_end_matches(".exe").trim_end_matches(".EXE");
    let base = base.rsplit('.').next().unwrap_or(base);
    let base = base.split('!').next_back().unwrap_or(base);
    if base.is_empty() {
        "Unknown".into()
    } else {
        base.to_string()
    }
}

/// Browsers report the page/video title; this helps the normaliser decide
/// whether "Artist - Title" splitting is worth trying.
pub fn is_browser(app: &str) -> bool {
    matches!(
        app,
        "Chrome" | "Edge" | "Firefox" | "Opera" | "Brave" | "Vivaldi" | "Arc" | "Safari"
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn friendly_names() {
        assert_eq!(friendly_app_name("Spotify.exe"), "Spotify");
        assert_eq!(friendly_app_name("SpotifyAB.SpotifyMusic_zpdnekdrzrea0!Spotify"), "Spotify");
        assert_eq!(friendly_app_name("MSEdge"), "Edge");
        assert_eq!(friendly_app_name("Chrome"), "Chrome");
        assert_eq!(friendly_app_name("com.apple.Music"), "Music");
        assert_eq!(friendly_app_name("C:\\Apps\\Strange.exe"), "Strange");
    }
}
