//! User settings, stored as JSON in the app config dir.

use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::path::Path;

/// How the lyrics are laid out. The first entry is the default.
pub const LYRICS_STYLES: &[&str] = &["cinematic", "karaoke", "minimal", "party"];
/// What sits behind the lyrics. The first entry is the default.
pub const BACKGROUND_STYLES: &[&str] = &["artwork", "gradient", "dark", "split"];
/// How the lyrics are shown: covering the screen, or as a normal window.
pub const WINDOW_MODES: &[&str] = &["fullscreen", "window"];

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
#[serde(rename_all = "camelCase", default)]
pub struct Settings {
    /// Active word size in px (72..=200).
    pub word_size: u32,
    /// Background blur in px (0..=60).
    pub blur: u32,
    /// Added to the playback position for lyrics only (-2000..=2000 ms).
    pub offset_ms: i32,
    /// Monitor name; `None` = primary.
    pub monitor: Option<String>,
    pub mirror_all: bool,
    pub auto_open: bool,
    /// 0 = never.
    pub auto_close_minutes: u32,
    pub launch_at_login: bool,
    pub hide_player_bar: bool,
    /// One of [`LYRICS_STYLES`].
    pub lyrics_style: String,
    /// One of [`BACKGROUND_STYLES`].
    pub background_style: String,
    /// Look up a sharper cover online when the player's artwork is small.
    pub hd_artwork: bool,
    /// One of [`WINDOW_MODES`].
    pub window_mode: String,
    /// The window's close button quits the app (otherwise it hides to the tray).
    pub close_quits: bool,
}

impl Settings {
    pub fn windowed(&self) -> bool {
        self.window_mode == "window"
    }
}

impl Default for Settings {
    fn default() -> Self {
        Self {
            word_size: 136,
            blur: 14,
            offset_ms: 0,
            monitor: None,
            mirror_all: false,
            auto_open: false,
            auto_close_minutes: 0,
            launch_at_login: false,
            hide_player_bar: false,
            lyrics_style: LYRICS_STYLES[0].into(),
            background_style: BACKGROUND_STYLES[0].into(),
            hd_artwork: true,
            window_mode: WINDOW_MODES[0].into(),
            close_quits: true,
        }
    }
}

/// Keeps `value` if it is one of `allowed`, otherwise resets it to the default.
fn one_of(value: &mut String, allowed: &[&str]) {
    if !allowed.contains(&value.as_str()) {
        *value = allowed[0].into();
    }
}

impl Settings {
    pub fn clamp(&mut self) {
        self.word_size = self.word_size.clamp(72, 200);
        self.blur = self.blur.min(60);
        self.offset_ms = self.offset_ms.clamp(-2000, 2000);
        self.auto_close_minutes = self.auto_close_minutes.min(24 * 60);
        if self
            .monitor
            .as_deref()
            .is_some_and(|m| m.trim().is_empty() || m.len() > 256)
        {
            self.monitor = None;
        }
        one_of(&mut self.lyrics_style, LYRICS_STYLES);
        one_of(&mut self.background_style, BACKGROUND_STYLES);
        one_of(&mut self.window_mode, WINDOW_MODES);
    }

    pub fn load(path: &Path) -> Self {
        let mut s = std::fs::read(path)
            .ok()
            .and_then(|b| serde_json::from_slice::<Settings>(&b).ok())
            .unwrap_or_default();
        s.clamp();
        s
    }

    pub fn save(&self, path: &Path) {
        if let Some(dir) = path.parent() {
            let _ = std::fs::create_dir_all(dir);
        }
        match serde_json::to_vec_pretty(self) {
            Ok(bytes) => {
                if let Err(e) = std::fs::write(path, bytes) {
                    log::warn!("saving settings failed: {e}");
                }
            }
            Err(e) => log::warn!("encoding settings failed: {e}"),
        }
    }

    /// Applies a partial update (`{"blur": 20}`) and clamps the result.
    pub fn merged(&self, patch: &Value) -> Result<Settings, String> {
        let mut base = serde_json::to_value(self).map_err(|e| e.to_string())?;
        let (Some(obj), Some(p)) = (base.as_object_mut(), patch.as_object()) else {
            return Err("settings patch must be an object".into());
        };
        for (k, v) in p {
            if obj.contains_key(k) {
                obj.insert(k.clone(), v.clone());
            }
        }
        let mut s: Settings = serde_json::from_value(base).map_err(|e| e.to_string())?;
        s.clamp();
        Ok(s)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn merge_and_clamp() {
        let s = Settings::default();
        let m = s
            .merged(&json!({"blur": 99, "offsetMs": -5000, "monitor": "X", "unknown": 1}))
            .unwrap();
        assert_eq!(m.blur, 60);
        assert_eq!(m.offset_ms, -2000);
        assert_eq!(m.monitor.as_deref(), Some("X"));
        let m = m.merged(&json!({"monitor": null})).unwrap();
        assert_eq!(m.monitor, None);
        assert!(s.merged(&json!({"blur": "lots"})).is_err());
    }

    #[test]
    fn missing_fields_take_defaults() {
        let s: Settings = serde_json::from_str(r#"{"blur": 30}"#).unwrap();
        assert_eq!(s.blur, 30);
        assert_eq!(s.word_size, 136);
        assert_eq!(s.lyrics_style, "cinematic");
        assert_eq!(s.background_style, "artwork");
        assert!(s.hd_artwork);
    }

    #[test]
    fn styles_are_validated() {
        let s = Settings::default();
        let m = s
            .merged(&json!({"lyricsStyle": "karaoke", "backgroundStyle": "split"}))
            .unwrap();
        assert_eq!(
            (m.lyrics_style.as_str(), m.background_style.as_str()),
            ("karaoke", "split")
        );
        let m = m
            .merged(&json!({"lyricsStyle": "disco", "backgroundStyle": ""}))
            .unwrap();
        assert_eq!(
            (m.lyrics_style.as_str(), m.background_style.as_str()),
            ("cinematic", "artwork")
        );
        assert!(s.merged(&json!({"lyricsStyle": 3})).is_err());
    }

    #[test]
    fn window_mode_is_validated() {
        let s = Settings::default();
        assert!(!s.windowed() && s.close_quits);
        let m = s.merged(&json!({"windowMode": "window"})).unwrap();
        assert!(m.windowed());
        let m = m.merged(&json!({"windowMode": "kiosk"})).unwrap();
        assert_eq!(m.window_mode, "fullscreen");
    }
}
