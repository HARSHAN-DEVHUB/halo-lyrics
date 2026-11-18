//! Cleaning titles/artists before the lyrics lookup.
//!
//! `clean_*` keep the original casing (they go into the query); `key` makes a
//! lowercase, punctuation-free form for comparing candidates.

/// Bracketed junk: removed when a `(...)` / `[...]` group starts with one of these.
const JUNK_GROUP_PREFIXES: &[&str] = &[
    "feat", "ft.", "ft ", "featuring", "with ", "prod", "official", "lyric", "lyrics", "audio", "video",
    "visualizer", "visualiser", "live", "remaster", "radio edit", "explicit", "clean", "hd", "hq", "4k",
    "mv", "m/v", "music video", "color coded", "colour coded", "mono", "stereo", "single version",
    "album version", "bonus track", "sped up", "slowed", "from ", "taken from", "full video", "full song",
    "video song", "full audio",
];

/// ` - suffix` junk: removed when the part after " - " starts with / contains these.
const JUNK_DASH_PREFIXES: &[&str] = &[
    "remaster", "radio edit", "live", "youtube music", "youtube", "single version", "album version",
    "mono", "stereo", "edit", "bonus track", "explicit", "clean", "official", "lyric", "audio", "video",
    "music video", "full video", "full song", "full audio", "video song", "4k", "hd",
];

/// Cleans a track title for lookup: drops "| credits | label" tails,
/// "(feat. …)", "(Official Video)", " - Music Video", " - Remastered 2011",
/// "(Live)", " - Radio Edit", " - YouTube Music" and leading/trailing punctuation.
pub fn clean_title(title: &str) -> String {
    let mut s = strip_pipe_credits(title);
    s = strip_groups(&s, JUNK_GROUP_PREFIXES);
    s = strip_dash_suffixes(&s);
    s = strip_inline_feat(&s);
    trim_punct(&s)
}

/// Cleans an artist: drops feat. clauses, " - Topic" (YouTube auto channels)
/// and a trailing "VEVO".
pub fn clean_artist(artist: &str) -> String {
    let mut s = strip_groups(artist, &["feat", "ft.", "ft ", "featuring"]);
    s = strip_inline_feat(&s);
    for suffix in [" - Topic", " – Topic", "- Topic"] {
        if let Some(stripped) = strip_suffix_ci(&s, suffix) {
            s = stripped.to_string();
        }
    }
    let t = s.trim_end();
    if t.len() > 4 {
        if let Some(stripped) = strip_suffix_ci(t, "vevo") {
            s = stripped.trim_end().to_string();
        }
    }
    trim_punct(&s)
}

/// First artist of "A, B & C" / "A x B" / "A; B".
pub fn primary_artist(artist: &str) -> String {
    // ASCII lowercasing keeps byte offsets identical, so indices found in
    // `lower` are valid in `artist` (full `to_lowercase` can change lengths).
    let lower = artist.to_ascii_lowercase();
    let mut cut = artist.len();
    for sep in [",", " & ", " x ", ";", " and ", " / ", " + "] {
        if let Some(i) = lower.find(sep) {
            cut = cut.min(i);
        }
    }
    trim_punct(&artist[..cut])
}

/// Video titles in browsers are often "Artist - Title". If the left side looks
/// like the reported artist (or the artist is a channel name), split it.
pub fn split_artist_title(title: &str, artist: &str) -> Option<(String, String)> {
    let (left, right) = title.split_once(" - ").or_else(|| title.split_once(" – "))?;
    let (left, right) = (left.trim(), right.trim());
    if left.is_empty() || right.is_empty() {
        return None;
    }
    let ka = key(&clean_artist(artist));
    let kl = key(left);
    let channel_like = artist.trim().is_empty()
        || artist.to_lowercase().ends_with("vevo")
        || artist.to_lowercase().ends_with("- topic")
        || artist.to_lowercase().contains("official");
    if channel_like || (!ka.is_empty() && (kl.contains(&ka) || ka.contains(&kl) || similarity(&kl, &ka) >= 0.7)) {
        Some((right.to_string(), left.to_string()))
    } else {
        None
    }
}

/// Lowercase, accents kept, punctuation removed, whitespace collapsed.
pub fn key(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    let mut space = false;
    for c in s.chars().flat_map(char::to_lowercase) {
        if c.is_alphanumeric() {
            if space && !out.is_empty() {
                out.push(' ');
            }
            space = false;
            out.push(c);
        } else if c == '&' {
            if !out.is_empty() {
                out.push(' ');
            }
            out.push_str("and");
            space = true;
        } else {
            space = true;
        }
    }
    out
}

/// 0..1 similarity of two already-`key`ed strings.
pub fn similarity(a: &str, b: &str) -> f64 {
    if a == b {
        return 1.0;
    }
    if a.is_empty() || b.is_empty() {
        return 0.0;
    }
    // Inputs come from the network; cap them so a huge title can't pin a CPU
    // core (Levenshtein is O(n·m)).
    let cap = |s: &str| -> String { s.chars().take(MAX_COMPARE_CHARS).collect() };
    let (a, b) = (cap(a), cap(b));
    let (a, b) = (a.as_str(), b.as_str());
    let (short, long) = if a.chars().count() <= b.chars().count() { (a, b) } else { (b, a) };
    // "song" vs "song live at x": containment is a strong signal.
    let containment: f64 = if short.chars().count() >= 4 && long.contains(short) { 0.9 } else { 0.0 };
    let dist = levenshtein(a, b) as f64;
    let max = a.chars().count().max(b.chars().count()) as f64;
    containment.max(1.0 - dist / max)
}

const MAX_COMPARE_CHARS: usize = 256;

fn levenshtein(a: &str, b: &str) -> usize {
    let a: Vec<char> = a.chars().collect();
    let b: Vec<char> = b.chars().collect();
    let mut prev: Vec<usize> = (0..=b.len()).collect();
    let mut cur = vec![0; b.len() + 1];
    for i in 1..=a.len() {
        cur[0] = i;
        for j in 1..=b.len() {
            let cost = usize::from(a[i - 1] != b[j - 1]);
            cur[j] = (prev[j] + 1).min(cur[j - 1] + 1).min(prev[j - 1] + cost);
        }
        std::mem::swap(&mut prev, &mut cur);
    }
    prev[b.len()]
}

/// Music-video titles (common on Indian label channels) list credits after
/// pipes: "Song - Music Video | Singer | Composer | Label". Keep the part
/// before the first pipe, unless that would leave nothing.
fn strip_pipe_credits(s: &str) -> String {
    match s.find(['|', '｜']) {
        Some(i) if !trim_punct(&s[..i]).is_empty() => s[..i].trim_end().to_string(),
        _ => s.to_string(),
    }
}

/// The credits after the first pipe of a video title ("Singer", "Composer", …),
/// which often name the real artist when the channel is a record label.
pub fn pipe_credits(title: &str) -> Vec<String> {
    title
        .split(['|', '｜'])
        .skip(1)
        .flat_map(|part| part.split(','))
        .map(trim_punct)
        .filter(|p| !p.is_empty() && p.chars().count() <= 40)
        .collect()
}

fn strip_groups(s: &str, prefixes: &[&str]) -> String {
    let mut out = String::with_capacity(s.len());
    let chars: Vec<char> = s.chars().collect();
    let mut i = 0;
    while i < chars.len() {
        let c = chars[i];
        let close = match c {
            '(' => Some(')'),
            '[' => Some(']'),
            '{' => Some('}'),
            _ => None,
        };
        if let Some(close) = close {
            if let Some(rel) = chars[i + 1..].iter().position(|&x| x == close) {
                let inner: String = chars[i + 1..i + 1 + rel].iter().collect();
                let inner_l = inner.trim().to_lowercase();
                if prefixes.iter().any(|p| inner_l.starts_with(p)) || is_year_remaster(&inner_l) {
                    i += rel + 2;
                    continue;
                }
            }
        }
        out.push(c);
        i += 1;
    }
    collapse_spaces(&out)
}

/// "2011 remaster", "2009 remastered version"
fn is_year_remaster(s: &str) -> bool {
    s.get(..4).is_some_and(|y| y.chars().all(|c| c.is_ascii_digit())) && s.contains("remaster")
}

fn strip_dash_suffixes(s: &str) -> String {
    let mut s = s.to_string();
    // Repeatedly drop trailing " - junk" parts ("Song - 2011 Remaster - Radio Edit").
    loop {
        let Some(i) = s.rfind(" - ").or_else(|| s.rfind(" – ")) else { break };
        let sep_len = if s[i..].starts_with(" - ") { 3 } else { " – ".len() };
        let tail = s[i + sep_len..].trim().to_lowercase();
        let junk = JUNK_DASH_PREFIXES.iter().any(|p| tail.starts_with(p))
            || is_year_remaster(&tail)
            || tail.contains("remaster");
        if junk && i > 0 {
            s.truncate(i);
        } else {
            break;
        }
    }
    s
}

fn strip_inline_feat(s: &str) -> String {
    // See `primary_artist`: indices must stay valid in `s`.
    let lower = s.to_ascii_lowercase();
    let mut cut = s.len();
    for pat in [" feat. ", " feat ", " ft. ", " ft ", " featuring "] {
        if let Some(i) = lower.find(pat) {
            if i > 0 {
                cut = cut.min(i);
            }
        }
    }
    if cut == s.len() {
        return s.to_string();
    }
    // Keep a " - suffix" that follows the feat clause, e.g. "Song ft. X - Remix".
    let rest = &s[cut..];
    let keep = rest.find(" - ").map(|j| &rest[j..]).unwrap_or("");
    format!("{}{}", &s[..cut], keep)
}

fn strip_suffix_ci<'a>(s: &'a str, suffix: &str) -> Option<&'a str> {
    if s.len() >= suffix.len() && s.is_char_boundary(s.len() - suffix.len()) && s[s.len() - suffix.len()..].eq_ignore_ascii_case(suffix) {
        Some(&s[..s.len() - suffix.len()])
    } else {
        None
    }
}

fn trim_punct(s: &str) -> String {
    let is_junk = |c: char| c.is_whitespace() || matches!(c, '-' | '–' | '—' | ':' | ';' | ',' | '.' | '·' | '|' | '/' | '"' | '“' | '”' | '*' | '~' | '_');
    s.trim_matches(is_junk).to_string()
}

fn collapse_spaces(s: &str) -> String {
    s.split_whitespace().collect::<Vec<_>>().join(" ")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn titles() {
        assert_eq!(clean_title("Here Comes the Sun - Remastered 2009"), "Here Comes the Sun");
        assert_eq!(clean_title("Song (feat. Someone Else)"), "Song");
        assert_eq!(clean_title("Song [Official Video]"), "Song");
        assert_eq!(clean_title("Song (Official Music Video) [4K]"), "Song");
        assert_eq!(clean_title("Song (Live)"), "Song");
        assert_eq!(clean_title("Song - Radio Edit"), "Song");
        assert_eq!(clean_title("Song - YouTube Music"), "Song");
        assert_eq!(clean_title("Song (2011 Remaster)"), "Song");
        assert_eq!(clean_title("Song ft. Guest"), "Song");
        assert_eq!(clean_title("Song - 2015 Remaster - Radio Edit"), "Song");
        assert_eq!(clean_title("  \"Song\" "), "Song");
        // Things that must survive
        assert_eq!(clean_title("Help!"), "Help!");
        assert_eq!(clean_title("Song (Remix)"), "Song (Remix)");
        assert_eq!(clean_title("Don't Stop Me Now"), "Don't Stop Me Now");
        assert_eq!(clean_title("Part 1 - The Beginning"), "Part 1 - The Beginning");
        assert_eq!(clean_title("Liveforever"), "Liveforever");
    }

    #[test]
    fn music_video_titles_with_credits() {
        assert_eq!(
            clean_title("Annakiliye - Music Video | Smith Asher | Viky Pup | Dheekshitha | Albuquerque Records | Anirudh"),
            "Annakiliye"
        );
        assert_eq!(
            clean_title("Rathinamo - Official Music Video | Saurav Srisan | Tanisha Gnanavel | Vidhu, Yogalakshmi |TamilIndie"),
            "Rathinamo"
        );
        assert_eq!(
            clean_title("Paal Dabba - Aeroplane (Music Video) | Ananya | Rahul Ashok Kumar | Think Indie"),
            "Paal Dabba - Aeroplane"
        );
        assert_eq!(clean_title("Song - Video Song"), "Song");
        assert_eq!(clean_title("Song (Full Video Song)"), "Song");
        // A pipe at the very start is not a credits list.
        assert_eq!(clean_title("| Song"), "Song");
        assert_eq!(
            pipe_credits("Rathinamo - Official Music Video | Saurav Srisan | Vidhu, Yogalakshmi |TamilIndie"),
            vec!["Saurav Srisan", "Vidhu", "Yogalakshmi", "TamilIndie"]
        );
        assert!(pipe_credits("No credits here").is_empty());
    }

    #[test]
    fn artists() {
        assert_eq!(clean_artist("Artist feat. Guest"), "Artist");
        assert_eq!(clean_artist("Artist - Topic"), "Artist");
        assert_eq!(clean_artist("ArtistVEVO"), "Artist");
        assert_eq!(clean_artist("Artist (feat. Guest)"), "Artist");
        assert_eq!(clean_artist("Vevo"), "Vevo");
        assert_eq!(primary_artist("A, B & C"), "A");
        assert_eq!(primary_artist("Simon & Garfunkel"), "Simon");
        assert_eq!(primary_artist("Solo"), "Solo");
    }

    #[test]
    fn keys_and_similarity() {
        assert_eq!(key("Don't  Stop -- Me Now!"), "don t stop me now");
        assert_eq!(key("Simon & Garfunkel"), "simon and garfunkel");
        assert_eq!(key("Beyoncé"), "beyoncé");
        assert!(similarity("hello", "hello") == 1.0);
        assert!(similarity("hello world", "hello word") > 0.85);
        assert!(similarity("song", "song live at wembley") >= 0.9);
        assert!(similarity("abc", "xyz") < 0.1);
    }

    #[test]
    fn youtube_splitting() {
        assert_eq!(
            split_artist_title("Some Band - Great Song (Official Video)", "SomeBandVEVO"),
            Some(("Great Song (Official Video)".into(), "Some Band".into()))
        );
        assert_eq!(
            split_artist_title("Some Band - Great Song", "Some Band"),
            Some(("Great Song".into(), "Some Band".into()))
        );
        assert_eq!(split_artist_title("Part 1 - The Beginning", "Composer"), None);
        assert_eq!(split_artist_title("No dash here", "x"), None);
    }

    #[test]
    fn length_changing_lowercase_does_not_panic() {
        // 'İ' lowercases to a longer string; indices from it used to be applied
        // to the original and could land inside a character.
        assert_eq!(primary_artist("İİİİ, Other"), "İİİİ");
        assert_eq!(clean_artist("İİİİ feat. Someone"), "İİİİ");
        assert_eq!(clean_artist("ÉtienneVEVO"), "Étienne");
        let _ = clean_title("İİİİ ft. X - Remix");
    }

    #[test]
    fn similarity_handles_huge_inputs_quickly() {
        let a = "a".repeat(200_000);
        let b = "b".repeat(200_000);
        let started = std::time::Instant::now();
        assert!(similarity(&a, &b) < 0.5);
        assert!(started.elapsed() < std::time::Duration::from_secs(2));
    }
}
