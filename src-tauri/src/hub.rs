//! Turns raw OS snapshots into `now-playing` events, kicks off lyrics lookups,
//! and drives auto-open / auto-close / keep-awake.
//!
//! * Change events are emitted immediately; otherwise a heartbeat at most 1 Hz.
//! * Artwork is processed once per distinct image and only sent to the
//!   frontend when it changes (`artworkKey` identifies it on heartbeats).

use crate::artwork::{self, Artwork};
use crate::lyrics::{fnv1a_hex, LyricsEvent, TrackInfo};
use crate::media::{now_ms, RawNowPlaying};
use crate::{ui, AppState};
use serde::Serialize;
use std::sync::mpsc::{Receiver, RecvTimeoutError};
use std::sync::Arc;
use std::time::{Duration, Instant};
use tauri::{AppHandle, Emitter, Manager};

const HEARTBEAT: Duration = Duration::from_secs(1);
const SEEK_THRESHOLD_MS: i64 = 400;
/// "Music starts" = playing after at least this long of not playing.
const RESTART_GAP: Duration = Duration::from_secs(5);

#[derive(Serialize, Clone, Debug)]
#[serde(rename_all = "camelCase")]
pub struct NowPlaying {
    pub source_app: String,
    pub title: String,
    pub artist: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub album: Option<String>,
    pub duration_ms: u64,
    pub position_ms: u64,
    pub position_at: u64,
    pub playing: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub artwork: Option<Artwork>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub artwork_key: Option<String>,
    pub track_key: String,
}

impl NowPlaying {
    fn position_now(&self, now: u64) -> i64 {
        let mut p = self.position_ms as i64;
        if self.playing {
            p += now as i64 - self.position_at as i64;
        }
        p
    }
}

pub fn track_key(title: &str, artist: &str, album: Option<&str>, duration_ms: u64) -> String {
    let secs = (duration_ms / 1000).to_string();
    fnv1a_hex(&[
        title.trim().to_lowercase().as_bytes(),
        artist.trim().to_lowercase().as_bytes(),
        album.unwrap_or("").trim().to_lowercase().as_bytes(),
        secs.as_bytes(),
    ])
}

#[derive(Default)]
struct Hub {
    raw: Option<RawNowPlaying>,
    // artwork cache
    art_arc: Option<Arc<Vec<u8>>>,
    art_key: Option<String>,
    art: Option<Artwork>,
    // emission
    last: Option<NowPlaying>,
    last_emit: Option<Instant>,
    last_emitted_art_key: Option<String>,
    // behaviour
    was_playing: bool,
    not_playing_since: Option<Instant>,
    auto_closed: bool,
}

pub fn spawn(app: AppHandle, rx: Receiver<Option<RawNowPlaying>>) {
    std::thread::Builder::new()
        .name("hub".into())
        .spawn(move || {
            let mut hub = Hub::default();
            loop {
                let incoming = match rx.recv_timeout(HEARTBEAT) {
                    Ok(s) => Some(s),
                    Err(RecvTimeoutError::Timeout) => None,
                    Err(RecvTimeoutError::Disconnected) => return,
                };
                // Only the newest snapshot matters.
                let mut latest = incoming;
                while let Ok(s) = rx.try_recv() {
                    latest = Some(s);
                }
                hub.tick(&app, latest);
            }
        })
        .expect("spawn hub thread");
}

impl Hub {
    fn tick(&mut self, app: &AppHandle, incoming: Option<Option<RawNowPlaying>>) {
        let state = app.state::<AppState>();
        let paused = state.detection_paused.load(std::sync::atomic::Ordering::Relaxed);
        if let Some(raw) = incoming {
            self.raw = raw;
        }
        if paused {
            self.raw = None;
        }

        let np = self.raw.clone().map(|r| self.to_now_playing(r));
        let now = now_ms();
        let changed = match (&self.last, &np) {
            (None, None) => false,
            (Some(_), None) | (None, Some(_)) => true,
            (Some(a), Some(b)) => {
                a.track_key != b.track_key
                    || a.playing != b.playing
                    || a.artwork_key != b.artwork_key
                    || a.title != b.title
                    || (a.position_now(now) - b.position_now(now)).abs() > SEEK_THRESHOLD_MS
            }
        };
        let track_changed = match (&self.last, &np) {
            (Some(a), Some(b)) => a.track_key != b.track_key,
            (None, Some(_)) => true,
            _ => false,
        };
        let due = self.last_emit.is_none_or(|t| t.elapsed() >= HEARTBEAT);

        if changed || due {
            let mut payload = np.clone();
            if let Some(p) = payload.as_mut() {
                // Only send the (large) artwork when it differs from what the
                // windows already have; they cache it by `artworkKey`. Windows
                // that open later get it from `get_state`.
                if p.artwork_key.is_some() && p.artwork_key == self.last_emitted_art_key {
                    p.artwork = None;
                }
                self.last_emitted_art_key = p.artwork_key.clone();
            }
            if let Err(e) = app.emit("now-playing", &payload) {
                log::warn!("emit now-playing failed: {e}");
            }
            self.last_emit = Some(Instant::now());
        }

        *state.now_playing.lock().unwrap() = np.clone();

        if track_changed {
            if let Some(n) = &np {
                log::info!("now playing: '{}' by '{}' via {} [{}]", n.title, n.artist, n.source_app, n.track_key);
                *state.lyrics.lock().unwrap() = None;
                fetch_lyrics(app, n);
            }
        }

        self.behaviour(app, np.as_ref());
        self.last = np;
    }

    fn to_now_playing(&mut self, raw: RawNowPlaying) -> NowPlaying {
        // Artwork: re-process only when the bytes actually change.
        match &raw.artwork {
            None => {
                self.art_arc = None;
                self.art_key = None;
                self.art = None;
            }
            Some(bytes) => {
                let same_arc = self.art_arc.as_ref().is_some_and(|a| Arc::ptr_eq(a, bytes));
                if !same_arc {
                    let key = fnv1a_hex(&[bytes.as_slice()]);
                    if self.art_key.as_deref() != Some(key.as_str()) {
                        let started = Instant::now();
                        self.art = artwork::process(bytes);
                        log::debug!("artwork processed in {:?}", started.elapsed());
                        self.art_key = self.art.as_ref().map(|_| key);
                    }
                    self.art_arc = Some(bytes.clone());
                }
            }
        }
        NowPlaying {
            track_key: track_key(&raw.title, &raw.artist, raw.album.as_deref(), raw.duration_ms),
            source_app: raw.source_app,
            title: raw.title,
            artist: raw.artist,
            album: raw.album,
            duration_ms: raw.duration_ms,
            position_ms: raw.position_ms,
            position_at: raw.position_at,
            playing: raw.playing,
            artwork: self.art.clone(),
            artwork_key: self.art_key.clone(),
        }
    }

    fn behaviour(&mut self, app: &AppHandle, np: Option<&NowPlaying>) {
        let state = app.state::<AppState>();
        let settings = state.settings.lock().unwrap().clone();
        let playing = np.is_some_and(|n| n.playing);
        let visible = ui::lyrics_visible(app);

        if playing {
            let restarted = !self.was_playing && self.not_playing_since.is_none_or(|t| t.elapsed() >= RESTART_GAP);
            if restarted && settings.auto_open && !visible {
                log::info!("music started: opening lyrics mode");
                ui::show_lyrics(app);
            }
            self.not_playing_since = None;
            self.auto_closed = false;
        } else if self.not_playing_since.is_none() {
            self.not_playing_since = Some(Instant::now());
        }

        if !playing && settings.auto_close_minutes > 0 && visible && !self.auto_closed {
            let limit = Duration::from_secs(settings.auto_close_minutes as u64 * 60);
            if self.not_playing_since.is_some_and(|t| t.elapsed() >= limit) {
                log::info!("music stopped for {} min: closing lyrics mode", settings.auto_close_minutes);
                self.auto_closed = true;
                ui::hide_lyrics(app);
            }
        }

        self.was_playing = playing;
        state.power.set(playing && ui::lyrics_visible(app));
    }
}

fn fetch_lyrics(app: &AppHandle, n: &NowPlaying) {
    let info = TrackInfo {
        track_key: n.track_key.clone(),
        title: n.title.clone(),
        artist: n.artist.clone(),
        album: n.album.clone(),
        duration_ms: n.duration_ms,
        source_app: n.source_app.clone(),
    };
    let app = app.clone();
    std::thread::spawn(move || {
        // Players often report duration 0 for a moment before the real value
        // (which changes the track key). Give it a beat instead of looking up twice.
        if info.duration_ms == 0 {
            std::thread::sleep(Duration::from_millis(1500));
            let state = app.state::<AppState>();
            let current = state.now_playing.lock().unwrap().as_ref().map(|n| n.track_key.clone());
            if current.as_deref() != Some(info.track_key.as_str()) {
                return;
            }
        }
        let service = app.state::<AppState>().lyrics_service.clone();
        let lyrics = tauri::async_runtime::block_on(service.resolve(&info));
        let event = LyricsEvent { track_key: info.track_key.clone(), lyrics };
        let state = app.state::<AppState>();
        let current = state.now_playing.lock().unwrap().as_ref().map(|n| n.track_key.clone());
        if current.as_deref() == Some(info.track_key.as_str()) {
            *state.lyrics.lock().unwrap() = Some(event.clone());
        }
        if let Err(e) = app.emit("lyrics", &event) {
            log::warn!("emit lyrics failed: {e}");
        }
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn track_keys_are_stable_and_case_insensitive() {
        let a = track_key("Song", "Band", Some("Album"), 200_400);
        let b = track_key("song ", "BAND", Some("album"), 200_900);
        assert_eq!(a, b);
        assert_ne!(a, track_key("Song", "Band", Some("Album"), 210_000));
        assert_eq!(a.len(), 16);
    }
}
