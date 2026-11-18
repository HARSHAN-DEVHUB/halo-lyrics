//! Artwork: decode, downscale to ≤1024 px, re-encode as a JPEG data URL and
//! extract a 5-colour palette (median cut on a 64×64 thumbnail).

use base64::Engine as _;
use image::{imageops::FilterType, DynamicImage, GenericImageView, ImageFormat};
use serde::Serialize;
use std::io::Cursor;

pub const MAX_SIDE: u32 = 1024;

#[derive(Serialize, Clone, Debug)]
#[serde(rename_all = "camelCase")]
pub struct Artwork {
    pub data_url: String,
    /// 5 hex colours, most vivid first.
    pub palette: Vec<String>,
}

pub fn process(bytes: &[u8]) -> Option<Artwork> {
    let img = image::load_from_memory(bytes).ok()?;
    let img = crop_letterbox(img);
    let (w, h) = img.dimensions();
    if w == 0 || h == 0 {
        return None;
    }
    let img = if w.max(h) > MAX_SIDE { img.resize(MAX_SIDE, MAX_SIDE, FilterType::Triangle) } else { img };
    let rgb = DynamicImage::ImageRgb8(img.to_rgb8());

    let mut jpeg = Vec::new();
    {
        let mut enc = image::codecs::jpeg::JpegEncoder::new_with_quality(Cursor::new(&mut jpeg), 88);
        if enc.encode_image(&rgb).is_err() {
            // Fall back to PNG if JPEG fails for some reason.
            jpeg.clear();
            rgb.write_to(&mut Cursor::new(&mut jpeg), ImageFormat::Png).ok()?;
            let b64 = base64::engine::general_purpose::STANDARD.encode(&jpeg);
            return Some(Artwork { data_url: format!("data:image/png;base64,{b64}"), palette: palette(&rgb) });
        }
    }
    let b64 = base64::engine::general_purpose::STANDARD.encode(&jpeg);
    Some(Artwork { data_url: format!("data:image/jpeg;base64,{b64}"), palette: palette(&rgb) })
}

/// Video thumbnails (YouTube in a browser) often arrive as 16:9 with black bars
/// around square art. Trim near-black rows/columns at the edges.
fn crop_letterbox(img: DynamicImage) -> DynamicImage {
    let (w, h) = img.dimensions();
    if w < 32 || h < 32 {
        return img;
    }
    // Square album art is never cropped (dark covers would lose their edges).
    let aspect = w.max(h) as f64 / w.min(h) as f64;
    if aspect < 1.15 {
        return img;
    }
    let small = img.thumbnail_exact(64, 64).to_rgb8();
    let dark = |x: u32, y: u32| {
        let p = small.get_pixel(x, y).0;
        (p[0] as u32 + p[1] as u32 + p[2] as u32) < 40
    };
    let col_dark = |x: u32| (0..64).all(|y| dark(x, y));
    let row_dark = |y: u32| (0..64).all(|x| dark(x, y));
    let (mut l, mut r, mut t, mut b) = (0u32, 63u32, 0u32, 63u32);
    while l < r && col_dark(l) {
        l += 1;
    }
    while r > l && col_dark(r) {
        r -= 1;
    }
    while t < b && row_dark(t) {
        t += 1;
    }
    while b > t && row_dark(b) {
        b -= 1;
    }
    // Only crop when there are real bars and something substantial remains.
    if (l == 0 && r == 63 && t == 0 && b == 63) || r - l < 16 || b - t < 16 {
        return img;
    }
    let sx = w as f64 / 64.0;
    let sy = h as f64 / 64.0;
    let x0 = (l as f64 * sx).floor() as u32;
    let y0 = (t as f64 * sy).floor() as u32;
    let x1 = (((r + 1) as f64) * sx).ceil().min(w as f64) as u32;
    let y1 = (((b + 1) as f64) * sy).ceil().min(h as f64) as u32;
    img.crop_imm(x0, y0, x1 - x0, y1 - y0)
}

// ------------------------------------------------------------------ palette

#[derive(Clone)]
struct Bucket {
    px: Vec<[u8; 3]>,
}

impl Bucket {
    fn range(&self) -> (usize, u8) {
        let mut lo = [255u8; 3];
        let mut hi = [0u8; 3];
        for p in &self.px {
            for c in 0..3 {
                lo[c] = lo[c].min(p[c]);
                hi[c] = hi[c].max(p[c]);
            }
        }
        let spans = [hi[0] - lo[0], hi[1] - lo[1], hi[2] - lo[2]];
        let ch = (0..3).max_by_key(|&c| spans[c]).unwrap_or(0);
        (ch, spans[ch])
    }
    fn mean(&self) -> [u8; 3] {
        let n = self.px.len().max(1) as u64;
        let mut s = [0u64; 3];
        for p in &self.px {
            for c in 0..3 {
                s[c] += p[c] as u64;
            }
        }
        [(s[0] / n) as u8, (s[1] / n) as u8, (s[2] / n) as u8]
    }
}

/// Median cut into up to 12 boxes, then the 5 most vivid colours (OKLCH chroma,
/// lightly weighted by population so a few stray pixels don't win).
pub fn palette(img: &DynamicImage) -> Vec<String> {
    let thumb = img.thumbnail_exact(64, 64).to_rgb8();
    let px: Vec<[u8; 3]> = thumb.pixels().map(|p| p.0).collect();
    let total = px.len() as f64;
    let mut buckets = vec![Bucket { px }];
    while buckets.len() < 12 {
        // Split the bucket with the widest channel range.
        let Some((idx, (ch, span))) = buckets
            .iter()
            .enumerate()
            .filter(|(_, b)| b.px.len() > 1)
            .map(|(i, b)| (i, b.range()))
            .max_by_key(|(_, (_, span))| *span)
        else {
            break;
        };
        if span < 8 {
            break;
        }
        let mut b = buckets.swap_remove(idx);
        b.px.sort_unstable_by_key(|p| p[ch]);
        let upper = b.px.split_off(b.px.len() / 2);
        buckets.push(b);
        buckets.push(Bucket { px: upper });
    }

    let mut colours: Vec<([u8; 3], f64, f64)> = buckets
        .iter()
        .filter(|b| !b.px.is_empty())
        .map(|b| {
            let m = b.mean();
            let share = b.px.len() as f64 / total;
            (m, chroma(m), share)
        })
        .collect();
    colours.sort_by(|a, b| {
        let sa = a.1 * (0.5 + 0.5 * a.2.sqrt());
        let sb = b.1 * (0.5 + 0.5 * b.2.sqrt());
        sb.partial_cmp(&sa).unwrap_or(std::cmp::Ordering::Equal)
    });
    let mut out: Vec<String> = colours.iter().map(|(c, _, _)| hex(*c)).collect();
    out.dedup();
    while out.len() < 5 {
        out.push(out.last().cloned().unwrap_or_else(|| "#09090C".into()));
    }
    out.truncate(5);
    out
}

fn hex(c: [u8; 3]) -> String {
    format!("#{:02X}{:02X}{:02X}", c[0], c[1], c[2])
}

/// OKLCH chroma of an sRGB colour.
pub fn chroma(c: [u8; 3]) -> f64 {
    let lin = |v: u8| {
        let v = v as f64 / 255.0;
        if v <= 0.04045 { v / 12.92 } else { ((v + 0.055) / 1.055).powf(2.4) }
    };
    let (r, g, b) = (lin(c[0]), lin(c[1]), lin(c[2]));
    let l = (0.412_221_470_8 * r + 0.536_332_536_3 * g + 0.051_445_992_9 * b).cbrt();
    let m = (0.211_903_498_2 * r + 0.680_699_545_1 * g + 0.107_396_956_6 * b).cbrt();
    let s = (0.088_302_461_9 * r + 0.281_718_837_6 * g + 0.629_978_700_5 * b).cbrt();
    let a = 1.977_998_495_1 * l - 2.428_592_205 * m + 0.450_593_709_9 * s;
    let bb = 0.025_904_037_1 * l + 0.782_771_766_2 * m - 0.808_675_766 * s;
    (a * a + bb * bb).sqrt()
}

#[cfg(test)]
mod tests {
    use super::*;
    use image::{Rgb, RgbImage};

    fn two_tone() -> DynamicImage {
        let mut img = RgbImage::new(200, 200);
        for (x, _y, p) in img.enumerate_pixels_mut() {
            *p = if x < 140 { Rgb([40, 40, 40]) } else { Rgb([220, 30, 60]) };
        }
        DynamicImage::ImageRgb8(img)
    }

    #[test]
    fn palette_puts_vivid_first() {
        let p = palette(&two_tone());
        assert_eq!(p.len(), 5);
        let first = u32::from_str_radix(&p[0][1..], 16).unwrap();
        let (r, g) = ((first >> 16) & 255, (first >> 8) & 255);
        assert!(r > 150 && g < 80, "{p:?}");
    }

    #[test]
    fn process_downscales_and_encodes() {
        let mut img = RgbImage::new(2000, 1500);
        for (x, y, p) in img.enumerate_pixels_mut() {
            *p = Rgb([(x % 256) as u8, (y % 256) as u8, 128]);
        }
        let mut png = Vec::new();
        DynamicImage::ImageRgb8(img).write_to(&mut Cursor::new(&mut png), ImageFormat::Png).unwrap();
        let art = process(&png).unwrap();
        assert!(art.data_url.starts_with("data:image/jpeg;base64,"));
        let b64 = &art.data_url["data:image/jpeg;base64,".len()..];
        let bytes = base64::engine::general_purpose::STANDARD.decode(b64).unwrap();
        let decoded = image::load_from_memory(&bytes).unwrap();
        assert_eq!(decoded.dimensions(), (1024, 768));
    }

    #[test]
    fn crops_letterbox_bars() {
        let mut img = RgbImage::new(320, 180);
        for (x, _y, p) in img.enumerate_pixels_mut() {
            *p = if (70..250).contains(&x) { Rgb([200, 120, 40]) } else { Rgb([0, 0, 0]) };
        }
        let cropped = crop_letterbox(DynamicImage::ImageRgb8(img));
        let (w, h) = cropped.dimensions();
        assert!((175..=195).contains(&w) && h == 180, "{w}x{h}");
    }

    #[test]
    fn garbage_is_rejected() {
        assert!(process(b"not an image").is_none());
    }
}
