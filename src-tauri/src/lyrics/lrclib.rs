//! LRCLIB (https://lrclib.net): free, no API key.

use super::normalize::{key, similarity};
use serde::{Deserialize, Serialize};
use std::time::Duration;

const BASE: &str = "https://lrclib.net/api";
/// Sent in the User-Agent as LRCLIB asks. Point this at your fork/repo.
pub const PROJECT_URL: &str = "https://github.com/your-name/halo-lyrics";
/// LRCLIB answers are a few KB; refuse anything absurd instead of buffering it.
const MAX_RESPONSE_BYTES: usize = 4 * 1024 * 1024;
/// Search results scored per query (scoring is not free).
const MAX_CANDIDATES: usize = 50;

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Record {
    #[serde(default)]
    pub id: Option<i64>,
    #[serde(default)]
    pub track_name: Option<String>,
    #[serde(default)]
    pub artist_name: Option<String>,
    #[serde(default)]
    pub album_name: Option<String>,
    /// seconds
    #[serde(default)]
    pub duration: Option<f64>,
    #[serde(default)]
    pub instrumental: bool,
    #[serde(default)]
    pub plain_lyrics: Option<String>,
    #[serde(default)]
    pub synced_lyrics: Option<String>,
}

impl Record {
    pub fn has_synced(&self) -> bool {
        self.synced_lyrics.as_deref().is_some_and(|s| !s.trim().is_empty())
    }
    pub fn has_plain(&self) -> bool {
        self.plain_lyrics.as_deref().is_some_and(|s| !s.trim().is_empty())
    }
}

pub struct Query<'a> {
    pub title: &'a str,
    pub artist: &'a str,
    pub album: Option<&'a str>,
    pub duration_ms: u64,
}

#[derive(Debug)]
pub enum FetchError {
    /// Network trouble / server error: don't cache, try again later.
    Transient(String),
}

impl std::fmt::Display for FetchError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            FetchError::Transient(msg) => write!(f, "{msg}"),
        }
    }
}

pub fn client() -> reqwest::Client {
    let ua = format!("HaloLyrics/{} ({PROJECT_URL})", env!("CARGO_PKG_VERSION"));
    reqwest::Client::builder()
        .user_agent(ua)
        .timeout(Duration::from_secs(12))
        .connect_timeout(Duration::from_secs(6))
        .build()
        .expect("http client")
}

/// `/api/get` (exact match, ±2 s duration tolerance on the server) then
/// `/api/search` with our own scoring. `Ok(None)` = definitely not found.
pub async fn find(client: &reqwest::Client, q: &Query<'_>) -> Result<Option<Record>, FetchError> {
    if q.duration_ms > 0 && !q.artist.is_empty() {
        let mut params: Vec<(&str, String)> = vec![
            ("track_name", q.title.to_string()),
            ("artist_name", q.artist.to_string()),
            ("duration", ((q.duration_ms as f64) / 1000.0).round().to_string()),
        ];
        if let Some(album) = q.album.filter(|a| !a.trim().is_empty()) {
            params.push(("album_name", album.to_string()));
        }
        match get_json::<Record>(client, &format!("{BASE}/get"), &params).await? {
            Some(rec) if rec.instrumental || rec.has_synced() || rec.has_plain() => return Ok(Some(rec)),
            _ => {}
        }
    }

    let mut candidates: Vec<Record> = Vec::new();
    let mut params = vec![("track_name", q.title.to_string())];
    if !q.artist.is_empty() {
        params.push(("artist_name", q.artist.to_string()));
    }
    if let Some(list) = get_json::<Vec<Record>>(client, &format!("{BASE}/search"), &params).await? {
        candidates = list;
    }
    if candidates.is_empty() {
        // Free-text search copes better with odd artist strings.
        let q_text = format!("{} {}", q.title, q.artist);
        if let Some(list) = get_json::<Vec<Record>>(client, &format!("{BASE}/search"), &[("q", q_text.trim().to_string())]).await? {
            candidates = list;
        }
    }
    Ok(pick_best(q, candidates))
}

async fn get_json<T: for<'de> Deserialize<'de>>(
    client: &reqwest::Client,
    url: &str,
    params: &[(&str, String)],
) -> Result<Option<T>, FetchError> {
    let mut resp = client
        .get(url)
        .query(params)
        .send()
        .await
        .map_err(|e| FetchError::Transient(e.to_string()))?;
    let status = resp.status();
    if status == reqwest::StatusCode::NOT_FOUND {
        return Ok(None);
    }
    if !status.is_success() {
        return Err(FetchError::Transient(format!("HTTP {status} from {url}")));
    }
    if resp.content_length().is_some_and(|n| n > MAX_RESPONSE_BYTES as u64) {
        return Err(FetchError::Transient(format!("response from {url} too large")));
    }
    let mut body: Vec<u8> = Vec::new();
    while let Some(chunk) = resp.chunk().await.map_err(|e| FetchError::Transient(e.to_string()))? {
        if body.len() + chunk.len() > MAX_RESPONSE_BYTES {
            return Err(FetchError::Transient(format!("response from {url} too large")));
        }
        body.extend_from_slice(&chunk);
    }
    serde_json::from_slice::<T>(&body).map(Some).map_err(|e| FetchError::Transient(e.to_string()))
}

/// Best candidate by: has synced lyrics, duration within ±3 s, title/artist similarity.
pub fn pick_best(q: &Query<'_>, candidates: Vec<Record>) -> Option<Record> {
    let qt = key(q.title);
    let qa = key(q.artist);
    let mut best: Option<(f64, Record)> = None;
    for rec in candidates.into_iter().take(MAX_CANDIDATES) {
        if !(rec.instrumental || rec.has_synced() || rec.has_plain()) {
            continue;
        }
        let ts = similarity(&qt, &key(rec.track_name.as_deref().unwrap_or("")));
        let as_ = if qa.is_empty() {
            0.5
        } else {
            similarity(&qa, &key(rec.artist_name.as_deref().unwrap_or("")))
        };
        if ts < 0.55 || as_ < 0.4 {
            continue;
        }
        let mut score = ts * 2.0 + as_ * 1.5;
        if rec.has_synced() {
            score += 2.0;
        }
        if q.duration_ms > 0 {
            if let Some(d) = rec.duration {
                let diff = ((d * 1000.0) - q.duration_ms as f64).abs();
                if diff <= 3000.0 {
                    score += 1.5;
                } else if diff > 20_000.0 {
                    // Almost certainly a different version (live, extended...).
                    continue;
                } else {
                    score -= diff / 20_000.0;
                }
            }
        }
        if best.as_ref().is_none_or(|(s, _)| score > *s) {
            best = Some((score, rec));
        }
    }
    best.map(|(_, r)| r)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn rec(track: &str, artist: &str, dur: f64, synced: bool) -> Record {
        Record {
            id: None,
            track_name: Some(track.into()),
            artist_name: Some(artist.into()),
            album_name: None,
            duration: Some(dur),
            instrumental: false,
            plain_lyrics: Some("la la".into()),
            synced_lyrics: synced.then(|| "[00:01.00]la".to_string()),
        }
    }

    #[test]
    fn prefers_synced_and_matching_duration() {
        let q = Query { title: "Song", artist: "Band", album: None, duration_ms: 200_000 };
        let picked = pick_best(
            &q,
            vec![
                rec("Song", "Band", 200.0, false),
                rec("Song", "Band", 201.0, true),
                rec("Song", "Band", 260.0, true), // different version
            ],
        )
        .unwrap();
        assert!(picked.has_synced());
        assert_eq!(picked.duration, Some(201.0));
    }

    #[test]
    fn rejects_unrelated_results() {
        let q = Query { title: "Song", artist: "Band", album: None, duration_ms: 0 };
        assert!(pick_best(&q, vec![rec("Completely Different", "Other", 200.0, true)]).is_none());
    }

    #[test]
    fn parses_api_shape() {
        let json = r#"{"id":1,"trackName":"T","artistName":"A","albumName":"B","duration":233.0,"instrumental":false,"plainLyrics":"x","syncedLyrics":"[00:01.00]x"}"#;
        let r: Record = serde_json::from_str(json).unwrap();
        assert!(r.has_synced());
        let json = r#"{"id":2,"trackName":"T","artistName":"A","instrumental":true,"plainLyrics":null,"syncedLyrics":null}"#;
        let r: Record = serde_json::from_str(json).unwrap();
        assert!(r.instrumental && !r.has_synced() && !r.has_plain());
    }
}
