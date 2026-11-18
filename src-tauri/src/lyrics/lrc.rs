//! LRC + enhanced-LRC parser.
//!
//! * `[mm:ss.xx] text`, several timestamps per line allowed, `[offset:±ms]`.
//! * Enhanced word tags `<mm:ss.xx>word` fill `words`.
//! * Lines without word timing get synthetic timing (char-weighted, min 120 ms
//!   per word, squeezed into the first 85% so the last word holds a beat).
//! * `endMs` = next line's start, capped at start + 8 s.
//! * Empty lines become a "♪" breather.

use super::{Line, Word};

pub const BREATHER: &str = "♪";
pub const MAX_LINE_MS: i64 = 8_000;
pub const MIN_WORD_MS: f64 = 120.0;
/// Synthetic words occupy this share of the line; the last word holds the rest.
pub const WORDS_SHARE: f64 = 0.85;
/// `[offset:...]` beyond ±1 h is nonsense; clamping also rules out overflow.
const MAX_OFFSET_MS: i64 = 60 * 60 * 1000;
/// Longest accepted timestamp component ("mmmm"), so the maths can't overflow.
const MAX_STAMP_DIGITS: usize = 4;

struct RawLine {
    start: i64,
    text: String,
    /// (absolute start, text) from enhanced tags, plus an optional trailing end tag.
    words: Option<(Vec<(i64, String)>, Option<i64>)>,
}

/// Parses an LRC document. `duration_ms` (if known) bounds the last line.
pub fn parse(src: &str, duration_ms: Option<u64>) -> Vec<Line> {
    let mut offset: i64 = 0;
    let mut raw: Vec<RawLine> = Vec::new();

    for line in src.lines() {
        let line = line.trim_start_matches('\u{feff}').trim();
        if line.is_empty() {
            continue;
        }
        let mut rest = line;
        let mut stamps: Vec<i64> = Vec::new();
        // Consume leading [..] tags.
        while let Some(stripped) = rest.strip_prefix('[') {
            let Some(close) = stripped.find(']') else { break };
            let tag = &stripped[..close];
            if let Some(t) = parse_timestamp(tag) {
                stamps.push(t);
            } else if let Some((key, value)) = tag.split_once(':') {
                if key.trim().eq_ignore_ascii_case("offset") {
                    offset = value
                        .trim()
                        .trim_start_matches('+')
                        .parse::<i64>()
                        .unwrap_or(0)
                        .clamp(-MAX_OFFSET_MS, MAX_OFFSET_MS);
                }
            } else {
                break;
            }
            rest = &stripped[close + 1..];
        }
        if stamps.is_empty() {
            continue;
        }
        let (text, words) = parse_enhanced(rest);
        let first = stamps[0];
        for t in stamps {
            // Repeated lines reuse the word tags, shifted by the stamp delta.
            let shifted = words.as_ref().map(|(ws, end)| {
                let d = t - first;
                (
                    ws.iter().map(|(s, w)| (s + d, w.clone())).collect::<Vec<_>>(),
                    end.map(|e| e + d),
                )
            });
            raw.push(RawLine { start: t, text: text.clone(), words: shifted });
        }
    }

    // A positive offset makes lyrics appear sooner.
    for r in &mut raw {
        r.start = (r.start - offset).max(0);
        if let Some((ws, end)) = &mut r.words {
            for (s, _) in ws.iter_mut() {
                *s = (*s - offset).max(0);
            }
            if let Some(e) = end {
                *e = (*e - offset).max(0);
            }
        }
    }
    raw.sort_by_key(|r| r.start); // stable: keeps file order for equal stamps

    let duration = duration_ms.map(|d| d as i64).filter(|d| *d > 0);
    let n = raw.len();
    let mut out = Vec::with_capacity(n);
    for i in 0..n {
        let start = raw[i].start;
        let cap = start + MAX_LINE_MS;
        let end = match raw.get(i + 1) {
            Some(next) => next.start.min(cap),
            None => match duration {
                Some(d) if d > start => d.min(cap),
                _ => cap,
            },
        }
        .max(start);

        let text = raw[i].text.trim().to_string();
        let (text, words) = if text.is_empty() {
            let w = vec![Word { start_ms: start as u64, end_ms: end as u64, text: BREATHER.into() }];
            (BREATHER.to_string(), w)
        } else if let Some((ws, tail)) = &raw[i].words {
            (text, enhanced_words(ws, *tail, start, end))
        } else {
            let w = synth_words(&text, start, end);
            (text, w)
        };
        out.push(Line { start_ms: start as u64, end_ms: end as u64, text, words: Some(words) });
    }
    out
}

/// True if the document has at least one timestamped line.
pub fn has_timestamps(src: &str) -> bool {
    src.lines().any(|l| {
        let l = l.trim();
        l.starts_with('[') && l[1..].split(']').next().and_then(parse_timestamp).is_some()
    })
}

/// `mm:ss`, `mm:ss.x`, `mm:ss.xx`, `mm:ss.xxx`, `mm:ss:xx`, `h:mm:ss.xx`.
pub fn parse_timestamp(tag: &str) -> Option<i64> {
    let tag = tag.trim();
    if tag.is_empty() || !tag.as_bytes()[0].is_ascii_digit() {
        return None;
    }
    let (main, frac) = match tag.find('.') {
        Some(i) => (&tag[..i], Some(&tag[i + 1..])),
        None => (tag, None),
    };
    let parts: Vec<&str> = main.split(':').collect();
    let all_digits = |s: &str| !s.is_empty() && s.bytes().all(|b| b.is_ascii_digit());
    if !parts.iter().all(|p| all_digits(p) && p.len() <= MAX_STAMP_DIGITS) {
        return None;
    }
    let nums: Vec<i64> = parts.iter().map(|p| p.parse().unwrap_or(0)).collect();
    let (h, m, s, mut ms) = match (nums.len(), frac) {
        (2, _) => (0, nums[0], nums[1], 0),
        // `mm:ss:xx` uses a colon for hundredths.
        (3, None) if parts[2].len() == 2 && nums[1] < 60 && parts[0].len() >= 2 => {
            (0, nums[0], nums[1], nums[2] * 10)
        }
        (3, _) => (nums[0], nums[1], nums[2], 0),
        _ => return None,
    };
    if s >= 60 {
        return None;
    }
    if let Some(f) = frac {
        if !all_digits(f) {
            return None;
        }
        let digits = &f[..f.len().min(3)];
        ms = format!("{digits:0<3}").parse().ok()?;
    }
    Some(((h * 60 + m) * 60 + s) * 1000 + ms)
}

/// Splits `<mm:ss.xx>word <mm:ss.xx>word <mm:ss.xx>` into words. Returns the
/// plain text and (if any tags were present) the word starts + trailing end tag.
fn parse_enhanced(text: &str) -> (String, Option<(Vec<(i64, String)>, Option<i64>)>) {
    if !text.contains('<') {
        return (text.trim().to_string(), None);
    }
    let mut words: Vec<(i64, String)> = Vec::new();
    let mut tail: Option<i64> = None;
    let mut rest = text;
    let mut leading = String::new();
    let mut saw_tag = false;
    while !rest.is_empty() {
        if let Some(after) = rest.strip_prefix('<') {
            if let Some(close) = after.find('>') {
                if let Some(t) = parse_timestamp(&after[..close]) {
                    saw_tag = true;
                    let body_start = close + 1;
                    let body_end = after[body_start..].find('<').map(|i| body_start + i).unwrap_or(after.len());
                    let body = after[body_start..body_end].trim();
                    if body.is_empty() {
                        tail = Some(t);
                    } else {
                        // A segment can hold several words ("<t>oh my"): split them,
                        // they share the segment's time and are spread out later.
                        for w in body.split_whitespace() {
                            words.push((t, w.to_string()));
                        }
                        tail = None;
                    }
                    rest = &after[body_end..];
                    continue;
                }
            }
        }
        // Text before the first tag (or a stray '<'). Step over the first
        // *character*, not byte: lyrics are often non-ASCII.
        let first = rest.chars().next().map_or(1, char::len_utf8);
        let next = rest[first..].find('<').map(|i| i + first).unwrap_or(rest.len());
        leading.push_str(&rest[..next]);
        rest = &rest[next..];
    }
    if !saw_tag || words.is_empty() {
        let plain = if leading.trim().is_empty() { text } else { &leading };
        return (plain.trim().to_string(), None);
    }
    let plain = words.iter().map(|(_, w)| w.as_str()).collect::<Vec<_>>().join(" ");
    (plain, Some((words, tail)))
}

fn enhanced_words(ws: &[(i64, String)], tail: Option<i64>, line_start: i64, line_end: i64) -> Vec<Word> {
    let mut out: Vec<Word> = Vec::with_capacity(ws.len());
    let n = ws.len();
    let mut i = 0;
    while i < n {
        // Group words that share a tag time, then spread them across the slot.
        let t = ws[i].0.clamp(line_start, line_end);
        let mut j = i + 1;
        while j < n && ws[j].0 == ws[i].0 {
            j += 1;
        }
        let slot_end = if j < n { ws[j].0 } else { tail.unwrap_or(line_end) }.clamp(t, line_end.max(t));
        let count = (j - i) as i64;
        let step = ((slot_end - t) / count).max(0);
        for (k, (_, w)) in ws[i..j].iter().enumerate() {
            let s = t + step * k as i64;
            let e = if k as i64 == count - 1 { slot_end } else { s + step };
            out.push(Word { start_ms: s as u64, end_ms: e as u64, text: w.clone() });
        }
        i = j;
    }
    // Enforce monotonic starts; last word holds to the end of the line.
    for k in 1..out.len() {
        if out[k].start_ms < out[k - 1].start_ms {
            out[k].start_ms = out[k - 1].start_ms;
        }
    }
    if let Some(last) = out.last_mut() {
        last.end_ms = (line_end as u64).max(last.start_ms);
    }
    out
}

/// True for scripts written without spaces between words (Han, kana).
fn is_cjk(c: char) -> bool {
    matches!(c as u32,
        0x3040..=0x30FF   // Hiragana, Katakana
        | 0x3400..=0x4DBF // CJK Ext A
        | 0x4E00..=0x9FFF // CJK Unified
        | 0xF900..=0xFAFF // CJK Compatibility
        | 0xFF66..=0xFF9D // Half-width Katakana
    )
}

/// Splits a line into display words. Han/kana are split per character since
/// they don't use spaces; trailing punctuation stays with its character.
pub fn split_words(text: &str) -> Vec<String> {
    let mut out: Vec<String> = Vec::new();
    for token in text.split_whitespace() {
        if !token.chars().any(is_cjk) {
            out.push(token.to_string());
            continue;
        }
        let mut cur = String::new();
        for c in token.chars() {
            if is_cjk(c) {
                if !cur.is_empty() {
                    out.push(std::mem::take(&mut cur));
                }
                cur.push(c);
            } else if c.is_alphanumeric() && !cur.is_empty() && cur.chars().all(is_cjk) {
                out.push(std::mem::take(&mut cur));
                cur.push(c);
            } else {
                cur.push(c);
            }
        }
        if !cur.is_empty() {
            out.push(cur);
        }
    }
    out
}

/// Synthetic word timing for a line `[start, end)`.
pub fn synth_words(text: &str, start: i64, end: i64) -> Vec<Word> {
    let tokens = split_words(text);
    let n = tokens.len();
    if n == 0 {
        return Vec::new();
    }
    let dur = (end - start).max(1) as f64;
    let avail = dur * WORDS_SHARE;
    let weights: Vec<f64> = tokens.iter().map(|t| t.chars().filter(|c| c.is_alphanumeric()).count().max(1) as f64).collect();

    // Water-filling: proportional to weight, but never below MIN_WORD_MS.
    let mut alloc = vec![0.0; n];
    if MIN_WORD_MS * n as f64 >= avail {
        alloc.iter_mut().for_each(|a| *a = avail / n as f64);
    } else {
        let mut fixed = vec![false; n];
        loop {
            let fixed_total = fixed.iter().filter(|f| **f).count() as f64 * MIN_WORD_MS;
            let free_weight: f64 = weights.iter().zip(&fixed).filter(|(_, f)| !**f).map(|(w, _)| *w).sum();
            let remaining = avail - fixed_total;
            let mut changed = false;
            for i in 0..n {
                if fixed[i] {
                    alloc[i] = MIN_WORD_MS;
                    continue;
                }
                alloc[i] = remaining * weights[i] / free_weight;
                if alloc[i] < MIN_WORD_MS {
                    fixed[i] = true;
                    changed = true;
                }
            }
            if !changed {
                break;
            }
        }
    }

    let mut words = Vec::with_capacity(n);
    let mut t = start as f64;
    for (i, tok) in tokens.into_iter().enumerate() {
        let s = t.round() as i64;
        t += alloc[i];
        let e = if i == n - 1 { end } else { t.round() as i64 };
        words.push(Word { start_ms: s as u64, end_ms: e.max(s) as u64, text: tok });
    }
    words
}

#[cfg(test)]
mod tests {
    use super::*;

    fn starts(lines: &[Line]) -> Vec<u64> {
        lines.iter().map(|l| l.start_ms).collect()
    }

    #[test]
    fn timestamps() {
        assert_eq!(parse_timestamp("00:12.34"), Some(12_340));
        assert_eq!(parse_timestamp("01:02.5"), Some(62_500));
        assert_eq!(parse_timestamp("01:02.123"), Some(62_123));
        assert_eq!(parse_timestamp("01:02"), Some(62_000));
        assert_eq!(parse_timestamp("01:02:50"), Some(62_500));
        assert_eq!(parse_timestamp("1:01:02.00"), Some(3_662_000));
        assert_eq!(parse_timestamp("ar:Someone"), None);
        assert_eq!(parse_timestamp("00:75.00"), None);
    }

    #[test]
    fn basic_lines_and_end_times() {
        let src = "[ti:Test]\n[00:01.00]One\n[00:03.50]Two\n[00:20.00]Three";
        let lines = parse(src, Some(25_000));
        assert_eq!(starts(&lines), vec![1000, 3500, 20_000]);
        assert_eq!(lines[0].end_ms, 3500);
        // capped at start + 8 s
        assert_eq!(lines[1].end_ms, 11_500);
        // last line bounded by duration
        assert_eq!(lines[2].end_ms, 25_000);
        assert_eq!(lines[1].text, "Two");
    }

    #[test]
    fn multiple_timestamps_per_line() {
        let src = "[00:10.00][00:30.00]Chorus line\n[00:20.00]Verse";
        let lines = parse(src, None);
        assert_eq!(starts(&lines), vec![10_000, 20_000, 30_000]);
        assert_eq!(lines[0].text, "Chorus line");
        assert_eq!(lines[2].text, "Chorus line");
        assert_eq!(lines[1].text, "Verse");
    }

    #[test]
    fn offsets() {
        // +500 → lyrics appear 500 ms sooner
        let lines = parse("[offset:+500]\n[00:02.00]Hello", None);
        assert_eq!(lines[0].start_ms, 1500);
        let lines = parse("[offset:-250]\n[00:02.00]Hello", None);
        assert_eq!(lines[0].start_ms, 2250);
        // never negative
        let lines = parse("[offset:5000]\n[00:01.00]Hello", None);
        assert_eq!(lines[0].start_ms, 0);
    }

    #[test]
    fn enhanced_word_tags() {
        let src = "[00:05.00]<00:05.00>Every <00:05.50>light <00:06.10>hums <00:07.00>\n[00:09.00]Next";
        let lines = parse(src, None);
        assert_eq!(lines[0].text, "Every light hums");
        let w = lines[0].words.as_ref().unwrap();
        assert_eq!(w.len(), 3);
        assert_eq!((w[0].start_ms, w[0].end_ms, w[0].text.as_str()), (5000, 5500, "Every"));
        assert_eq!((w[1].start_ms, w[1].end_ms), (5500, 6100));
        // last word holds to the end of the line
        assert_eq!((w[2].start_ms, w[2].end_ms), (6100, 9000));
    }

    #[test]
    fn enhanced_tags_with_offset_and_repeat() {
        let src = "[offset:+1000]\n[00:05.00][00:15.00]<00:05.00>A <00:06.00>B";
        let lines = parse(src, None);
        assert_eq!(starts(&lines), vec![4000, 14_000]);
        let w2 = lines[1].words.as_ref().unwrap();
        assert_eq!((w2[0].start_ms, w2[1].start_ms), (14_000, 15_000));
    }

    #[test]
    fn empty_lines_become_breathers() {
        let lines = parse("[00:01.00]Hi\n[00:04.00]\n[00:10.00]Again", None);
        assert_eq!(lines[1].text, BREATHER);
        let w = lines[1].words.as_ref().unwrap();
        assert_eq!(w.len(), 1);
        assert_eq!(w[0].text, BREATHER);
        assert_eq!((lines[1].start_ms, lines[1].end_ms), (4000, 10_000));
    }

    #[test]
    fn junk_and_metadata_lines_are_ignored() {
        let lines = parse("[ar:Me]\n[length: 03:20]\nno tag here\n[00:01.00]Ok", None);
        assert_eq!(lines.len(), 1);
        assert_eq!(lines[0].text, "Ok");
    }

    #[test]
    fn synthetic_timing_is_weighted_and_holds_the_last_word() {
        let w = synth_words("a beautiful day", 0, 4000);
        assert_eq!(w.len(), 3);
        assert_eq!(w[0].start_ms, 0);
        // words are contiguous
        assert_eq!(w[0].end_ms, w[1].start_ms);
        assert_eq!(w[1].end_ms, w[2].start_ms);
        // longer words get more time
        let d0 = w[0].end_ms - w[0].start_ms;
        let d1 = w[1].end_ms - w[1].start_ms;
        assert!(d1 > d0 * 3, "d0={d0} d1={d1}");
        // the last word starts within the first 85% and holds until the end
        let last_slot = {
            let total = 4000.0 * WORDS_SHARE;
            total - w[2].start_ms as f64
        };
        assert!(last_slot > 0.0);
        assert!((w[2].start_ms as f64) < 4000.0 * WORDS_SHARE);
        assert_eq!(w[2].end_ms, 4000);
    }

    #[test]
    fn synthetic_timing_minimum_per_word() {
        // "a" alone would get ~1/31 of the time; it must still get 120 ms.
        let w = synth_words("a incomprehensibilities", 1000, 3000);
        assert!(w[0].end_ms - w[0].start_ms >= 120, "{:?}", w);
        // If even the minimum doesn't fit, split evenly.
        let w = synth_words("a b c d e f g h i j", 0, 600);
        assert_eq!(w.len(), 10);
        assert!(w.windows(2).all(|p| p[0].start_ms <= p[1].start_ms));
        assert_eq!(w[9].end_ms, 600);
    }

    #[test]
    fn cjk_is_split_per_character() {
        assert_eq!(split_words("君の名は"), vec!["君", "の", "名", "は"]);
        assert_eq!(split_words("hello 世界!"), vec!["hello", "世", "界!"]);
        assert_eq!(split_words("사랑 해요"), vec!["사랑", "해요"]);
    }

    #[test]
    fn detects_timestamps() {
        assert!(has_timestamps("[ar:x]\n[00:01.00]hi"));
        assert!(!has_timestamps("just\nplain\nlyrics"));
    }

    #[test]
    fn non_ascii_before_angle_bracket_does_not_panic() {
        // Used to slice inside a multi-byte char and abort the app.
        let lines = parse("[00:01.00]♥ <3 you
[00:03.00]日本<00:03.50>語
[00:05.00]é<x", None);
        assert_eq!(lines.len(), 3);
        assert_eq!(lines[0].text, "♥ <3 you");
    }

    #[test]
    fn absurd_timestamps_and_offsets_do_not_overflow() {
        assert_eq!(parse_timestamp("99999999999999999:00.00"), None);
        assert_eq!(parse_timestamp("9223372036854775807:00:00"), None);
        let lines = parse("[offset:-9223372036854775808]
[00:01.00]a
[00:02.00]b", None);
        assert_eq!(lines.len(), 2);
        let lines = parse("[offset:9223372036854775807]
[00:01.00]a", None);
        assert_eq!(lines[0].start_ms, 0);
    }
}
