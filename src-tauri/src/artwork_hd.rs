//! High-resolution artwork from public music catalogues, for when the player
//! only gives the OS a small thumbnail (browsers usually do).
//!
//! * iTunes Search API (free, no key): covers up to 2000 × 2000.
//! * Deezer API (free, no key): 1000 × 1000 covers; a wider catalogue for some
//!   regions, so it is the fallback.
//!
//! Only the song title and artist are sent. Results must match both, using the
//! same similarity scoring as the lyrics search, so a wrong cover is unlikely;
//! when nothing matches, the player's own artwork is kept.

use crate::lyrics::normalize::{key, similarity};
use serde::Deserialize;

/// Player artwork at least this big is used as it is.
pub const GOOD_ENOUGH_SIDE: u32 = 1000;
/// Cover size asked from iTunes (it scales its originals on request).
const ITUNES_SIDE: u32 = 2000;
const MAX_JSON_BYTES: usize = 2 * 1024 * 1024;
const MAX_IMAGE_BYTES: usize = 12 * 1024 * 1024;

/// One catalogue hit, reduced to what the scoring needs.
#[derive(Debug, Clone)]
struct Hit {
    title: String,
    artist: String,
    duration_ms: Option<u64>,
    image_url: String,
}

/// Looks the song up and downloads its cover. `variants` are (title, artist)
/// pairs to try, best first (see `lyrics::query_variants`).
pub async fn find(
    client: &reqwest::Client,
    variants: &[(String, String)],
    duration_ms: u64,
) -> Option<Vec<u8>> {
    for (title, artist) in variants {
        if title.trim().is_empty() {
            continue;
        }
        for source in [Source::Itunes, Source::Deezer] {
            let hits = match source.search(client, title, artist).await {
                Ok(h) => h,
                Err(e) => {
                    log::warn!("artwork: {} search failed: {e}", source.name());
                    continue;
                }
            };
            let Some(hit) = pick_best(title, artist, duration_ms, hits) else {
                continue;
            };
            if !image_host_allowed(&hit.image_url) {
                log::warn!("artwork: ignoring a cover from an unexpected address");
                continue;
            }
            match download(client, &hit.image_url, MAX_IMAGE_BYTES).await {
                Ok(bytes) => {
                    log::info!(
                        "artwork: high-res cover from {} for '{}' / '{}'",
                        source.name(),
                        hit.title,
                        hit.artist
                    );
                    return Some(bytes);
                }
                Err(e) => log::warn!("artwork: {} image download failed: {e}", source.name()),
            }
        }
    }
    log::info!("artwork: no high-res cover found");
    None
}

#[derive(Clone, Copy)]
enum Source {
    Itunes,
    Deezer,
}

impl Source {
    fn name(self) -> &'static str {
        match self {
            Source::Itunes => "iTunes",
            Source::Deezer => "Deezer",
        }
    }

    async fn search(
        self,
        client: &reqwest::Client,
        title: &str,
        artist: &str,
    ) -> Result<Vec<Hit>, String> {
        let term = format!("{artist} {title}");
        match self {
            Source::Itunes => {
                let url = reqwest::Url::parse_with_params(
                    "https://itunes.apple.com/search",
                    &[
                        ("term", term.trim()),
                        ("media", "music"),
                        ("entity", "song"),
                        ("limit", "15"),
                    ],
                )
                .map_err(|e| e.to_string())?;
                let body: ItunesResponse = get_json(client, url).await?;
                Ok(body
                    .results
                    .into_iter()
                    .filter_map(ItunesItem::into_hit)
                    .collect())
            }
            Source::Deezer => {
                let url = reqwest::Url::parse_with_params(
                    "https://api.deezer.com/search",
                    &[("q", term.trim()), ("limit", "15")],
                )
                .map_err(|e| e.to_string())?;
                let body: DeezerResponse = get_json(client, url).await?;
                Ok(body
                    .data
                    .into_iter()
                    .filter_map(DeezerItem::into_hit)
                    .collect())
            }
        }
    }
}

#[derive(Deserialize)]
struct ItunesResponse {
    #[serde(default)]
    results: Vec<ItunesItem>,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct ItunesItem {
    track_name: Option<String>,
    artist_name: Option<String>,
    track_time_millis: Option<u64>,
    artwork_url100: Option<String>,
}

impl ItunesItem {
    fn into_hit(self) -> Option<Hit> {
        let small = self.artwork_url100?;
        // ".../100x100bb.jpg" → ".../2000x2000bb.jpg"
        let image_url = small.replace("100x100bb", &format!("{ITUNES_SIDE}x{ITUNES_SIDE}bb"));
        Some(Hit {
            title: self.track_name?,
            artist: self.artist_name.unwrap_or_default(),
            duration_ms: self.track_time_millis,
            image_url,
        })
    }
}

#[derive(Deserialize)]
struct DeezerResponse {
    #[serde(default)]
    data: Vec<DeezerItem>,
}

#[derive(Deserialize)]
struct DeezerItem {
    title: Option<String>,
    /// seconds
    duration: Option<u64>,
    artist: Option<DeezerArtist>,
    album: Option<DeezerAlbum>,
}

#[derive(Deserialize)]
struct DeezerArtist {
    name: Option<String>,
}

#[derive(Deserialize)]
struct DeezerAlbum {
    cover_xl: Option<String>,
}

impl DeezerItem {
    fn into_hit(self) -> Option<Hit> {
        Some(Hit {
            title: self.title?,
            artist: self.artist.and_then(|a| a.name).unwrap_or_default(),
            duration_ms: self.duration.map(|s| s * 1000),
            image_url: self.album?.cover_xl.filter(|u| !u.is_empty())?,
        })
    }
}

/// Best hit whose title and artist both match well enough; duration breaks ties
/// and rules out clearly different recordings.
fn pick_best(title: &str, artist: &str, duration_ms: u64, hits: Vec<Hit>) -> Option<Hit> {
    let qt = key(title);
    let qa = key(artist);
    let mut best: Option<(f64, Hit)> = None;
    for hit in hits {
        let ts = similarity(&qt, &key(&hit.title));
        let as_ = if qa.is_empty() {
            0.5
        } else {
            similarity(&qa, &key(&hit.artist))
        };
        if ts < 0.6 || as_ < 0.5 {
            continue;
        }
        let mut score = ts * 2.0 + as_ * 1.5;
        if let (true, Some(d)) = (duration_ms > 0, hit.duration_ms) {
            let diff = (d as f64 - duration_ms as f64).abs();
            if diff <= 4000.0 {
                score += 1.0;
            } else if diff > 45_000.0 {
                continue;
            }
        }
        if best.as_ref().is_none_or(|(s, _)| score > *s) {
            best = Some((score, hit));
        }
    }
    best.map(|(_, h)| h)
}

async fn get_json<T: for<'de> Deserialize<'de>>(
    client: &reqwest::Client,
    url: reqwest::Url,
) -> Result<T, String> {
    let bytes = download(client, url.as_str(), MAX_JSON_BYTES).await?;
    serde_json::from_slice(&bytes).map_err(|e| e.to_string())
}

/// Cover images are only fetched over HTTPS from Apple's and Deezer's image
/// servers, whatever the search response says.
fn image_host_allowed(url: &str) -> bool {
    let Ok(u) = reqwest::Url::parse(url) else {
        return false;
    };
    u.scheme() == "https"
        && u.host_str().is_some_and(|h| {
            [".mzstatic.com", ".dzcdn.net"].iter().any(|d| h.ends_with(d))
        })
}

async fn download(client: &reqwest::Client, url: &str, limit: usize) -> Result<Vec<u8>, String> {
    if !url.starts_with("https://") {
        return Err("refusing a non-https URL".into());
    }
    let mut resp = client.get(url).send().await.map_err(|e| e.to_string())?;
    let status = resp.status();
    if !status.is_success() {
        return Err(format!("HTTP {status}"));
    }
    if resp.content_length().is_some_and(|n| n > limit as u64) {
        return Err("response too large".into());
    }
    let mut body = Vec::new();
    while let Some(chunk) = resp.chunk().await.map_err(|e| e.to_string())? {
        if body.len() + chunk.len() > limit {
            return Err("response too large".into());
        }
        body.extend_from_slice(&chunk);
    }
    Ok(body)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn hit(title: &str, artist: &str, secs: u64) -> Hit {
        Hit {
            title: title.into(),
            artist: artist.into(),
            duration_ms: Some(secs * 1000),
            image_url: format!("https://x/{title}.jpg"),
        }
    }

    #[test]
    fn picks_the_matching_song() {
        let hits = vec![
            hit("Other Song", "Band", 200),
            hit("Young and Beautiful", "Lana Del Rey", 236),
            hit("Young and Beautiful (Live)", "Lana Del Rey", 300),
        ];
        let best = pick_best("Young And Beautiful", "Lana Del Rey", 236_000, hits).unwrap();
        assert_eq!(best.image_url, "https://x/Young and Beautiful.jpg");
    }

    #[test]
    fn rejects_wrong_artist_or_title() {
        assert!(pick_best(
            "Kalyani",
            "ARJN",
            180_000,
            vec![hit("Kalyani", "Someone Else Entirely", 180)]
        )
        .is_none());
        assert!(pick_best(
            "Kalyani",
            "ARJN",
            180_000,
            vec![hit("Totally Different", "ARJN", 180)]
        )
        .is_none());
    }

    #[test]
    fn rejects_a_very_different_length() {
        assert!(pick_best("Song", "Band", 200_000, vec![hit("Song", "Band", 400)]).is_none());
    }

    #[test]
    fn itunes_url_is_upsized() {
        let item = ItunesItem {
            track_name: Some("Song".into()),
            artist_name: Some("Band".into()),
            track_time_millis: Some(1000),
            artwork_url100: Some(
                "https://is1-ssl.mzstatic.com/image/thumb/a/b/100x100bb.jpg".into(),
            ),
        };
        assert!(item
            .into_hit()
            .unwrap()
            .image_url
            .ends_with("/2000x2000bb.jpg"));
    }

    #[test]
    fn only_known_image_hosts() {
        assert!(image_host_allowed("https://is1-ssl.mzstatic.com/image/thumb/x/2000x2000bb.jpg"));
        assert!(image_host_allowed("https://e-cdns-images.dzcdn.net/images/cover/x/1000x1000.jpg"));
        assert!(!image_host_allowed("http://is1-ssl.mzstatic.com/x.jpg"));
        assert!(!image_host_allowed("https://evil.example/x.jpg"));
        assert!(!image_host_allowed("https://mzstatic.com.evil.example/x.jpg"));
        assert!(!image_host_allowed("file:///C:/x.jpg"));
    }
}
