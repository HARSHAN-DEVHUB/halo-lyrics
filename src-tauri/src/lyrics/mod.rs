//! Lyrics: user override folder → local cache → LRCLIB, then parse.

pub mod lrc;
pub mod lrclib;
pub mod normalize;

use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};

use crate::media::{is_browser, now_ms};

/// Cached misses expire after this long; hits are kept until the cache is cleared.
const MISS_TTL_MS: u64 = 24 * 60 * 60 * 1000;
const CACHE_VERSION: u32 = 1;
/// A real .lrc is a few KB; don't load something huge by mistake.
const MAX_OVERRIDE_BYTES: u64 = 1024 * 1024;

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct Word {
    pub start_ms: u64,
    pub end_ms: u64,
    pub text: String,
}

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct Line {
    pub start_ms: u64,
    pub end_ms: u64,
    pub text: String,
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub words: Option<Vec<Word>>,
}

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
#[serde(tag = "kind", rename_all = "lowercase")]
pub enum Lyrics {
    Synced {
        lines: Vec<Line>,
    },
    Plain {
        text: String,
    },
    Instrumental,
    #[serde(rename = "none")]
    NotFound,
}

#[derive(Serialize, Clone, Debug)]
#[serde(rename_all = "camelCase")]
pub struct LyricsEvent {
    pub track_key: String,
    pub lyrics: Lyrics,
}

#[derive(Clone, Debug)]
pub struct TrackInfo {
    pub track_key: String,
    pub title: String,
    pub artist: String,
    pub album: Option<String>,
    pub duration_ms: u64,
    pub source_app: String,
}

#[derive(Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct CacheEntry {
    version: u32,
    fetched_at: u64,
    miss: bool,
    /// The raw LRCLIB record, kept so we can re-parse after parser changes.
    raw: Option<lrclib::Record>,
    lyrics: Lyrics,
}

pub struct LyricsService {
    client: reqwest::Client,
    cache_dir: PathBuf,
    override_dir: Option<PathBuf>,
}

impl LyricsService {
    pub fn new(cache_dir: PathBuf, override_dir: Option<PathBuf>) -> Self {
        let _ = std::fs::create_dir_all(&cache_dir);
        Self {
            client: lrclib::client(),
            cache_dir,
            override_dir,
        }
    }

    /// Shared HTTP client (also used for the high-res artwork lookup).
    pub fn client(&self) -> &reqwest::Client {
        &self.client
    }

    pub fn override_dir(&self) -> Option<&Path> {
        self.override_dir.as_deref()
    }

    pub async fn resolve(&self, t: &TrackInfo) -> Lyrics {
        if let Some(l) = self.from_override(t) {
            log::info!("lyrics: using override file for {}", t.track_key);
            return l;
        }
        if let Some(l) = self.from_cache(t) {
            return l;
        }
        match self.fetch(t).await {
            Ok((raw, lyrics)) => {
                let miss = raw.is_none();
                self.write_cache(
                    &t.track_key,
                    CacheEntry {
                        version: CACHE_VERSION,
                        fetched_at: now_ms(),
                        miss,
                        raw,
                        lyrics: lyrics.clone(),
                    },
                );
                lyrics
            }
            Err(e) => {
                // Don't cache network trouble; we'll try again next time.
                log::warn!("lyrics fetch failed for {}: {e}", t.track_key);
                Lyrics::NotFound
            }
        }
    }

    async fn fetch(
        &self,
        t: &TrackInfo,
    ) -> Result<(Option<lrclib::Record>, Lyrics), lrclib::FetchError> {
        for (title, artist) in query_variants(t) {
            let q = lrclib::Query {
                title: &title,
                artist: &artist,
                album: t.album.as_deref(),
                duration_ms: t.duration_ms,
            };
            if let Some(rec) = lrclib::find(&self.client, &q).await? {
                log::info!(
                    "lyrics: found LRCLIB #{:?} for '{title}' / '{artist}' (synced: {})",
                    rec.id,
                    rec.has_synced()
                );
                let lyrics = record_to_lyrics(&rec, t.duration_ms);
                return Ok((Some(rec), lyrics));
            }
        }
        log::info!("lyrics: nothing found for {}", t.track_key);
        Ok((None, Lyrics::NotFound))
    }

    fn cache_path(&self, track_key: &str) -> PathBuf {
        let safe: String = track_key
            .chars()
            .filter(|c| c.is_ascii_alphanumeric())
            .collect();
        self.cache_dir.join(format!("{safe}.json"))
    }

    fn from_cache(&self, t: &TrackInfo) -> Option<Lyrics> {
        let bytes = std::fs::read(self.cache_path(&t.track_key)).ok()?;
        let entry: CacheEntry = serde_json::from_slice(&bytes).ok()?;
        if entry.version != CACHE_VERSION {
            return None;
        }
        if entry.miss {
            if now_ms().saturating_sub(entry.fetched_at) > MISS_TTL_MS {
                return None;
            }
            return Some(Lyrics::NotFound);
        }
        // Re-parse from the raw record so parser fixes apply to cached songs too.
        Some(match &entry.raw {
            Some(raw) => record_to_lyrics(raw, t.duration_ms),
            None => entry.lyrics,
        })
    }

    fn write_cache(&self, track_key: &str, entry: CacheEntry) {
        let path = self.cache_path(track_key);
        let tmp = path.with_extension("json.tmp");
        match serde_json::to_vec(&entry) {
            Ok(bytes) => {
                if std::fs::write(&tmp, bytes).is_ok() {
                    let _ = std::fs::rename(&tmp, &path);
                }
            }
            Err(e) => log::warn!("cache encode failed: {e}"),
        }
    }

    pub fn clear_cache(&self) -> usize {
        let mut n = 0;
        if let Ok(rd) = std::fs::read_dir(&self.cache_dir) {
            for e in rd.flatten() {
                let p = e.path();
                if p.extension().is_some_and(|x| x == "json" || x == "tmp")
                    && std::fs::remove_file(&p).is_ok()
                {
                    n += 1;
                }
            }
        }
        log::info!("lyrics cache cleared ({n} files)");
        n
    }

    pub fn cache_count(&self) -> usize {
        std::fs::read_dir(&self.cache_dir)
            .map(|rd| {
                rd.flatten()
                    .filter(|e| e.path().extension().is_some_and(|x| x == "json"))
                    .count()
            })
            .unwrap_or(0)
    }

    /// `<Music>/Lyrics/<Artist> - <Title>.lrc`
    fn from_override(&self, t: &TrackInfo) -> Option<Lyrics> {
        let dir = self.override_dir.as_ref()?;
        if !dir.is_dir() {
            return None;
        }
        let mut names = Vec::new();
        let pairs = [
            (t.artist.clone(), t.title.clone()),
            (
                normalize::clean_artist(&t.artist),
                normalize::clean_title(&t.title),
            ),
        ];
        for (artist, title) in pairs {
            for repl in ["", "_"] {
                names.push(format!(
                    "{} - {}.lrc",
                    sanitize(&artist, repl),
                    sanitize(&title, repl)
                ));
            }
        }
        for name in names {
            let path = dir.join(&name);
            if std::fs::metadata(&path)
                .map_or(true, |m| !m.is_file() || m.len() > MAX_OVERRIDE_BYTES)
            {
                continue;
            }
            if let Ok(text) = std::fs::read_to_string(&path) {
                return Some(text_to_lyrics(&text, t.duration_ms));
            }
        }
        None
    }
}

/// Title/artist pairs to try, best first, de-duplicated.
pub fn query_variants(t: &TrackInfo) -> Vec<(String, String)> {
    // YouTube Music sometimes reports "Artist • Album • Year" as the artist.
    let artist_raw = t.artist.split(" • ").next().unwrap_or(&t.artist);
    let title = normalize::clean_title(&t.title);
    let artist = normalize::clean_artist(artist_raw);
    let mut out: Vec<(String, String)> = Vec::new();
    let mut push = |ti: String, ar: String| {
        if !ti.is_empty()
            && !out.iter().any(|(a, b)| {
                normalize::key(a) == normalize::key(&ti) && normalize::key(b) == normalize::key(&ar)
            })
        {
            out.push((ti, ar));
        }
    };
    // Browser video titles: "Artist - Title (Official Video)".
    if is_browser(&t.source_app)
        || t.artist.to_lowercase().ends_with("vevo")
        || t.artist.ends_with("- Topic")
    {
        if let Some((ti, ar)) = normalize::split_artist_title(&t.title, artist_raw) {
            push(normalize::clean_title(&ti), normalize::clean_artist(&ar));
        }
    }
    push(title.clone(), artist.clone());
    let primary = normalize::primary_artist(&artist);
    if primary != artist {
        push(title.clone(), primary);
    }
    out
}

pub fn record_to_lyrics(rec: &lrclib::Record, duration_ms: u64) -> Lyrics {
    if rec.instrumental {
        return Lyrics::Instrumental;
    }
    if let Some(synced) = rec
        .synced_lyrics
        .as_deref()
        .filter(|s| !s.trim().is_empty())
    {
        let lines = lrc::parse(synced, Some(duration_ms).filter(|d| *d > 0));
        if lines.iter().any(|l| l.text != lrc::BREATHER) {
            return Lyrics::Synced { lines };
        }
    }
    if let Some(plain) = rec.plain_lyrics.as_deref().filter(|s| !s.trim().is_empty()) {
        return Lyrics::Plain {
            text: plain.trim().to_string(),
        };
    }
    Lyrics::NotFound
}

/// A user's .lrc file: synced if it has timestamps, otherwise plain text.
pub fn text_to_lyrics(text: &str, duration_ms: u64) -> Lyrics {
    if lrc::has_timestamps(text) {
        let lines = lrc::parse(text, Some(duration_ms).filter(|d| *d > 0));
        if !lines.is_empty() {
            return Lyrics::Synced { lines };
        }
    }
    let plain = text.trim();
    if plain.is_empty() {
        Lyrics::NotFound
    } else {
        Lyrics::Plain {
            text: plain.to_string(),
        }
    }
}

fn sanitize(s: &str, repl: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for c in s.chars() {
        if matches!(c, '<' | '>' | ':' | '"' | '/' | '\\' | '|' | '?' | '*') || c.is_control() {
            out.push_str(repl);
        } else {
            out.push(c);
        }
    }
    out.trim().trim_end_matches('.').to_string()
}

/// Stable 64-bit FNV-1a, hex encoded. Used for track keys and artwork keys.
pub fn fnv1a_hex(parts: &[&[u8]]) -> String {
    let mut h: u64 = 0xcbf2_9ce4_8422_2325;
    for (i, p) in parts.iter().enumerate() {
        if i > 0 {
            h ^= 0x1f;
            h = h.wrapping_mul(0x0000_0100_0000_01b3);
        }
        for b in *p {
            h ^= *b as u64;
            h = h.wrapping_mul(0x0000_0100_0000_01b3);
        }
    }
    format!("{h:016x}")
}

#[cfg(test)]
mod tests {
    use super::*;

    fn track(title: &str, artist: &str, app: &str) -> TrackInfo {
        TrackInfo {
            track_key: "k".into(),
            title: title.into(),
            artist: artist.into(),
            album: None,
            duration_ms: 180_000,
            source_app: app.into(),
        }
    }

    #[test]
    fn variants_for_youtube_video() {
        let v = query_variants(&track(
            "Some Band - Great Song (Official Video)",
            "SomeBandVEVO",
            "Chrome",
        ));
        assert_eq!(v[0], ("Great Song".to_string(), "Some Band".to_string()));
    }

    #[test]
    fn variants_for_normal_player() {
        let v = query_variants(&track("Song - Remastered 2011", "A & B", "Spotify"));
        assert_eq!(v[0], ("Song".to_string(), "A & B".to_string()));
        assert_eq!(v[1], ("Song".to_string(), "A".to_string()));
        assert_eq!(v.len(), 2);
    }

    #[test]
    fn youtube_music_artist_bullets() {
        let v = query_variants(&track("Song", "Artist • Album • 2020", "Edge"));
        assert_eq!(v[0], ("Song".to_string(), "Artist".to_string()));
    }

    #[test]
    fn lyrics_serialize_with_kind_tag() {
        let j = serde_json::to_value(Lyrics::NotFound).unwrap();
        assert_eq!(j, serde_json::json!({"kind":"none"}));
        let j = serde_json::to_value(Lyrics::Instrumental).unwrap();
        assert_eq!(j, serde_json::json!({"kind":"instrumental"}));
        let j = serde_json::to_value(Lyrics::Synced {
            lines: vec![Line {
                start_ms: 1,
                end_ms: 2,
                text: "a".into(),
                words: None,
            }],
        })
        .unwrap();
        assert_eq!(
            j,
            serde_json::json!({"kind":"synced","lines":[{"startMs":1,"endMs":2,"text":"a"}]})
        );
    }

    #[test]
    fn override_text() {
        assert!(matches!(
            text_to_lyrics("[00:01.00]hi", 0),
            Lyrics::Synced { .. }
        ));
        assert!(matches!(
            text_to_lyrics("hi\nthere", 0),
            Lyrics::Plain { .. }
        ));
        assert!(matches!(text_to_lyrics("  ", 0), Lyrics::NotFound));
    }

    #[test]
    fn fnv_is_stable() {
        assert_eq!(fnv1a_hex(&[b"a"]), "af63dc4c8601ec8c");
        assert_ne!(fnv1a_hex(&[b"ab", b"c"]), fnv1a_hex(&[b"a", b"bc"]));
    }

    #[test]
    fn cache_roundtrip_and_miss_ttl() {
        let dir = std::env::temp_dir().join(format!("halo-cache-test-{}", std::process::id()));
        let svc = LyricsService::new(dir.clone(), None);
        let t = TrackInfo {
            track_key: "abc123".into(),
            ..track("T", "A", "Spotify")
        };
        assert!(svc.from_cache(&t).is_none());
        svc.write_cache(
            "abc123",
            CacheEntry {
                version: CACHE_VERSION,
                fetched_at: now_ms(),
                miss: true,
                raw: None,
                lyrics: Lyrics::NotFound,
            },
        );
        assert_eq!(svc.from_cache(&t), Some(Lyrics::NotFound));
        // expired miss
        svc.write_cache(
            "abc123",
            CacheEntry {
                version: CACHE_VERSION,
                fetched_at: now_ms() - MISS_TTL_MS - 1,
                miss: true,
                raw: None,
                lyrics: Lyrics::NotFound,
            },
        );
        assert!(svc.from_cache(&t).is_none());
        assert_eq!(svc.cache_count(), 1);
        assert_eq!(svc.clear_cache(), 1);
        let _ = std::fs::remove_dir_all(dir);
    }
}
