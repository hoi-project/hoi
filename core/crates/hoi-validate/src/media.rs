//! Media bytes (PROTOCOL.md §17): images, audio and video, checked byte by byte.
//!
//! Two functions matter:
//!
//! - [`sanitize`] runs on the device that shares a file. It sniffs the type from the bytes
//!   (never from a name or a claim), strips everything that isn't the picture or the sound
//!   (EXIF with GPS positions, XMP, comments, thumbnails, embedded profiles, titles, tags,
//!   attachments, trailing data such as a "motion photo" video after a JPEG), and returns the
//!   bytes that get hashed and shared.
//! - [`check`] runs on every device that receives media. It accepts only what [`sanitize`]
//!   would produce: a known container whose structure tiles the file exactly, only the chunks
//!   and boxes on an allow-list, no metadata, nothing after the end marker, sane dimensions.
//!
//! That kills polyglots (a file that is also HTML, a ZIP or a script) and metadata leaks on
//! arrival. It does not decode pixels or samples: browsers and OS decoders do that, sandboxed.
//! SVG and every other scriptable format are not media here and never will be.

pub const PNG: &str = "image/png";
pub const JPEG: &str = "image/jpeg";
pub const GIF: &str = "image/gif";
pub const WEBP: &str = "image/webp";
pub const OGG: &str = "audio/ogg";
pub const MP4: &str = "video/mp4";
pub const WEBM: &str = "video/webm";

pub const MIMES: [&str; 7] = [PNG, JPEG, GIF, WEBP, OGG, MP4, WEBM];

/// Largest side of an image, and its pixel count (decompression bombs; every picture accepted
/// must also be small enough for every device to fingerprint, [`crate::similar`]).
pub const MAX_SIDE: u32 = 16_384;
pub const MAX_PIXELS: u64 = 16_777_216;
/// 16-bit PNGs take twice the memory to decode: half the pixels.
pub const MAX_PIXELS_16: u64 = MAX_PIXELS / 2;

/// Size cap per item, by type.
pub fn max_size(mime: &str) -> Option<u64> {
    match mime {
        PNG | JPEG | GIF | WEBP => Some(2 * 1024 * 1024),
        OGG => Some(8 * 1024 * 1024),
        MP4 | WEBM => Some(16 * 1024 * 1024),
        _ => None,
    }
}

pub fn is_image(mime: &str) -> bool {
    matches!(mime, PNG | JPEG | GIF | WEBP)
}

/// What checking found.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Info {
    pub mime: &'static str,
    pub w: Option<u32>,
    pub h: Option<u32>,
}

type R<T> = Result<T, String>;

fn err<T>(m: &str) -> R<T> {
    Err(m.to_string())
}

/// The type of `b`, from its first bytes.
pub fn sniff(b: &[u8]) -> Option<&'static str> {
    if b.starts_with(b"\x89PNG\r\n\x1a\n") {
        Some(PNG)
    } else if b.starts_with(&[0xFF, 0xD8, 0xFF]) {
        Some(JPEG)
    } else if b.starts_with(b"GIF87a") || b.starts_with(b"GIF89a") {
        Some(GIF)
    } else if b.len() >= 12 && &b[0..4] == b"RIFF" && &b[8..12] == b"WEBP" {
        Some(WEBP)
    } else if b.starts_with(b"OggS") {
        Some(OGG)
    } else if b.len() >= 8 && &b[4..8] == b"ftyp" {
        Some(MP4)
    } else if b.starts_with(&[0x1A, 0x45, 0xDF, 0xA3]) {
        Some(WEBM)
    } else {
        None
    }
}

/// Strict check of received media claimed to be `mime`.
pub fn check(mime: &str, b: &[u8]) -> R<Info> {
    let cap = max_size(mime).ok_or_else(|| format!("{mime} is not a media type hoi carries"))?;
    if b.len() as u64 > cap {
        return err("media is too big");
    }
    if sniff(b) != Some(mime_static(mime)) {
        return err("media bytes don't match their type");
    }
    let info = match mime {
        PNG => png(b, false)?.1,
        JPEG => jpeg(b, false)?.1,
        GIF => gif(b, false)?.1,
        WEBP => webp(b, false)?.1,
        OGG => ogg(b, false)?.1,
        MP4 => mp4(b, false)?.1,
        WEBM => webm(b, false)?.1,
        _ => unreachable!("max_size knows the list"),
    };
    image_dims(&info)?;
    Ok(info)
}

/// Strips metadata from a file someone is about to share, then checks the result.
pub fn sanitize(b: &[u8]) -> R<(Vec<u8>, Info)> {
    let mime = sniff(b).ok_or("hoi shares PNG, JPEG, GIF and WebP images, Ogg Opus audio and MP4 or WebM video")?;
    let (clean, _) = match mime {
        PNG => png(b, true)?,
        JPEG => jpeg(b, true)?,
        GIF => gif(b, true)?,
        WEBP => webp(b, true)?,
        OGG => ogg(b, true)?,
        MP4 => mp4(b, true)?,
        WEBM => webm(b, true)?,
        _ => unreachable!(),
    };
    let info = check(mime, &clean)?;
    Ok((clean, info))
}

fn mime_static(m: &str) -> &'static str {
    MIMES.iter().find(|x| **x == m).copied().unwrap_or("")
}

fn image_dims(i: &Info) -> R<()> {
    if is_image(i.mime) {
        let (w, h) = (i.w.unwrap_or(0), i.h.unwrap_or(0));
        if w == 0 || h == 0 || w > MAX_SIDE || h > MAX_SIDE || w as u64 * h as u64 > MAX_PIXELS {
            return err("image dimensions out of range");
        }
    }
    Ok(())
}

/// `b[start..start + len]`, refusing lengths that would overflow (wasm is 32-bit).
fn span(b: &[u8], start: usize, len: usize) -> R<&[u8]> {
    let end = start.checked_add(len).ok_or("media length overflows")?;
    b.get(start..end).ok_or_else(|| "truncated media".into())
}

fn be16(b: &[u8], i: usize) -> R<u16> {
    b.get(i..i + 2).map(|s| u16::from_be_bytes([s[0], s[1]])).ok_or_else(|| "truncated".into())
}
fn be32(b: &[u8], i: usize) -> R<u32> {
    b.get(i..i + 4).map(|s| u32::from_be_bytes([s[0], s[1], s[2], s[3]])).ok_or_else(|| "truncated".into())
}
fn le16(b: &[u8], i: usize) -> R<u16> {
    b.get(i..i + 2).map(|s| u16::from_le_bytes([s[0], s[1]])).ok_or_else(|| "truncated".into())
}
fn le32(b: &[u8], i: usize) -> R<u32> {
    b.get(i..i + 4).map(|s| u32::from_le_bytes([s[0], s[1], s[2], s[3]])).ok_or_else(|| "truncated".into())
}

// ------------------------------------------------------------------ CRCs

fn crc32_ieee(data: &[u8]) -> u32 {
    let mut c = 0xFFFF_FFFFu32;
    for &x in data {
        c ^= x as u32;
        for _ in 0..8 {
            c = if c & 1 != 0 { 0xEDB8_8320 ^ (c >> 1) } else { c >> 1 };
        }
    }
    !c
}

fn crc32_ogg(data: &[u8]) -> u32 {
    let mut c = 0u32;
    for &x in data {
        c ^= (x as u32) << 24;
        for _ in 0..8 {
            c = if c & 0x8000_0000 != 0 { (c << 1) ^ 0x04C1_1DB7 } else { c << 1 };
        }
    }
    c
}

// ------------------------------------------------------------------ PNG

fn png_chunk(out: &mut Vec<u8>, ty: &[u8], data: &[u8]) {
    out.extend_from_slice(&(data.len() as u32).to_be_bytes());
    out.extend_from_slice(ty);
    out.extend_from_slice(data);
    let mut c = ty.to_vec();
    c.extend_from_slice(data);
    out.extend_from_slice(&crc32_ieee(&c).to_be_bytes());
}

fn png(b: &[u8], fix: bool) -> R<(Vec<u8>, Info)> {
    const KEEP: [&[u8; 4]; 12] = [b"IHDR", b"PLTE", b"IDAT", b"IEND", b"tRNS", b"gAMA", b"cHRM", b"sRGB", b"sBIT", b"bKGD", b"pHYs", b"acTL"];
    const APNG: [&[u8; 4]; 2] = [b"fcTL", b"fdAT"];
    let mut out = b[..8].to_vec();
    let mut i = 8;
    let (mut w, mut h) = (0, 0);
    let mut seen_idat = false;
    let mut idat_done = false;
    let mut first = true;
    loop {
        let len = be32(b, i)? as usize;
        let ty: [u8; 4] = b.get(i + 4..i + 8).ok_or("truncated png")?.try_into().expect("4");
        let data = span(b, i + 8, len)?;
        let crc = be32(b, i + 8 + len)?;
        if crc != crc32_ieee(&b[i + 4..i + 8 + len]) {
            return err("png chunk checksum is wrong");
        }
        i += 12 + len;
        if first {
            if &ty != b"IHDR" || len != 13 {
                return err("png must start with its header");
            }
            w = be32(data, 0)?;
            h = be32(data, 4)?;
            let (depth, color) = (data[8], data[9]);
            let ok = match color {
                0 => matches!(depth, 1 | 2 | 4 | 8 | 16),
                3 => matches!(depth, 1 | 2 | 4 | 8),
                2 | 4 | 6 => matches!(depth, 8 | 16),
                _ => false,
            };
            if !ok || data[10] != 0 || data[11] != 0 || data[12] > 1 {
                return err("png header is invalid");
            }
            if depth == 16 && w as u64 * h as u64 > MAX_PIXELS_16 {
                return err("16-bit png has too many pixels");
            }
            first = false;
        } else if &ty == b"IHDR" {
            return err("png has two headers");
        }
        if &ty == b"IDAT" {
            if idat_done {
                return err("png image data is split up");
            }
            seen_idat = true;
        } else if seen_idat {
            idat_done = true;
        }
        let keep = KEEP.contains(&&ty) || APNG.contains(&&ty);
        if keep {
            png_chunk(&mut out, &ty, data);
        } else if !fix {
            return err(&format!("png carries a {} chunk", String::from_utf8_lossy(&ty)));
        } else if ty[0] & 0x20 == 0 {
            return err("png has an unknown critical chunk");
        }
        if &ty == b"IEND" {
            if len != 0 {
                return err("png end chunk isn't empty");
            }
            if i != b.len() && !fix {
                return err("png has data after its end");
            }
            break;
        }
    }
    if !seen_idat {
        return err("png has no image data");
    }
    Ok((out, Info { mime: PNG, w: Some(w), h: Some(h) }))
}

// ------------------------------------------------------------------ JPEG

/// The only EXIF a shared JPEG may carry: big-endian TIFF, one IFD0 entry, Orientation.
fn exif_orientation_segment(o: u16) -> Vec<u8> {
    let mut p = b"Exif\0\0MM\0\x2a\0\0\0\x08\0\x01".to_vec();
    p.extend_from_slice(&[0x01, 0x12, 0x00, 0x03, 0, 0, 0, 1]);
    p.extend_from_slice(&o.to_be_bytes());
    p.extend_from_slice(&[0, 0, 0, 0, 0, 0]);
    p
}

/// Orientation (1–8) from an EXIF APP1 payload, if it says one.
fn exif_orientation(p: &[u8]) -> Option<u16> {
    let t = p.strip_prefix(b"Exif\0\0")?;
    let be = match t.get(0..2)? {
        b"MM" => true,
        b"II" => false,
        _ => return None,
    };
    let rd16 = |i: usize| t.get(i..i + 2).map(|s| if be { u16::from_be_bytes([s[0], s[1]]) } else { u16::from_le_bytes([s[0], s[1]]) });
    let rd32 = |i: usize| t.get(i..i + 4).map(|s| if be { u32::from_be_bytes([s[0], s[1], s[2], s[3]]) } else { u32::from_le_bytes([s[0], s[1], s[2], s[3]]) });
    let ifd = rd32(4)? as usize;
    let n = rd16(ifd)? as usize;
    for k in 0..n.min(512) {
        let e = ifd + 2 + k * 12;
        if rd16(e)? == 0x0112 {
            let v = rd16(e + 8)?;
            return (1..=8).contains(&v).then_some(v);
        }
    }
    None
}

fn jpeg(b: &[u8], fix: bool) -> R<(Vec<u8>, Info)> {
    let mut out = vec![0xFF, 0xD8];
    let mut i = 2;
    let (mut w, mut h) = (None, None);
    let mut orientation = None;
    let mut wrote_exif = false;
    loop {
        if b.get(i) != Some(&0xFF) {
            return err("jpeg structure is broken");
        }
        while b.get(i + 1) == Some(&0xFF) {
            i += 1;
        }
        let m = *b.get(i + 1).ok_or("truncated jpeg")?;
        i += 2;
        match m {
            0xD9 => {
                out.extend_from_slice(&[0xFF, 0xD9]);
                if i != b.len() && !fix {
                    return err("jpeg has data after its end");
                }
                break;
            }
            0xD8 | 0x01 | 0xD0..=0xD7 => return err("jpeg has a marker out of place"),
            _ => {}
        }
        let len = be16(b, i)? as usize;
        if len < 2 {
            return err("jpeg segment is broken");
        }
        let seg = b.get(i..i + len).ok_or("truncated jpeg")?;
        let payload = &seg[2..];
        i += len;
        let keep = match m {
            0xC0..=0xC3 | 0xC5..=0xC7 | 0xC9..=0xCB | 0xCD..=0xCF => {
                if payload.len() < 6 {
                    return err("jpeg frame header is broken");
                }
                h = Some(be16(payload, 1)? as u32);
                w = Some(be16(payload, 3)? as u32);
                true
            }
            0xC4 | 0xCC | 0xDB | 0xDD | 0xDA | 0xDC => true,
            0xEE => payload.starts_with(b"Adobe") && payload.len() == 12,
            0xE1 if !fix => {
                let o = exif_orientation(payload);
                if payload != exif_orientation_segment(o.unwrap_or(0)).as_slice() || o.is_none() || wrote_exif {
                    return err("jpeg carries metadata");
                }
                wrote_exif = true;
                true
            }
            0xE1 => {
                if let Some(o) = exif_orientation(payload) {
                    orientation.get_or_insert(o);
                }
                false
            }
            _ => false,
        };
        if keep {
            if fix && m == 0xDA && !wrote_exif {
                // Orientation is the one piece of EXIF worth keeping: without it phone photos
                // show sideways. Written once, just before the first scan.
                if let Some(o) = orientation.filter(|o| *o != 1) {
                    let p = exif_orientation_segment(o);
                    out.extend_from_slice(&[0xFF, 0xE1]);
                    out.extend_from_slice(&((p.len() + 2) as u16).to_be_bytes());
                    out.extend_from_slice(&p);
                }
                wrote_exif = true;
            }
            out.extend_from_slice(&[0xFF, m]);
            out.extend_from_slice(seg);
        } else if !fix {
            return err("jpeg carries metadata or an unknown segment");
        }
        if m == 0xDA {
            // entropy-coded data runs until a marker that isn't a restart or a stuffed 0xFF00
            let start = i;
            loop {
                let ff = b[i..].iter().position(|x| *x == 0xFF).map(|p| i + p).ok_or("truncated jpeg scan")?;
                match b.get(ff + 1) {
                    Some(0x00) | Some(0xD0..=0xD7) => i = ff + 2,
                    Some(0xFF) => i = ff + 1,
                    Some(_) => {
                        i = ff;
                        break;
                    }
                    None => return err("truncated jpeg scan"),
                }
            }
            out.extend_from_slice(&b[start..i]);
        }
    }
    if w.is_none() {
        return err("jpeg has no frame");
    }
    Ok((out, Info { mime: JPEG, w, h }))
}

// ------------------------------------------------------------------ GIF

fn gif_subblocks(b: &[u8], mut i: usize) -> R<usize> {
    loop {
        let n = *b.get(i).ok_or("truncated gif")? as usize;
        i += 1 + n;
        if n == 0 {
            return Ok(i);
        }
        if i > b.len() {
            return err("truncated gif");
        }
    }
}

fn gif(b: &[u8], fix: bool) -> R<(Vec<u8>, Info)> {
    if b.len() < 13 {
        return err("truncated gif");
    }
    let w = le16(b, 6)? as u32;
    let h = le16(b, 8)? as u32;
    let packed = b[10];
    let mut i = 13;
    if packed & 0x80 != 0 {
        i += 3 << ((packed & 7) + 1);
    }
    if i > b.len() {
        return err("truncated gif");
    }
    let mut out = b[..i].to_vec();
    let mut frames = 0;
    loop {
        match *b.get(i).ok_or("truncated gif")? {
            0x3B => {
                out.push(0x3B);
                if i + 1 != b.len() && !fix {
                    return err("gif has data after its end");
                }
                break;
            }
            0x2C => {
                frames += 1;
                if frames > 2000 {
                    return err("gif has too many frames");
                }
                let lp = *b.get(i + 9).ok_or("truncated gif")?;
                let mut j = i + 10;
                if lp & 0x80 != 0 {
                    j += 3 << ((lp & 7) + 1);
                }
                let code = *b.get(j).ok_or("truncated gif")?;
                if !(2..=8).contains(&code) {
                    return err("gif image data is invalid");
                }
                let end = gif_subblocks(b, j + 1)?;
                out.extend_from_slice(&b[i..end]);
                i = end;
            }
            0x21 => {
                let label = *b.get(i + 1).ok_or("truncated gif")?;
                let end = gif_subblocks(b, i + 2)?;
                let keep = match label {
                    0xF9 => true,
                    0xFF => {
                        let app = b.get(i + 3..i + 14).unwrap_or(&[]);
                        app == b"NETSCAPE2.0" || app == b"ANIMEXTS1.0"
                    }
                    _ => false,
                };
                if keep {
                    out.extend_from_slice(&b[i..end]);
                } else if !fix {
                    return err("gif carries comments or other extensions");
                }
                i = end;
            }
            _ => return err("gif structure is broken"),
        }
    }
    if frames == 0 {
        return err("gif has no image");
    }
    Ok((out, Info { mime: GIF, w: Some(w), h: Some(h) }))
}

// ------------------------------------------------------------------ WebP

fn webp(b: &[u8], fix: bool) -> R<(Vec<u8>, Info)> {
    let end = (le32(b, 4)? as usize).checked_add(8).ok_or("webp size field is wrong")?;
    if end != b.len() && !(fix && end < b.len()) {
        return err("webp size field is wrong");
    }
    let mut chunks: Vec<([u8; 4], &[u8])> = Vec::new();
    let mut i = 12;
    while i < end {
        let ty: [u8; 4] = b.get(i..i + 4).ok_or("truncated webp")?.try_into().expect("4");
        let len = le32(b, i + 4)? as usize;
        let data = span(b, i + 8, len)?;
        i += 8 + len + (len & 1);
        chunks.push((ty, data));
    }
    if i != end {
        return err("webp chunks don't fill the file");
    }
    let (mut w, mut h) = (None, None);
    let mut out_chunks: Vec<([u8; 4], Vec<u8>)> = Vec::new();
    for (k, (ty, data)) in chunks.iter().enumerate() {
        match ty {
            b"VP8X" => {
                if k != 0 || data.len() != 10 {
                    return err("webp extended header out of place");
                }
                let mut d = data.to_vec();
                if fix {
                    d[0] &= !(0x20 | 0x08 | 0x04); // no ICC, EXIF or XMP
                } else if d[0] & (0x20 | 0x08 | 0x04) != 0 {
                    return err("webp says it carries metadata");
                }
                w = Some(1 + (d[4] as u32 | (d[5] as u32) << 8 | (d[6] as u32) << 16));
                h = Some(1 + (d[7] as u32 | (d[8] as u32) << 8 | (d[9] as u32) << 16));
                out_chunks.push((*ty, d));
            }
            b"VP8 " => {
                if data.len() < 10 || data[3..6] != [0x9D, 0x01, 0x2A] {
                    return err("webp lossy frame is invalid");
                }
                if w.is_none() {
                    w = Some(le16(data, 6)? as u32 & 0x3FFF);
                    h = Some(le16(data, 8)? as u32 & 0x3FFF);
                }
                out_chunks.push((*ty, data.to_vec()));
            }
            b"VP8L" => {
                if data.len() < 5 || data[0] != 0x2F {
                    return err("webp lossless frame is invalid");
                }
                if w.is_none() {
                    let v = le32(data, 1)?;
                    w = Some((v & 0x3FFF) + 1);
                    h = Some(((v >> 14) & 0x3FFF) + 1);
                }
                out_chunks.push((*ty, data.to_vec()));
            }
            b"ALPH" | b"ANIM" | b"ANMF" => out_chunks.push((*ty, data.to_vec())),
            _ if fix => {}
            _ => return err("webp carries metadata or an unknown chunk"),
        }
    }
    if !out_chunks.iter().any(|(t, _)| t == b"VP8 " || t == b"VP8L" || t == b"ANMF") {
        return err("webp has no image");
    }
    let mut body = b"WEBP".to_vec();
    for (ty, d) in &out_chunks {
        body.extend_from_slice(ty);
        body.extend_from_slice(&(d.len() as u32).to_le_bytes());
        body.extend_from_slice(d);
        if d.len() & 1 == 1 {
            body.push(0);
        }
    }
    let mut out = b"RIFF".to_vec();
    out.extend_from_slice(&(body.len() as u32).to_le_bytes());
    out.extend_from_slice(&body);
    Ok((out, Info { mime: WEBP, w, h }))
}

// ------------------------------------------------------------------ MP4

const MP4_TOP: [&[u8; 4]; 10] = [b"ftyp", b"moov", b"mdat", b"free", b"skip", b"wide", b"moof", b"mfra", b"sidx", b"styp"];
const MP4_CONTAINERS: [&[u8; 4]; 11] = [b"moov", b"trak", b"mdia", b"minf", b"stbl", b"dinf", b"edts", b"mvex", b"moof", b"traf", b"mfra"];
const MP4_METADATA: [&[u8; 4]; 3] = [b"udta", b"meta", b"uuid"];

/// Walks boxes in `b[start..end]`. Metadata boxes become `free` with zeroed contents when
/// fixing (sizes and offsets stay put), and fail the check otherwise.
fn mp4_boxes(b: &mut [u8], start: usize, end: usize, depth: usize, fix: bool, count: &mut usize, top: &mut Vec<[u8; 4]>) -> R<()> {
    if depth > 12 {
        return err("mp4 nests too deep");
    }
    let mut i = start;
    while i < end {
        *count += 1;
        if *count > 200_000 {
            return err("mp4 has too many boxes");
        }
        let size32 = be32(b, i)? as usize;
        let ty: [u8; 4] = b.get(i + 4..i + 8).ok_or("truncated mp4")?.try_into().expect("4");
        let (size, header) = match size32 {
            1 => {
                let hi = be32(b, i + 8)? as u64;
                let lo = be32(b, i + 12)? as u64;
                (usize::try_from(hi << 32 | lo).map_err(|_| "mp4 box too large")?, 16)
            }
            0 if depth == 0 => (end - i, 8),
            0 => return err("mp4 box without a size"),
            n => (n, 8),
        };
        if size < header || i.checked_add(size).is_none_or(|e| e > end) {
            return err("mp4 boxes don't fit together");
        }
        if depth == 0 {
            top.push(ty);
        }
        if depth == 0 && !MP4_TOP.contains(&&ty) && !MP4_METADATA.contains(&&ty) {
            if !fix {
                return err(&format!("mp4 carries a {} box", String::from_utf8_lossy(&ty)));
            }
            b[i + 4..i + 8].copy_from_slice(b"free");
            b[i + header..i + size].fill(0);
        } else if MP4_METADATA.contains(&&ty) {
            if !fix {
                return err("mp4 carries metadata");
            }
            b[i + 4..i + 8].copy_from_slice(b"free");
            b[i + header..i + size].fill(0);
        } else if MP4_CONTAINERS.contains(&&ty) {
            mp4_boxes(b, i + header, i + size, depth + 1, fix, count, top)?;
        }
        i += size;
    }
    if i != end {
        return err("mp4 boxes don't fit together");
    }
    Ok(())
}

fn mp4(b: &[u8], fix: bool) -> R<(Vec<u8>, Info)> {
    let mut out = b.to_vec();
    let mut count = 0;
    let mut top = Vec::new();
    let len = out.len();
    mp4_boxes(&mut out, 0, len, 0, fix, &mut count, &mut top)?;
    let has = |t: &[u8; 4]| top.contains(t);
    if top.first() != Some(b"ftyp") || top.iter().filter(|t| *t == b"ftyp").count() != 1 {
        return err("mp4 must start with exactly one type box");
    }
    if !(has(b"moov") && (has(b"mdat") || has(b"moof"))) {
        return err("mp4 has no movie");
    }
    Ok((out, Info { mime: MP4, w: None, h: None }))
}

// ------------------------------------------------------------------ WebM (Matroska)

/// Reads an EBML element id (with its length marker) at `i`.
fn ebml_id(b: &[u8], i: usize) -> R<(u32, usize)> {
    let first = *b.get(i).ok_or("truncated webm")?;
    let n = first.leading_zeros() as usize + 1;
    if n > 4 {
        return err("webm id is invalid");
    }
    let mut v = 0u32;
    for k in 0..n {
        v = v << 8 | *b.get(i + k).ok_or("truncated webm")? as u32;
    }
    Ok((v, n))
}

/// Reads an EBML size at `i`. `None` is the "unknown size" value, which hoi refuses.
fn ebml_size(b: &[u8], i: usize) -> R<(Option<u64>, usize)> {
    let first = *b.get(i).ok_or("truncated webm")?;
    let n = first.leading_zeros() as usize + 1;
    if n > 8 {
        return err("webm size is invalid");
    }
    let mut v = (first as u64) & (0xFF >> n);
    for k in 1..n {
        v = v << 8 | *b.get(i + k).ok_or("truncated webm")? as u64;
    }
    let all_ones = (1u64 << (7 * n)) - 1;
    Ok(((v != all_ones).then_some(v), n))
}

/// Turns the element at `i` (total length `total`) into a zero-filled Void element.
fn ebml_void(b: &mut [u8], i: usize, total: usize) -> R<()> {
    if total < 2 {
        return err("webm element too small to blank");
    }
    let sl = (total - 1).min(8);
    let payload = total - 1 - sl;
    b[i] = 0xEC;
    // size field of `sl` bytes: marker bit then the value
    let mut v = payload as u64 | (1u64 << (7 * sl));
    for k in (0..sl).rev() {
        b[i + 1 + k] = v as u8;
        v >>= 8;
    }
    b[i + 1 + sl..i + total].fill(0);
    Ok(())
}

const SEGMENT: u32 = 0x1853_8067;
const SEG_KEEP: [u32; 7] = [0x114D_9B74, 0x1549_A966, 0x1654_AE6B, 0x1F43_B675, 0x1C53_BB6B, 0xEC, 0xBF];
const INFO: u32 = 0x1549_A966;
const TRACKS: u32 = 0x1654_AE6B;
const TRACK_ENTRY: u32 = 0xAE;
/// Title, DateUTC, MuxingApp, WritingApp, SegmentUUID: who made it, when, with what.
const INFO_DROP: [u32; 5] = [0x7BA9, 0x4461, 0x4D80, 0x5741, 0x73A4];
/// A track's Name.
const TRACK_DROP: [u32; 1] = [0x536E];

/// Walks children of `[start, end)`; `drop` lists ids to blank. Returns child (id, start, end) spans.
fn ebml_children(b: &mut [u8], start: usize, end: usize, drop: &[u32], keep_only: Option<&[u32]>, fix: bool) -> R<Vec<(u32, usize, usize)>> {
    let mut out = Vec::new();
    let mut i = start;
    while i < end {
        let (id, il) = ebml_id(b, i)?;
        let (size, sl) = ebml_size(b, i + il)?;
        let size = size.ok_or("webm element has unknown size")? as usize;
        let body = i + il + sl;
        let next = body.checked_add(size).ok_or("webm element too large")?;
        if next > end {
            return err("webm elements don't fit together");
        }
        let unwanted = drop.contains(&id) || keep_only.is_some_and(|k| !k.contains(&id));
        if unwanted {
            if !fix {
                return err("webm carries metadata or attachments");
            }
            ebml_void(b, i, next - i)?;
        } else {
            out.push((id, body, next));
        }
        i = next;
        if out.len() > 500_000 {
            return err("webm has too many elements");
        }
    }
    if i != end {
        return err("webm elements don't fit together");
    }
    Ok(out)
}

fn webm(b: &[u8], fix: bool) -> R<(Vec<u8>, Info)> {
    let mut out = b.to_vec();
    let len = out.len();
    let top = ebml_children(&mut out, 0, len, &[], Some(&[0x1A45_DFA3, SEGMENT, 0xEC]), fix)?;
    let header = top.first().filter(|(id, ..)| *id == 0x1A45_DFA3).ok_or("webm must start with its header")?;
    let (hs, he) = (header.1, header.2);
    let doctype = ebml_children(&mut out, hs, he, &[], None, false)?.into_iter().find(|(id, ..)| *id == 0x4282).ok_or("webm header has no doctype")?;
    if &out[doctype.1..doctype.2] != b"webm" {
        return err("only webm, not other matroska files");
    }
    let segs: Vec<_> = top.iter().filter(|(id, ..)| *id == SEGMENT).copied().collect();
    if segs.len() != 1 {
        return err("webm needs exactly one segment");
    }
    let (_, ss, se) = segs[0];
    let children = ebml_children(&mut out, ss, se, &[], Some(&SEG_KEEP), fix)?;
    let mut has_tracks = false;
    for (id, s, e) in children {
        match id {
            INFO => {
                ebml_children(&mut out, s, e, &INFO_DROP, None, fix)?;
            }
            TRACKS => {
                has_tracks = true;
                for (tid, ts, te) in ebml_children(&mut out, s, e, &[], None, fix)? {
                    if tid == TRACK_ENTRY {
                        ebml_children(&mut out, ts, te, &TRACK_DROP, None, fix)?;
                    }
                }
            }
            _ => {}
        }
    }
    if !has_tracks {
        return err("webm has no tracks");
    }
    Ok((out, Info { mime: WEBM, w: None, h: None }))
}

// ------------------------------------------------------------------ Ogg Opus

struct Page<'a> {
    header_type: u8,
    granule: u64,
    serial: u32,
    seq: u32,
    segs: &'a [u8],
    data: &'a [u8],
}

fn ogg_pages(b: &[u8]) -> R<Vec<Page<'_>>> {
    let mut pages = Vec::new();
    let mut i = 0;
    while i < b.len() {
        if b.get(i..i + 4) != Some(b"OggS") || b.get(i + 4) != Some(&0) {
            return err("ogg page is broken");
        }
        let n = *b.get(i + 26).ok_or("truncated ogg")? as usize;
        let segs = b.get(i + 27..i + 27 + n).ok_or("truncated ogg")?;
        let dlen: usize = segs.iter().map(|x| *x as usize).sum();
        let end = i + 27 + n + dlen;
        let page = b.get(i..end).ok_or("truncated ogg")?;
        let mut zeroed = page.to_vec();
        zeroed[22..26].fill(0);
        if crc32_ogg(&zeroed) != le32(page, 22)? {
            return err("ogg page checksum is wrong");
        }
        pages.push(Page {
            header_type: page[5],
            granule: u64::from_le_bytes(page[6..14].try_into().expect("8")),
            serial: le32(page, 14)?,
            seq: le32(page, 18)?,
            segs,
            data: &page[27 + n..],
        });
        i = end;
        if pages.len() > 100_000 {
            return err("ogg has too many pages");
        }
    }
    Ok(pages)
}

fn ogg_page(header_type: u8, granule: u64, serial: u32, seq: u32, packet_lacing: &[u8], data: &[u8]) -> Vec<u8> {
    let mut p = b"OggS\0".to_vec();
    p.push(header_type);
    p.extend_from_slice(&granule.to_le_bytes());
    p.extend_from_slice(&serial.to_le_bytes());
    p.extend_from_slice(&seq.to_le_bytes());
    p.extend_from_slice(&[0; 4]);
    p.push(packet_lacing.len() as u8);
    p.extend_from_slice(packet_lacing);
    p.extend_from_slice(data);
    let c = crc32_ogg(&p);
    p[22..26].copy_from_slice(&c.to_le_bytes());
    p
}

fn lacing(len: usize) -> Vec<u8> {
    let mut v = vec![255u8; len / 255];
    v.push((len % 255) as u8);
    v
}

fn ogg(b: &[u8], fix: bool) -> R<(Vec<u8>, Info)> {
    let pages = ogg_pages(b)?;
    let first = pages.first().ok_or("empty ogg")?;
    let serial = first.serial;
    for (k, p) in pages.iter().enumerate() {
        if p.serial != serial {
            return err("ogg holds more than one stream");
        }
        if p.seq as usize != k {
            return err("ogg pages are out of order");
        }
        if (k == 0) != (p.header_type & 0x02 != 0) {
            return err("ogg stream start is wrong");
        }
    }
    if pages.last().is_some_and(|p| p.header_type & 0x04 == 0) {
        return err("ogg stream doesn't end");
    }
    // page 0: exactly the OpusHead packet
    let head = first.data;
    if first.segs.last() == Some(&255) || head.len() < 19 || !head.starts_with(b"OpusHead") || head[8] > 15 || head[9] == 0 {
        return err("not an opus stream");
    }
    // the OpusTags packet starts on page 1 and may span pages
    let mut tags = Vec::new();
    let mut k = 1;
    let mut rest_start = None;
    while let Some(p) = pages.get(k) {
        if k > 1 && p.header_type & 0x01 == 0 {
            return err("opus tags are broken");
        }
        // the tags packet must end exactly at a page end
        let mut off = 0;
        for (si, s) in p.segs.iter().enumerate() {
            off += *s as usize;
            if *s < 255 && si + 1 != p.segs.len() {
                return err("opus tags share a page with audio");
            }
        }
        tags.extend_from_slice(&p.data[..off]);
        k += 1;
        if p.segs.last().is_some_and(|s| *s < 255) {
            rest_start = Some(k);
            break;
        }
    }
    let rest_start = rest_start.ok_or("opus stream has no tags")?;
    if !tags.starts_with(b"OpusTags") {
        return err("opus tags are missing");
    }
    let vlen = le32(&tags, 8)? as usize;
    let vendor = tags.get(12..12 + vlen).ok_or("opus tags are broken")?;
    let comments = le32(&tags, 12 + vlen)?;
    let clean = vlen <= 64 && vendor.iter().all(|c| (0x20..0x7F).contains(c)) && comments == 0 && tags.len() == 16 + vlen;
    if !fix {
        if !clean {
            return err("opus carries tags");
        }
        return Ok((b.to_vec(), Info { mime: OGG, w: None, h: None }));
    }
    let mut new_tags = b"OpusTags".to_vec();
    new_tags.extend_from_slice(&3u32.to_le_bytes());
    new_tags.extend_from_slice(b"hoi");
    new_tags.extend_from_slice(&0u32.to_le_bytes());
    let mut out = Vec::with_capacity(b.len());
    out.extend_from_slice(&ogg_page(first.header_type, first.granule, serial, 0, first.segs, first.data));
    out.extend_from_slice(&ogg_page(0, 0, serial, 1, &lacing(new_tags.len()), &new_tags));
    for (n, p) in pages[rest_start..].iter().enumerate() {
        out.extend_from_slice(&ogg_page(p.header_type, p.granule, serial, 2 + n as u32, p.segs, p.data));
    }
    Ok((out, Info { mime: OGG, w: None, h: None }))
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;
    use proptest::prelude::*;

    pub fn tiny_png(extra: &[(&[u8; 4], &[u8])]) -> Vec<u8> {
        let mut b = b"\x89PNG\r\n\x1a\n".to_vec();
        let mut ihdr = Vec::new();
        ihdr.extend_from_slice(&1u32.to_be_bytes());
        ihdr.extend_from_slice(&1u32.to_be_bytes());
        ihdr.extend_from_slice(&[8, 2, 0, 0, 0]);
        png_chunk(&mut b, b"IHDR", &ihdr);
        for (t, d) in extra {
            png_chunk(&mut b, *t, d);
        }
        png_chunk(&mut b, b"IDAT", &[0x78, 0x9c, 0x63, 0xf8, 0xcf, 0xc0, 0, 0, 3, 1, 1, 0]);
        png_chunk(&mut b, b"IEND", &[]);
        b
    }

    fn tiny_jpeg(exif: Option<Vec<u8>>, comment: bool, trailer: &[u8]) -> Vec<u8> {
        let mut b = vec![0xFF, 0xD8];
        let seg = |b: &mut Vec<u8>, m: u8, p: &[u8]| {
            b.extend_from_slice(&[0xFF, m]);
            b.extend_from_slice(&((p.len() + 2) as u16).to_be_bytes());
            b.extend_from_slice(p);
        };
        seg(&mut b, 0xE0, b"JFIF\0\x01\x01\0\0\x01\0\x01\0\0");
        if let Some(e) = exif {
            seg(&mut b, 0xE1, &e);
        }
        if comment {
            seg(&mut b, 0xFE, b"shot at 52.37N 4.89E");
        }
        seg(&mut b, 0xDB, &[0; 65]);
        seg(&mut b, 0xC0, &[8, 0, 2, 0, 3, 1, 1, 0x11, 0]);
        seg(&mut b, 0xC4, &[0; 17]);
        seg(&mut b, 0xDA, &[1, 1, 0, 0, 0x3F, 0]);
        b.extend_from_slice(&[0x12, 0xFF, 0x00, 0x34, 0xFF, 0xD0, 0x56]);
        b.extend_from_slice(&[0xFF, 0xD9]);
        b.extend_from_slice(trailer);
        b
    }

    fn le_exif_with_gps(orientation: u16) -> Vec<u8> {
        // little-endian TIFF, IFD0 with Orientation and a GPS pointer
        let mut p = b"Exif\0\0II\x2a\0\x08\0\0\0".to_vec();
        p.extend_from_slice(&2u16.to_le_bytes());
        p.extend_from_slice(&[0x12, 0x01, 3, 0, 1, 0, 0, 0]);
        p.extend_from_slice(&orientation.to_le_bytes());
        p.extend_from_slice(&[0, 0]);
        p.extend_from_slice(&[0x25, 0x88, 4, 0, 1, 0, 0, 0, 0x26, 0, 0, 0]);
        p.extend_from_slice(&[0; 4]);
        p
    }

    #[test]
    fn png_metadata_is_stripped() {
        let clean = tiny_png(&[]);
        assert_eq!(check(PNG, &clean).unwrap(), Info { mime: PNG, w: Some(1), h: Some(1) });
        let dirty = tiny_png(&[(b"tEXt", b"GPS\x0052.37,4.89"), (b"eXIf", b"MM"), (b"iCCP", b"x\0\0")]);
        assert!(check(PNG, &dirty).is_err());
        let (fixed, info) = sanitize(&dirty).unwrap();
        assert_eq!(fixed, clean);
        assert_eq!(info.w, Some(1));
        let mut trailing = clean.clone();
        trailing.extend_from_slice(b"<html><script>alert(1)</script>");
        assert!(check(PNG, &trailing).is_err(), "polyglot tail");
        assert_eq!(sanitize(&trailing).unwrap().0, clean);
        let mut broken = clean.clone();
        broken[20] ^= 1;
        assert!(check(PNG, &broken).is_err(), "checksum");
    }

    #[test]
    fn jpeg_keeps_orientation_only() {
        let dirty = tiny_jpeg(Some(le_exif_with_gps(6)), true, b"\0\0ftypmp42 motion photo");
        assert!(check(JPEG, &dirty).is_err());
        let (fixed, info) = sanitize(&dirty).unwrap();
        assert_eq!((info.w, info.h), (Some(3), Some(2)));
        assert!(!fixed.windows(4).any(|w| w == b"JFIF" || w == b"ftyp"));
        assert!(!fixed.windows(5).any(|w| w == b"52.37"));
        assert_eq!(exif_orientation(&exif_orientation_segment(6)), Some(6));
        assert!(fixed.windows(exif_orientation_segment(6).len()).any(|w| w == exif_orientation_segment(6).as_slice()));
        check(JPEG, &fixed).unwrap();
        // upright photos keep no EXIF at all
        let (upright, _) = sanitize(&tiny_jpeg(Some(le_exif_with_gps(1)), false, b"")).unwrap();
        assert!(!upright.windows(4).any(|w| w == b"Exif"));
    }

    fn tiny_gif(comment: bool) -> Vec<u8> {
        let mut b = b"GIF89a".to_vec();
        b.extend_from_slice(&[2, 0, 2, 0, 0x80, 0, 0]);
        b.extend_from_slice(&[0, 0, 0, 255, 255, 255]);
        if comment {
            b.extend_from_slice(&[0x21, 0xFE, 5]);
            b.extend_from_slice(b"hello");
            b.push(0);
        }
        b.extend_from_slice(&[0x21, 0xF9, 4, 0, 0, 0, 0, 0]);
        b.extend_from_slice(&[0x2C, 0, 0, 0, 0, 2, 0, 2, 0, 0, 2, 2, 0x44, 0x01, 0]);
        b.push(0x3B);
        b
    }

    #[test]
    fn gif_comments_go() {
        let clean = tiny_gif(false);
        assert_eq!(check(GIF, &clean).unwrap().w, Some(2));
        assert!(check(GIF, &tiny_gif(true)).is_err());
        assert_eq!(sanitize(&tiny_gif(true)).unwrap().0, clean);
    }

    fn tiny_webp(exif: bool) -> Vec<u8> {
        let mut body = b"WEBP".to_vec();
        let chunk = |body: &mut Vec<u8>, t: &[u8], d: &[u8]| {
            body.extend_from_slice(t);
            body.extend_from_slice(&(d.len() as u32).to_le_bytes());
            body.extend_from_slice(d);
            if d.len() % 2 == 1 {
                body.push(0);
            }
        };
        if exif {
            chunk(&mut body, b"VP8X", &[0x08, 0, 0, 0, 9, 0, 0, 4, 0, 0]);
        }
        let mut v = vec![0x2F];
        v.extend_from_slice(&((9u32) | (4 << 14)).to_le_bytes());
        chunk(&mut body, b"VP8L", &v);
        if exif {
            chunk(&mut body, b"EXIF", b"MM\0*gps");
        }
        let mut b = b"RIFF".to_vec();
        b.extend_from_slice(&(body.len() as u32).to_le_bytes());
        b.extend_from_slice(&body);
        b
    }

    #[test]
    fn webp_exif_goes() {
        assert_eq!(check(WEBP, &tiny_webp(false)).unwrap(), Info { mime: WEBP, w: Some(10), h: Some(5) });
        assert!(check(WEBP, &tiny_webp(true)).is_err());
        let (fixed, info) = sanitize(&tiny_webp(true)).unwrap();
        assert_eq!((info.w, info.h), (Some(10), Some(5)));
        assert!(!fixed.windows(4).any(|w| w == b"EXIF"));
    }

    fn mp4_box(t: &[u8; 4], body: &[u8]) -> Vec<u8> {
        let mut b = ((body.len() + 8) as u32).to_be_bytes().to_vec();
        b.extend_from_slice(t);
        b.extend_from_slice(body);
        b
    }

    fn tiny_mp4(meta: bool) -> Vec<u8> {
        let mut trak = mp4_box(b"tkhd", &[0; 84]);
        trak.extend(mp4_box(b"mdia", &mp4_box(b"mdhd", &[0; 24])));
        let mut moov = mp4_box(b"mvhd", &[0; 100]);
        moov.extend(mp4_box(b"trak", &trak));
        if meta {
            moov.extend(mp4_box(b"udta", &mp4_box(b"\xa9xyz", b"+52.37+004.89/")));
        }
        let mut f = mp4_box(b"ftyp", b"isom\0\0\x02\0isomiso2mp41");
        f.extend(mp4_box(b"moov", &moov));
        f.extend(mp4_box(b"mdat", &[1, 2, 3, 4]));
        f
    }

    #[test]
    fn mp4_location_goes() {
        check(MP4, &tiny_mp4(false)).unwrap();
        let dirty = tiny_mp4(true);
        assert!(check(MP4, &dirty).is_err());
        let (fixed, _) = sanitize(&dirty).unwrap();
        assert_eq!(fixed.len(), dirty.len(), "offsets stay put");
        assert!(!fixed.windows(5).any(|w| w == b"52.37"));
        let mut tail = tiny_mp4(false);
        tail.extend_from_slice(b"PK\x03\x04zip");
        assert!(check(MP4, &tail).is_err(), "trailing zip");
    }

    fn ebml(id: &[u8], body: &[u8]) -> Vec<u8> {
        let mut b = id.to_vec();
        b.push(0x01); // 8-byte size: marker in the first byte, then 7 more
        b.extend_from_slice(&(body.len() as u64).to_be_bytes()[1..]);
        b.extend_from_slice(body);
        b
    }

    fn tiny_webm(title: bool) -> Vec<u8> {
        let header = ebml(&[0x1A, 0x45, 0xDF, 0xA3], &ebml(&[0x42, 0x82], b"webm"));
        let mut info = ebml(&[0x2A, 0xD7, 0xB1], &[0x0F, 0x42, 0x40]);
        if title {
            info.extend(ebml(&[0x7B, 0xA9], b"my holiday at home"));
        }
        let mut seg = ebml(&[0x15, 0x49, 0xA9, 0x66], &info);
        seg.extend(ebml(&[0x16, 0x54, 0xAE, 0x6B], &ebml(&[0xAE], &ebml(&[0xD7], &[1]))));
        seg.extend(ebml(&[0x1F, 0x43, 0xB6, 0x75], &[0xE7, 0x81, 0x00]));
        let mut f = header;
        f.extend(ebml(&[0x18, 0x53, 0x80, 0x67], &seg));
        f
    }

    #[test]
    fn webm_title_goes() {
        check(WEBM, &tiny_webm(false)).unwrap();
        let dirty = tiny_webm(true);
        assert!(check(WEBM, &dirty).is_err());
        let (fixed, _) = sanitize(&dirty).unwrap();
        assert!(!fixed.windows(7).any(|w| w == b"holiday"));
        assert_eq!(fixed.len(), dirty.len());
    }

    fn tiny_opus(comment: &[u8]) -> Vec<u8> {
        let mut head = b"OpusHead".to_vec();
        head.extend_from_slice(&[1, 2, 0x38, 1, 0x80, 0xBB, 0, 0, 0, 0, 0]);
        let mut tags = b"OpusTags".to_vec();
        tags.extend_from_slice(&7u32.to_le_bytes());
        tags.extend_from_slice(b"libopus");
        if comment.is_empty() {
            tags.extend_from_slice(&0u32.to_le_bytes());
        } else {
            tags.extend_from_slice(&1u32.to_le_bytes());
            tags.extend_from_slice(&(comment.len() as u32).to_le_bytes());
            tags.extend_from_slice(comment);
        }
        let mut f = ogg_page(0x02, 0, 7, 0, &lacing(head.len()), &head);
        f.extend(ogg_page(0, 0, 7, 1, &lacing(tags.len()), &tags));
        f.extend(ogg_page(0x04, 960, 7, 2, &lacing(3), &[0xFC, 0xFF, 0xFE]));
        f
    }

    #[test]
    fn opus_tags_go() {
        check(OGG, &tiny_opus(b"")).unwrap();
        let big = [b"METADATA_BLOCK_PICTURE=".as_slice(), &[7u8; 600]].concat();
        for c in [b"ARTIST=me, at my address".as_slice(), &big] {
            let dirty = tiny_opus(c);
            assert!(check(OGG, &dirty).is_err());
            let (fixed, _) = sanitize(&dirty).unwrap();
            assert!(!fixed.windows(6).any(|w| w == b"ARTIST" || w == b"METADA"));
            check(OGG, &fixed).unwrap();
        }
    }

    #[test]
    fn scriptable_and_unknown_formats_are_refused() {
        assert!(sanitize(b"<svg onload=alert(1)>").is_err());
        assert!(sanitize(b"%PDF-1.7").is_err());
        assert!(check("image/svg+xml", b"<svg/>").is_err());
        assert!(check(PNG, &tiny_gif(false)).is_err(), "type claims must match the bytes");
    }

    proptest! {
        #[test]
        fn never_panics(b in proptest::collection::vec(any::<u8>(), 0..300), k in 0usize..7) {
            let mut b = b;
            // give the fuzzer a head start past each magic
            let magic: [&[u8]; 7] = [b"\x89PNG\r\n\x1a\n", &[0xFF, 0xD8, 0xFF], b"GIF89a", b"RIFF\0\0\0\0WEBP", b"OggS\0", b"\0\0\0\x10ftyp", &[0x1A, 0x45, 0xDF, 0xA3]];
            let mut v = magic[k].to_vec();
            v.append(&mut b);
            let _ = sanitize(&v);
            for m in MIMES {
                let _ = check(m, &v);
            }
        }

        #[test]
        fn mutated_files_never_panic(i in 0usize..400, x in any::<u8>(), k in 0usize..6) {
            let files = [tiny_png(&[]), tiny_jpeg(Some(le_exif_with_gps(6)), true, b""), tiny_gif(true), tiny_webp(true), tiny_mp4(true), tiny_opus(b"A=b")];
            let mut f = files[k].clone();
            let n = i % f.len();
            f[n] = x;
            if let Ok((clean, info)) = sanitize(&f) {
                prop_assert!(check(info.mime, &clean).is_ok());
            }
        }
    }
}
