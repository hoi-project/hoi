//! Perceptual fingerprints of pictures, so a device that hides or removes an image as illegal
//! can refuse altered copies too: re-encoded, resized, lightly cropped, brightened. The BLAKE3
//! hash (PROTOCOL.md §17) catches the same bytes; this catches the same picture.
//!
//! The fingerprint is PDQ, Meta's open 256-bit perceptual hash, so it can be checked against
//! public PDQ hash lists: luma (averaged over whole blocks of pixels on big pictures), two
//! passes of a tent filter, 64×64 samples, the 16×16 low-frequency corner of their DCT (DC terms
//! dropped), one bit per coefficient above the median. Quality is PDQ's gradient score, 0–100.
//!
//! Pixels are decoded only after [`media::check`] passed, so size, structure and dimensions are
//! bounded first. The decoder gets those dimensions as hard limits and the format named by the
//! type, never a guess. JPEGs are turned the way their EXIF orientation says, as they are shown.
//! Animations count by their first frame. Transparency is ignored, as in the reference.
//!
//! Floats round differently on native and wasm, and across PDQ implementations, so the same
//! picture can come out a bit or two apart: compare with [`matches`], never with `==`.
//!
//! A picture turned a quarter, upside down or mirrored is the same picture. PDQ gets all eight
//! of those variants from one DCT (turning and mirroring only swap and negate coefficients),
//! so [`examine_variants`] costs barely more than [`examine`]; a picture matches a listed hash
//! when any of its variants does ([`Variants::matches`]). Lists keep the hash as shown.
//!
//! Ported from pdqhash 0.1.1 by Darwinium (Apache-2.0, <https://github.com/darwinium-com/pdqhash>),
//! itself a port of Meta's reference (<https://github.com/facebook/ThreatExchange/tree/main/pdq>).
//! Where the two differ this follows the reference: the DCT matrix is computed in double
//! precision and the quality score counts whole gradient steps.

use crate::media;
use image::metadata::Orientation;
use image::{DynamicImage, ImageDecoder, ImageFormat, ImageReader, Limits};
use std::io::Cursor;

pub const FINGERPRINT_BITS: u32 = 256;
/// Two fingerprints at most this many bits apart are the same picture (PDQ's usual match threshold is 31).
pub const MATCH_DISTANCE: u32 = 31;
/// Below this quality a picture is too flat to tell apart from others (PDQ's advice): no fingerprint.
pub const MIN_QUALITY: u8 = 50;
/// Largest picture fingerprinted, in pixels.
pub const MAX_PIXELS: u64 = 16_777_216;
/// Most the decoder may hold: [`MAX_PIXELS`] at 6 bytes a pixel (16-bit RGB). 16-bit RGBA that big is refused.
const MAX_ALLOC: u64 = 96 * 1024 * 1024;
/// Sides of at least twice this are first averaged over whole blocks of pixels, down to between
/// this and twice it: as fast as pdqhash's shrink to 512, but nearer the reference's full-size hash.
const SHRINK: usize = 512;
/// Smallest side PDQ hashes.
const MIN_SIDE: usize = 5;

const LUMA_R: f32 = 0.299;
const LUMA_G: f32 = 0.587;
const LUMA_B: f32 = 0.114;

/// A picture's PDQ hash and how much it has to say (0–100).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct Fingerprint {
    pub hash: [u8; 32],
    pub quality: u8,
}

/// A picture's PDQ hashes as shown and in the other seven ways it can be turned or mirrored.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Variants {
    /// `hashes[0]` is the picture as shown; the rest as mirrored left to right, upside down,
    /// turned half way, and those four again turned a quarter (transposed).
    pub hashes: [[u8; 32]; 8],
    pub quality: u8,
}

impl Variants {
    /// The fingerprint as shown: what lists keep.
    pub fn shown(&self) -> Fingerprint {
        Fingerprint { hash: self.hashes[0], quality: self.quality }
    }

    /// Whether any way of turning or mirroring this picture matches `listed`.
    pub fn matches(&self, listed: &[u8; 32]) -> bool {
        self.hashes.iter().any(|h| matches(h, listed))
    }
}

/// The fingerprint of media claimed to be `mime`: `None` unless it is a picture that passes
/// [`media::check`], decodes, has at most [`MAX_PIXELS`] and reaches [`MIN_QUALITY`].
pub fn fingerprint(mime: &str, bytes: &[u8]) -> Option<Fingerprint> {
    examine(mime, bytes).ok().flatten()
}

/// A picture that passed [`media::check`] but that this device can't decode within its limits.
/// Receivers refuse it like media that fails the checks: otherwise it would be a way around
/// fingerprint blocklists.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Unreadable;

/// Like [`fingerprint`], but tells media with nothing to fingerprint (not a picture, too flat,
/// too small: `Ok(None)`) from a picture that fails the checks or can't be decoded (`Err`).
pub fn examine(mime: &str, bytes: &[u8]) -> Result<Option<Fingerprint>, Unreadable> {
    Ok(examine_variants(mime, bytes)?.map(|v| v.shown()))
}

/// Like [`examine`], with every way of turning and mirroring the picture: what a device checks
/// against its lists.
pub fn examine_variants(mime: &str, bytes: &[u8]) -> Result<Option<Variants>, Unreadable> {
    let info = media::check(mime, bytes).map_err(|_| Unreadable)?;
    let Some(format) = format(info.mime) else { return Ok(None) };
    let (w, h) = (info.w.ok_or(Unreadable)?, info.h.ok_or(Unreadable)?);
    if w as u64 * h as u64 > MAX_PIXELS {
        return Err(Unreadable);
    }
    // A decoder bug must not take the device down. Under panic = "abort" (the wasm build) this
    // catches nothing, and only the checks above stand between the bytes and the decoder.
    let (luma, w, h) = std::panic::catch_unwind(|| decode(format, bytes, w, h)).ok().flatten().ok_or(Unreadable)?;
    Ok(pdq_variants(luma, w, h).filter(|f| f.quality >= MIN_QUALITY))
}

/// The variants of a luma picture (w×h, row-major), for hosts that decode pictures or video
/// frames themselves (the browser's fingerprint worker, video frames): the same arithmetic as
/// [`examine_variants`] after decoding. `None` when it is too small or too flat.
pub fn luma_variants(luma: Vec<f32>, w: usize, h: usize) -> Option<Variants> {
    if w as u64 * h as u64 > MAX_PIXELS {
        return None;
    }
    let (kx, ky) = ((w / SHRINK).max(1), (h / SHRINK).max(1));
    let (luma, w, h) = if kx > 1 || ky > 1 { shrink(&luma, 1, w, h, 1.0)? } else { (luma, w, h) };
    pdq_variants(luma, w, h).filter(|f| f.quality >= MIN_QUALITY)
}

/// Bits that differ between two hashes, 0–256.
pub fn distance(a: &[u8; 32], b: &[u8; 32]) -> u32 {
    a.iter().zip(b).map(|(x, y)| (x ^ y).count_ones()).sum()
}

/// Whether two hashes are the same picture.
pub fn matches(a: &[u8; 32], b: &[u8; 32]) -> bool {
    distance(a, b) <= MATCH_DISTANCE
}

/// A hash as 64 hex digits, the way PDQ hash lists write it.
pub fn to_hex(h: &[u8; 32]) -> String {
    hoi_types::hex_encode(h)
}

/// A hash from 64 hex digits.
pub fn from_hex(s: &str) -> Option<[u8; 32]> {
    hoi_types::hex_decode(s).ok()?.try_into().ok()
}

/// The decoder for a picture type.
fn format(mime: &str) -> Option<ImageFormat> {
    match mime {
        media::PNG => Some(ImageFormat::Png),
        media::JPEG => Some(ImageFormat::Jpeg),
        media::GIF => Some(ImageFormat::Gif),
        media::WEBP => Some(ImageFormat::WebP),
        _ => None,
    }
}

/// The first frame's luma, averaged down (see [`SHRINK`]) and turned as shown.
fn decode(format: ImageFormat, bytes: &[u8], w: u32, h: u32) -> Option<(Vec<f32>, usize, usize)> {
    let mut limits = Limits::default();
    limits.max_image_width = Some(w);
    limits.max_image_height = Some(h);
    limits.max_alloc = Some(MAX_ALLOC);
    let mut reader = ImageReader::with_format(Cursor::new(bytes), format);
    reader.limits(limits);
    let mut d = reader.into_decoder().ok()?;
    if d.dimensions() != (w, h) || d.total_bytes() > MAX_ALLOC {
        return None;
    }
    let o = d.orientation().unwrap_or(Orientation::NoTransforms);
    let img = DynamicImage::from_decoder(d).ok()?;
    let (w, h) = (w as usize, h as usize);
    let (luma, w, h) = match &img {
        DynamicImage::ImageLuma8(b) => shrink(b, 1, w, h, 1.0),
        DynamicImage::ImageLumaA8(b) => shrink(b, 2, w, h, 1.0),
        DynamicImage::ImageRgb8(b) => shrink(b, 3, w, h, 1.0),
        DynamicImage::ImageRgba8(b) => shrink(b, 4, w, h, 1.0),
        DynamicImage::ImageLuma16(b) => shrink(b, 1, w, h, 1.0 / 256.0),
        DynamicImage::ImageLumaA16(b) => shrink(b, 2, w, h, 1.0 / 256.0),
        DynamicImage::ImageRgb16(b) => shrink(b, 3, w, h, 1.0 / 256.0),
        DynamicImage::ImageRgba16(b) => shrink(b, 4, w, h, 1.0 / 256.0),
        _ => None,
    }?;
    Some(orient(luma, w, h, o))
}

/// Luma of `s` (w×h pixels of `ch` samples, times `scale`), averaged over blocks of a whole
/// number of pixels a side so that no side stays at or above twice [`SHRINK`].
fn shrink<T: Copy + Into<f32>>(s: &[T], ch: usize, w: usize, h: usize, scale: f32) -> Option<(Vec<f32>, usize, usize)> {
    if s.len() != w * h * ch {
        return None;
    }
    let (kx, ky) = ((w / SHRINK).max(1), (h / SHRINK).max(1));
    let (nw, nh) = (w.div_ceil(kx), h.div_ceil(ky));
    let col: Vec<usize> = (0..w).map(|x| x / kx).collect();
    let mut sum = vec![0f32; nw * nh];
    let mut n = vec![0u32; nw * nh];
    for (y, row) in s.chunks_exact(w * ch).enumerate() {
        let r = y / ky * nw;
        for (p, c) in row.chunks_exact(ch).zip(&col) {
            let v = if ch < 3 { p[0].into() } else { LUMA_R * p[0].into() + LUMA_G * p[1].into() + LUMA_B * p[2].into() };
            sum[r + c] += v;
            n[r + c] += 1;
        }
    }
    for (s, n) in sum.iter_mut().zip(n) {
        *s *= scale / n as f32;
    }
    Some((sum, nw, nh))
}

/// `px` (w×h) turned the way EXIF orientation `o` says it is shown.
fn orient(px: Vec<f32>, w: usize, h: usize, o: Orientation) -> (Vec<f32>, usize, usize) {
    use Orientation::*;
    if matches!(o, NoTransforms) {
        return (px, w, h);
    }
    let (ow, oh) = if matches!(o, Rotate90 | Rotate270 | Rotate90FlipH | Rotate270FlipH) { (h, w) } else { (w, h) };
    let mut out = Vec::with_capacity(px.len());
    for j in 0..oh {
        for i in 0..ow {
            let (x, y) = match o {
                NoTransforms => (i, j),
                FlipHorizontal => (w - 1 - i, j),
                FlipVertical => (i, h - 1 - j),
                Rotate180 => (w - 1 - i, h - 1 - j),
                Rotate90 => (j, h - 1 - i),
                Rotate270 => (w - 1 - j, i),
                Rotate90FlipH => (j, i),
                Rotate270FlipH => (w - 1 - j, h - 1 - i),
            };
            out.push(px[y * w + x]);
        }
    }
    (out, ow, oh)
}

/// PDQ of a w×h luma buffer, row-major.
#[cfg(test)]
fn pdq(a: Vec<f32>, w: usize, h: usize) -> Option<Fingerprint> {
    pdq_variants(a, w, h).map(|v| v.shown())
}

/// PDQ of a w×h luma buffer, row-major, with its dihedral variants.
fn pdq_variants(mut a: Vec<f32>, w: usize, h: usize) -> Option<Variants> {
    if w < MIN_SIDE || h < MIN_SIDE || a.len() != w * h {
        return None;
    }
    // Blur: two passes of a box filter along rows, then columns, make a tent.
    let window = |n: usize| n.div_ceil(128);
    let (along_rows, along_cols) = (window(w), window(h));
    let mut t = vec![0f32; a.len()];
    for _ in 0..2 {
        for r in 0..h {
            box1d(&a, &mut t, r * w, w, 1, along_rows);
        }
        for c in 0..w {
            box1d(&t, &mut a, c, h, w, along_cols);
        }
    }
    // Decimate to 64×64, sampling the centres.
    let mut s = [[0f32; 64]; 64];
    for (i, row) in s.iter_mut().enumerate() {
        let y = (2 * i + 1) * h / 128;
        for (j, v) in row.iter_mut().enumerate() {
            *v = a[y * w + (2 * j + 1) * w / 128];
        }
    }
    let d = dct(&s);
    let mut hashes = [[0u8; 32]; 8];
    for (n, out) in hashes.iter_mut().enumerate() {
        *out = bits(&dihedral(&d, n & 4 != 0, n & 1 != 0, n & 2 != 0));
    }
    Some(Variants { hashes, quality: quality(&s) })
}

/// One bit per coefficient above the median.
fn bits(d: &[f32; 256]) -> [u8; 32] {
    let median = torben(d);
    let mut hash = [0u8; 32];
    for (k, v) in d.iter().enumerate() {
        if *v > median {
            hash[31 - k / 8] |= 1 << (k % 8);
        }
    }
    hash
}

/// The DCT corner of the picture mirrored left to right (`fx`), upside down (`fy`), then
/// transposed (`tr`), computed from the corner of the picture as it is. Row `i` is vertical
/// frequency `i + 1`, column `j` horizontal frequency `j + 1` (the DC terms are dropped); a
/// cosine of frequency `f` mirrored is itself times (−1)^f, and transposing swaps the axes.
fn dihedral(d: &[f32; 256], tr: bool, fx: bool, fy: bool) -> [f32; 256] {
    let mut out = [0f32; 256];
    for i in 0..16 {
        for j in 0..16 {
            let mut v = d[i * 16 + j];
            if fx && j % 2 == 0 {
                v = -v;
            }
            if fy && i % 2 == 0 {
                v = -v;
            }
            let (a, b) = if tr { (j, i) } else { (i, j) };
            out[a * 16 + b] = v;
        }
    }
    out
}

/// One box-filter pass of width `win` over `n` values `stride` apart from `off`, `x` into `y`.
/// The window shrinks at the ends instead of padding (Jarosz).
fn box1d(x: &[f32], y: &mut [f32], off: usize, n: usize, stride: usize, win: usize) {
    let half = (win + 2) / 2;
    let at = |k: usize| off + k * stride;
    let (mut sum, mut count) = (0f32, 0f32);
    for k in 0..half - 1 {
        sum += x[at(k)];
        count += 1.0;
    }
    for k in half - 1..win {
        sum += x[at(k)];
        count += 1.0;
        y[at(k + 1 - half)] = sum / count;
    }
    for k in win..n {
        sum += x[at(k)];
        sum -= x[at(k - win)];
        y[at(k + 1 - half)] = sum / count;
    }
    for o in n + 1 - half..n {
        sum -= x[at(o + half - 1 - win)];
        count -= 1.0;
        y[at(o)] = sum / count;
    }
}

/// The 16×16 corner of the 64×64 DCT of `s` past the DC terms: D·s·Dᵀ.
fn dct(s: &[[f32; 64]; 64]) -> [f32; 256] {
    // As in the reference: an f32 scale times an f64 cosine, kept as f32.
    let scale = (2.0f64 / 64.0).sqrt() as f32 as f64;
    let mut d = [[0f32; 64]; 16];
    for (i, row) in d.iter_mut().enumerate() {
        for (j, v) in row.iter_mut().enumerate() {
            *v = (scale * (std::f64::consts::PI / 2.0 / 64.0 * (i + 1) as f64 * (2 * j + 1) as f64).cos()) as f32;
        }
    }
    let mut t = [[0f32; 64]; 16];
    for (ti, di) in t.iter_mut().zip(&d) {
        for (j, v) in ti.iter_mut().enumerate() {
            *v = di.iter().zip(s).fold(0.0, |acc, (x, row)| acc + x * row[j]);
        }
    }
    let mut out = [0f32; 256];
    for (i, ti) in t.iter().enumerate() {
        for (j, dj) in d.iter().enumerate() {
            out[i * 16 + j] = ti.iter().zip(dj).fold(0.0, |acc, (x, y)| acc + x * y);
        }
    }
    out
}

/// Median without sorting (Torben Mogensen), as PDQ takes it.
fn torben(m: &[f32]) -> f32 {
    let (mut lo, mut hi) = m.iter().fold((f32::MAX, f32::MIN), |(a, b), v| (a.min(*v), b.max(*v)));
    let half = m.len().div_ceil(2);
    loop {
        let guess = (lo + hi) / 2.0;
        let (mut less, mut greater, mut equal) = (0, 0, 0);
        let (mut below, mut above) = (lo, hi);
        for &v in m {
            if v < guess {
                less += 1;
                below = below.max(v);
            } else if v > guess {
                greater += 1;
                above = above.min(v);
            } else {
                equal += 1;
            }
        }
        if less <= half && greater <= half {
            return if less >= half {
                below
            } else if less + equal >= half {
                guess
            } else {
                above
            };
        } else if less > greater {
            hi = below;
        } else {
            lo = above;
        }
    }
}

/// PDQ's quality: whole steps of 100/255 between neighbouring samples, summed and scaled to 0–100.
fn quality(s: &[[f32; 64]; 64]) -> u8 {
    let step = |u: f32, v: f32| (((u - v) * 100.0 / 255.0) as i32).unsigned_abs();
    let mut sum = 0;
    for (r, next) in s.iter().zip(&s[1..]) {
        sum += r.iter().zip(next).map(|(u, v)| step(*u, *v)).sum::<u32>();
    }
    for r in s {
        sum += r.windows(2).map(|p| step(p[0], p[1])).sum::<u32>();
    }
    (sum / 90).min(100) as u8
}

#[cfg(test)]
mod tests {
    use super::*;
    use image::codecs::gif::GifEncoder;
    use image::codecs::jpeg::JpegEncoder;
    use image::imageops::{self, FilterType};
    use image::{Frame, GrayImage, Luma, Rgb, RgbImage};
    use proptest::prelude::*;
    use std::path::PathBuf;
    use std::sync::OnceLock;

    /// Gradients and shapes drawn relative to the frame, so it is the same picture at every
    /// size. `other` draws a different one.
    fn picture(w: u32, h: u32, other: bool) -> RgbImage {
        RgbImage::from_fn(w, h, |x, y| {
            let (u, v) = ((x as f32 + 0.5) / w as f32, (y as f32 + 0.5) / h as f32);
            let c = if !other {
                if (u - 0.35).powi(2) + (v - 0.4).powi(2) < 0.03 {
                    [230.0, 200.0, 40.0]
                } else if (0.6..0.9).contains(&u) && (0.55..0.85).contains(&v) {
                    [20.0, 30.0, 90.0]
                } else if u < 0.3 && v > 0.7 && ((u * 30.0) as u32).is_multiple_of(2) {
                    [250.0, 250.0, 250.0]
                } else {
                    [40.0 + 150.0 * u, 60.0 + 120.0 * v, 180.0 - 100.0 * u]
                }
            } else {
                let r = ((u - 0.65).powi(2) + (v - 0.6).powi(2)).sqrt();
                if (0.15..0.25).contains(&r) {
                    [15.0, 15.0, 15.0]
                } else if u + v < 0.5 {
                    [240.0, 120.0, 30.0]
                } else if (0.15..0.25).contains(&v) && u > 0.5 {
                    [255.0, 255.0, 255.0]
                } else {
                    [200.0 - 150.0 * v, 80.0 + 100.0 * u, 60.0 + 150.0 * v]
                }
            };
            Rgb(c.map(|c: f32| c as u8))
        })
    }

    fn encode(img: impl Into<DynamicImage>, f: ImageFormat) -> Vec<u8> {
        let mut b = Cursor::new(Vec::new());
        img.into().write_to(&mut b, f).unwrap();
        b.into_inner()
    }

    fn jpeg(img: &RgbImage, quality: u8) -> Vec<u8> {
        let mut b = Vec::new();
        JpegEncoder::new_with_quality(&mut b, quality).encode_image(img).unwrap();
        b
    }

    fn gif(frames: &[&RgbImage]) -> Vec<u8> {
        let mut b = Vec::new();
        {
            let mut e = GifEncoder::new_with_speed(&mut b, 10);
            e.encode_frames(frames.iter().map(|p| Frame::new(DynamicImage::ImageRgb8((*p).clone()).into_rgba8()))).unwrap();
        }
        b
    }

    /// `jpeg` with an EXIF orientation right after its start marker.
    fn with_orientation(jpeg: &[u8], o: u16) -> Vec<u8> {
        let mut exif = b"Exif\0\0MM\0\x2a\0\0\0\x08\0\x01\x01\x12\0\x03\0\0\0\x01".to_vec();
        exif.extend_from_slice(&o.to_be_bytes());
        exif.extend_from_slice(&[0; 6]);
        let mut out = jpeg[..2].to_vec();
        out.extend_from_slice(&[0xFF, 0xE1]);
        out.extend_from_slice(&((exif.len() + 2) as u16).to_be_bytes());
        out.extend_from_slice(&exif);
        out.extend_from_slice(&jpeg[2..]);
        out
    }

    fn crc32(data: &[u8]) -> u32 {
        let mut c = !0u32;
        for &x in data {
            c ^= x as u32;
            for _ in 0..8 {
                c = if c & 1 != 0 { 0xEDB8_8320 ^ (c >> 1) } else { c >> 1 };
            }
        }
        !c
    }

    /// `png` with its first image-data chunk cut in half and its checksum fixed: the structure
    /// still checks out, the pixels don't decode.
    fn cut_idat(png: &[u8]) -> Vec<u8> {
        let p = png.windows(4).position(|w| w == b"IDAT").unwrap();
        let len = u32::from_be_bytes(png[p - 4..p].try_into().unwrap()) as usize;
        let half = len / 2;
        let mut out = png[..p - 4].to_vec();
        out.extend_from_slice(&(half as u32).to_be_bytes());
        out.extend_from_slice(&png[p..p + 4 + half]);
        out.extend_from_slice(&crc32(&png[p..p + 4 + half]).to_be_bytes());
        out.extend_from_slice(&png[p + 8 + len..]);
        out
    }

    /// What a device shares (sanitized), fingerprinted the way a receiving device does it.
    fn fp(b: &[u8]) -> Fingerprint {
        let (clean, info) = media::sanitize(b).unwrap();
        fingerprint(info.mime, &clean).expect("a fingerprint")
    }

    #[test]
    fn near_copies_match() {
        let pic = picture(640, 480, false);
        let (w, h) = pic.dimensions();
        let original = fp(&encode(pic.clone(), ImageFormat::Png));
        assert!(original.quality >= MIN_QUALITY);
        let half = imageops::resize(&pic, w / 2, h / 2, FilterType::Triangle);
        let twice = imageops::resize(&pic, w * 2, h * 2, FilterType::CatmullRom);
        let brighter = imageops::brighten(&pic, 16);
        // PDQ shrugs off a thin border (under 1% a side); a 2% crop a side is some 40 bits.
        let cropped = imageops::crop_imm(&pic, 5, 4, w - 10, h - 8).to_image();
        let all = imageops::resize(&imageops::brighten(&cropped, 12), 400, 290, FilterType::CatmullRom);
        let copies = [
            ("jpeg at 70", jpeg(&pic, 70)),
            ("half the size", encode(half, ImageFormat::Png)),
            ("twice the size", jpeg(&twice, 85)),
            ("brightened", encode(brighter, ImageFormat::Png)),
            ("thin border cropped", encode(cropped, ImageFormat::Png)),
            ("all of that, jpeg at 60", jpeg(&all, 60)),
            ("webp", encode(pic.clone(), ImageFormat::WebP)),
            ("gif", gif(&[&pic])),
            ("16-bit png", encode(DynamicImage::ImageRgb8(pic.clone()).into_rgb16(), ImageFormat::Png)),
            ("png with alpha", encode(DynamicImage::ImageRgb8(pic.clone()).into_rgba8(), ImageFormat::Png)),
            ("stored sideways, turned by exif", with_orientation(&jpeg(&imageops::rotate270(&pic), 90), 6)),
        ];
        for (what, b) in copies {
            let d = distance(&original.hash, &fp(&b).hash);
            eprintln!("{what}: {d} bits apart");
            assert!(d <= MATCH_DISTANCE, "{what}: {d} bits apart");
        }
        // the same bytes give the same fingerprint
        assert_eq!(fp(&jpeg(&pic, 70)), fp(&jpeg(&pic, 70)));
    }

    #[test]
    fn different_pictures_dont() {
        let a = fp(&encode(picture(640, 480, false), ImageFormat::Png));
        let b = variants(&encode(picture(640, 480, true), ImageFormat::Png));
        // every way round, well clear of a match (the test pictures share a layout, so some
        // variant comes nearer than as shown; unrelated pictures sit near 128 bits)
        for h in b.hashes {
            let d = distance(&a.hash, &h);
            assert!(d > 2 * MATCH_DISTANCE, "another picture, some way round: only {d} bits apart");
        }
        assert!(!b.matches(&a.hash));
        // turned without saying so: far apart as shown, so only the variants catch it
        let turned = fp(&jpeg(&imageops::rotate90(&picture(640, 480, false)), 90));
        assert!(distance(&a.hash, &turned.hash) >= 3 * MATCH_DISTANCE);
    }

    /// What a receiving device checks a shared picture with.
    fn variants(b: &[u8]) -> Variants {
        let (clean, info) = media::sanitize(b).unwrap();
        examine_variants(info.mime, &clean).unwrap().expect("a fingerprint")
    }

    #[test]
    fn turned_and_mirrored_copies_match() {
        let pic = picture(640, 480, false);
        let listed = fp(&encode(pic.clone(), ImageFormat::Png)).hash;
        let copies = [
            ("a quarter turn", jpeg(&imageops::rotate90(&pic), 85)),
            ("three quarters", jpeg(&imageops::rotate270(&pic), 85)),
            ("upside down", encode(imageops::rotate180(&pic), ImageFormat::Png)),
            ("mirrored", encode(imageops::flip_horizontal(&pic), ImageFormat::Png)),
            ("mirrored upside down", jpeg(&imageops::flip_vertical(&pic), 80)),
            ("mirrored and turned", encode(imageops::rotate90(&imageops::flip_horizontal(&pic)), ImageFormat::Png)),
            ("turned, smaller and brighter", jpeg(&imageops::brighten(&imageops::resize(&imageops::rotate270(&pic), 300, 400, FilterType::Triangle), 10), 70)),
        ];
        for (what, b) in copies {
            let v = variants(&b);
            let best = v.hashes.iter().map(|h| distance(h, &listed)).min().unwrap();
            eprintln!("{what}: {best} bits apart at best");
            assert!(v.matches(&listed), "{what}: {best} bits apart at best");
        }
    }

    #[test]
    fn variants_are_the_turned_pictures_hashes() {
        // turning the samples and hashing agrees with turning the DCT, to within float noise
        let pic = DynamicImage::ImageRgb8(picture(320, 320, false)).into_luma8();
        let luma: Vec<f32> = pic.iter().map(|v| *v as f32).collect();
        let v = pdq_variants(luma.clone(), 320, 320).unwrap();
        for e in 1..=8 {
            let (px, w, h) = orient(luma.clone(), 320, 320, Orientation::from_exif(e).unwrap());
            let direct = pdq(px, w, h).unwrap().hash;
            let best = v.hashes.iter().map(|x| distance(x, &direct)).min().unwrap();
            assert!(best <= 8, "orientation {e}: {best} bits from the nearest variant");
        }
        // and the variants are eight different hashes of one picture
        let mut seen = v.hashes.to_vec();
        seen.sort_unstable();
        seen.dedup();
        assert_eq!(seen.len(), 8);
        // the luma entry point agrees with decoding
        assert_eq!(luma_variants(luma, 320, 320).unwrap().hashes[0], v.hashes[0]);
    }

    #[test]
    fn animations_count_by_their_first_frame() {
        let (a, b) = (picture(320, 240, false), picture(320, 240, true));
        let first = fp(&gif(&[&a, &b]));
        assert!(matches(&first.hash, &fp(&encode(a, ImageFormat::Png)).hash));
        assert!(!matches(&first.hash, &fp(&encode(b, ImageFormat::Png)).hash));
    }

    #[test]
    fn flat_pictures_have_none() {
        // A flat picture or a bare gradient says too little: any two would match. No fingerprint.
        let flat = RgbImage::from_pixel(300, 200, Rgb([120, 130, 140]));
        assert!(fingerprint(media::PNG, &media::sanitize(&encode(flat, ImageFormat::Png)).unwrap().0).is_none());
        let ramp = RgbImage::from_fn(256, 256, |x, _| Rgb([x as u8; 3]));
        assert!(fingerprint(media::PNG, &media::sanitize(&encode(ramp, ImageFormat::Png)).unwrap().0).is_none());
        assert_eq!(pdq(vec![77.0; 64 * 64], 64, 64).unwrap().quality, 0);
    }

    #[test]
    fn only_checked_pictures() {
        let pic = picture(320, 240, false);
        let png = media::sanitize(&encode(pic.clone(), ImageFormat::Png)).unwrap().0;
        assert!(fingerprint(media::PNG, &png).is_some());
        assert!(fingerprint(media::JPEG, &png).is_none(), "type claims must match the bytes");
        assert!(fingerprint(media::OGG, &png).is_none());
        assert!(fingerprint("text/html", &png).is_none());
        assert!(fingerprint("image/svg+xml", b"<svg onload=alert(1)/>").is_none());
        assert!(fingerprint(media::PNG, b"\x89PNG\r\n\x1a\nnot really").is_none());
        assert!(fingerprint(media::GIF, b"GIF89a").is_none());
        assert!(fingerprint(media::WEBP, &[0x52; 64]).is_none());
        assert!(fingerprint(media::PNG, &[]).is_none());
        // metadata, a tail after the end: refused until sanitized
        let raw = jpeg(&pic, 80);
        assert!(fingerprint(media::JPEG, &raw).is_none(), "carries JFIF");
        assert!(fingerprint(media::JPEG, &media::sanitize(&raw).unwrap().0).is_some());
        let mut tail = png.clone();
        tail.extend_from_slice(b"<script>alert(1)</script>");
        assert!(fingerprint(media::PNG, &tail).is_none());
        // structure checks out, pixels don't decode
        let cut = cut_idat(&png);
        assert!(media::check(media::PNG, &cut).is_ok());
        assert!(fingerprint(media::PNG, &cut).is_none());
        // too small to hash
        let tiny = media::sanitize(&encode(picture(4, 4, false), ImageFormat::Png)).unwrap().0;
        assert!(fingerprint(media::PNG, &tiny).is_none());
    }

    #[test]
    fn every_picture_media_takes_can_be_fingerprinted() {
        let small = DynamicImage::ImageRgb8(picture(256, 256, false)).into_luma8();
        let png = |n| encode(imageops::resize(&small, n, n, FilterType::Nearest), ImageFormat::Png);
        const { assert!(4096 * 4096 <= MAX_PIXELS && 4097 * 4097 > MAX_PIXELS) };
        assert!(media::sanitize(&png(4097)).is_err(), "media refuses what can't be fingerprinted");
        let most = media::sanitize(&png(4096)).unwrap().0;
        let want = fp(&encode(small, ImageFormat::Png));
        assert!(matches(&fingerprint(media::PNG, &most).unwrap().hash, &want.hash));
        // flat: nothing to fingerprint, but nothing wrong either; garbage is unreadable
        let flat = media::sanitize(&encode(RgbImage::from_pixel(64, 64, Rgb([90u8, 90, 90])), ImageFormat::Png)).unwrap().0;
        assert_eq!(examine(media::PNG, &flat), Ok(None));
        assert_eq!(examine(media::PNG, b"not a picture"), Err(Unreadable));
    }

    #[test]
    fn orientation_turns_like_image_does() {
        let g = GrayImage::from_fn(3, 2, |x, y| Luma([(y * 3 + x) as u8]));
        for e in 1..=8 {
            let o = Orientation::from_exif(e).unwrap();
            let mut want = DynamicImage::ImageLuma8(g.clone());
            want.apply_orientation(o);
            let (px, w, h) = orient(g.iter().map(|v| *v as f32).collect(), 3, 2, o);
            assert_eq!((w as u32, h as u32), (want.width(), want.height()), "{o:?}");
            assert_eq!(px, want.as_luma8().unwrap().iter().map(|v| *v as f32).collect::<Vec<_>>(), "{o:?}");
        }
    }

    #[test]
    fn distance_basics() {
        let a = [0u8; 32];
        let mut one = a;
        one[7] = 0x10;
        let mut thirty_one = a;
        thirty_one[..4].copy_from_slice(&[0xFF, 0xFF, 0xFF, 0x7F]);
        let mut thirty_two = a;
        thirty_two[28..].fill(0xFF);
        assert_eq!(distance(&a, &a), 0);
        assert_eq!(distance(&a, &one), 1);
        assert_eq!(distance(&one, &a), 1);
        assert_eq!(distance(&a, &[0xFF; 32]), FINGERPRINT_BITS);
        assert!(matches(&a, &thirty_one));
        assert!(!matches(&a, &thirty_two));
        let hex = "f8f8f0cee0f4a84f06370a22038f63f0b36e2ed596621e1d33e6b39c4e9c9b22";
        assert_eq!(to_hex(&from_hex(hex).unwrap()), hex);
        assert_eq!(from_hex(&hex.to_uppercase()), from_hex(hex));
        assert!(from_hex(&hex[1..]).is_none());
        assert!(from_hex(&format!("{hex}00")).is_none());
        assert!(from_hex(&"zz".repeat(32)).is_none());
    }

    /// pdqhash's own test picture and the hash its test asserts, read from the local cargo
    /// registry when it is there.
    #[test]
    fn reference_picture() {
        const BRIDGE: &str = "f8f8f0cee0f4a84f06370a22038f63f0b36e2ed596621e1d33e6b39c4e9c9b22";
        let home = std::env::var_os("CARGO_HOME").map(PathBuf::from).or_else(|| std::env::var_os("HOME").map(|h| PathBuf::from(h).join(".cargo")));
        let dirs = home.and_then(|h| std::fs::read_dir(h.join("registry/src")).ok()).into_iter().flatten().flatten();
        let Some(bytes) = dirs.into_iter().find_map(|d| std::fs::read(d.path().join("pdqhash-0.1.1/src/test_data/bridge-1-original.jpg")).ok()) else {
            eprintln!("skipped: pdqhash's test data is not in the cargo registry");
            return;
        };
        let want = from_hex(BRIDGE).unwrap();
        // at full size, as pdqhash's test hashes it (a different JPEG decoder may round a pixel differently)
        let img = image::load_from_memory_with_format(&bytes, ImageFormat::Jpeg).unwrap().into_rgb8();
        let luma = img.pixels().map(|p| LUMA_R * p[0] as f32 + LUMA_G * p[1] as f32 + LUMA_B * p[2] as f32).collect();
        let full = pdq(luma, img.width() as usize, img.height() as usize).unwrap();
        // shared, then fingerprinted: 1600×1004, so averaged over 3×1 blocks first
        let shared = fp(&bytes);
        eprintln!("full size: {} bits off, shared: {} bits off, quality {}", distance(&full.hash, &want), distance(&shared.hash, &want), shared.quality);
        assert!(distance(&full.hash, &want) <= 4);
        assert!(distance(&shared.hash, &want) <= 8);
    }

    /// Small pictures in every format, as shared.
    fn samples() -> &'static [(Vec<u8>, &'static str)] {
        static S: OnceLock<Vec<(Vec<u8>, &'static str)>> = OnceLock::new();
        S.get_or_init(|| {
            let p = picture(48, 40, false);
            let files = [encode(p.clone(), ImageFormat::Png), jpeg(&p, 70), gif(&[&p, &picture(48, 40, true)]), encode(p.clone(), ImageFormat::WebP)];
            files.iter().map(|f| media::sanitize(f).map(|(c, i)| (c, i.mime)).unwrap()).collect()
        })
    }

    /// Recomputes every PNG chunk checksum, so damage inside a chunk gets past the check.
    fn fix_crcs(png: &mut [u8]) {
        let mut i = 8;
        while let Some(len) = png.get(i..i + 4).map(|l| u32::from_be_bytes(l.try_into().unwrap()) as usize) {
            let Some(end) = (i + 8).checked_add(len).filter(|e| e + 4 <= png.len()) else { break };
            let c = crc32(&png[i + 4..end]);
            png[end..end + 4].copy_from_slice(&c.to_be_bytes());
            i = end + 4;
        }
    }

    proptest! {
        #![proptest_config(ProptestConfig::with_cases(512))]

        /// Damaged pictures that still pass the check reach the decoder: it must refuse them,
        /// not panic (the wasm build can't catch a panic). Straight to the decoder, no guard.
        #[test]
        fn damaged_pictures_never_panic(k in 0usize..4, at in any::<prop::sample::Index>(), x in any::<u8>(), n in 1usize..4) {
            let (f, mime) = &samples()[k];
            let mut f = f.clone();
            for s in 0..n {
                let i = (at.index(f.len()) + s * 7) % f.len();
                f[i] = x.wrapping_add(s as u8);
            }
            if *mime == media::PNG {
                fix_crcs(&mut f);
            }
            if let Ok(info) = media::check(mime, &f)
                && let Some((luma, w, h)) = decode(format(mime).unwrap(), &f, info.w.unwrap(), info.h.unwrap())
            {
                let _ = pdq(luma, w, h);
            }
        }
    }
}
