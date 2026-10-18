//! The sprite PNG's acceptance check (spec §10): "a picture with no
//! background" is a PNG that can carry transparency and has at least one
//! fully transparent pixel. Native only: the browser tier has no file access.

// TODO(U4a): main.rs loads the sprite; until then the bin crate has no caller.
#![allow(dead_code)]

use std::fmt;
use std::fs;
use std::path::{Path, PathBuf};

use crate::config::SpriteConfig;

/// Why a sprite is not shown. The strings are the product copy the settings
/// window shows and the log line carries.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SpriteError {
    NoPath,
    NotFound(PathBuf),
    NotPng,
    NoAlpha,
    Opaque,
    Undecodable,
}

impl fmt::Display for SpriteError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::NoPath => f.write_str("Choose a PNG file for the sprite"),
            Self::NotFound(path) => write!(f, "Sprite file not found: {}", path.display()),
            Self::NotPng => f.write_str("Sprite is not a PNG file"),
            Self::NoAlpha => {
                f.write_str("Sprite PNG has no alpha channel — it needs a transparent background")
            }
            Self::Opaque => f.write_str(
                "Sprite PNG has no transparent pixels — it needs a transparent background",
            ),
            Self::Undecodable => f.write_str("Sprite PNG could not be decoded"),
        }
    }
}

impl std::error::Error for SpriteError {}

/// What the header says: size, and whether the PNG can carry transparency
/// at all (an alpha channel, or a `tRNS` chunk).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PngInfo {
    pub width: u32,
    pub height: u32,
    pub has_alpha: bool,
}

/// The sprite's pixels, unmultiplied RGBA, and its size in pixels.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DecodedSprite {
    pub rgba: Vec<u8>,
    pub px: [u32; 2],
}

const PNG_SIGNATURE: [u8; 8] = [0x89, 0x50, 0x4e, 0x47, 0x0d, 0x0a, 0x1a, 0x0a];
/// Signature plus the IHDR chunk: the least a PNG can be.
const PNG_HEADER_LEN: usize = 33;

fn u32_at(bytes: &[u8], at: usize) -> u32 {
    u32::from_be_bytes([bytes[at], bytes[at + 1], bytes[at + 2], bytes[at + 3]])
}

fn chunk_type(bytes: &[u8], at: usize) -> &[u8] {
    &bytes[at + 4..at + 8]
}

/// Signature, size and alpha from the 33-byte header, then a chunk walk for
/// a `tRNS` up to the first `IDAT`. No CRC check; a truncated file is simply
/// not a PNG.
pub fn inspect_png(bytes: &[u8]) -> Result<PngInfo, SpriteError> {
    if bytes.len() < PNG_HEADER_LEN || bytes[..8] != PNG_SIGNATURE {
        return Err(SpriteError::NotPng);
    }
    if u32_at(bytes, 8) != 13 || chunk_type(bytes, 8) != b"IHDR" {
        return Err(SpriteError::NotPng);
    }
    let width = u32_at(bytes, 16);
    let height = u32_at(bytes, 20);
    let colour_type = bytes[25];
    let mut has_alpha = colour_type == 4 || colour_type == 6;
    let mut at = PNG_HEADER_LEN;
    while !has_alpha && at + 8 <= bytes.len() {
        let kind = chunk_type(bytes, at);
        has_alpha = kind == b"tRNS";
        if kind == b"IDAT" || kind == b"IEND" {
            break; // tRNS must precede IDAT
        }
        at += 12 + u32_at(bytes, at) as usize;
    }
    Ok(PngInfo {
        width,
        height,
        has_alpha,
    })
}

/// True if any pixel of a 4-bytes-per-pixel bitmap has alpha 0.
pub fn has_transparent_pixel(pixels: &[u8]) -> bool {
    pixels.chunks_exact(4).any(|p| p[3] == 0)
}

/// The check, keeping the pixels: `decode` runs only once the header passes,
/// so a PNG without an alpha channel is refused without being decoded.
fn checked_pixels(
    bytes: &[u8],
    decode: impl FnOnce(&[u8]) -> Option<Vec<u8>>,
) -> Result<DecodedSprite, SpriteError> {
    let info = inspect_png(bytes)?;
    if !info.has_alpha {
        return Err(SpriteError::NoAlpha);
    }
    let rgba = decode(bytes)
        .filter(|p| !p.is_empty())
        .ok_or(SpriteError::Undecodable)?;
    if !has_transparent_pixel(&rgba) {
        return Err(SpriteError::Opaque);
    }
    Ok(DecodedSprite {
        rgba,
        px: [info.width, info.height],
    })
}

/// "A PNG without a background": an alpha channel and at least one fully
/// transparent pixel. `decode` returns a 4-bytes-per-pixel bitmap, or `None`
/// when the image cannot be decoded.
pub fn check_sprite_png(
    bytes: &[u8],
    decode: impl FnOnce(&[u8]) -> Option<Vec<u8>>,
) -> Result<[u32; 2], SpriteError> {
    checked_pixels(bytes, decode).map(|sprite| sprite.px)
}

/// Unmultiplied RGBA; a `tRNS` chunk is expanded into the alpha channel.
pub fn decode_rgba(bytes: &[u8]) -> Option<(Vec<u8>, [u32; 2])> {
    let img = image::load_from_memory_with_format(bytes, image::ImageFormat::Png)
        .ok()?
        .to_rgba8();
    let px = [img.width(), img.height()];
    Some((img.into_raw(), px))
}

fn decode(bytes: &[u8]) -> Option<Vec<u8>> {
    decode_rgba(bytes).map(|(rgba, _)| rgba)
}

fn read_sprite(path: &Path) -> Result<Vec<u8>, SpriteError> {
    if path.as_os_str().is_empty() {
        return Err(SpriteError::NoPath);
    }
    fs::read(path).map_err(|_| SpriteError::NotFound(path.to_path_buf()))
}

/// The check the settings window runs on Save: the size on success, the
/// reason otherwise.
pub fn check_sprite_file(path: &Path) -> Result<[u32; 2], SpriteError> {
    let bytes = read_sprite(path)?;
    check_sprite_png(&bytes, decode)
}

/// The sprite a config asks for: `None` while it is off, otherwise the
/// decoded pixels or the reason it is not shown.
pub fn load_sprite(cfg: &SpriteConfig) -> Option<Result<DecodedSprite, SpriteError>> {
    if !cfg.enabled {
        return None;
    }
    Some(read_sprite(Path::new(&cfg.path)).and_then(|bytes| checked_pixels(&bytes, decode)))
}

#[cfg(test)]
mod tests {
    use super::*;
    use image::{ImageFormat, RgbImage, RgbaImage};
    use std::io::Cursor;

    const GREEN: &[u8] = include_bytes!(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/electron/tests/e2e/fixtures/sprite-green.png"
    ));

    fn rgba_png(pixel: impl Fn(u32) -> [u8; 4]) -> Vec<u8> {
        let img = RgbaImage::from_fn(4, 4, |x, _| image::Rgba(pixel(x)));
        let mut out = Cursor::new(Vec::new());
        img.write_to(&mut out, ImageFormat::Png).unwrap();
        out.into_inner()
    }

    /// A 4×4 RGB PNG — no alpha channel at all.
    fn rgb_png() -> Vec<u8> {
        let img = RgbImage::from_pixel(4, 4, image::Rgb([10, 20, 30]));
        let mut out = Cursor::new(Vec::new());
        img.write_to(&mut out, ImageFormat::Png).unwrap();
        out.into_inner()
    }

    /// A 4×4 RGBA PNG whose every pixel is fully opaque.
    fn opaque_rgba_png() -> Vec<u8> {
        rgba_png(|_| [10, 20, 30, 255])
    }

    /// A 4×4 RGBA PNG with a transparent left half — a valid sprite.
    fn cutout_rgba_png() -> Vec<u8> {
        rgba_png(|x| [10, 20, 30, if x < 2 { 0 } else { 255 }])
    }

    // The hand-rolled encoder mirrors electron/tests/fixtures/png.js: 8-bit,
    // filter 0 on every row, one stored (uncompressed) IDAT, tRNS before IDAT.
    fn crc32(bytes: &[u8]) -> u32 {
        bytes.iter().fold(!0u32, |crc, &b| {
            (0..8).fold(crc ^ u32::from(b), |c, _| {
                if c & 1 == 1 {
                    0xEDB8_8320 ^ (c >> 1)
                } else {
                    c >> 1
                }
            })
        }) ^ !0
    }

    fn chunk(kind: &[u8; 4], data: &[u8]) -> Vec<u8> {
        let mut body = kind.to_vec();
        body.extend_from_slice(data);
        let mut out = (data.len() as u32).to_be_bytes().to_vec();
        out.extend_from_slice(&body);
        out.extend_from_slice(&crc32(&body).to_be_bytes());
        out
    }

    fn zlib_stored(raw: &[u8]) -> Vec<u8> {
        let (a, b) = raw.iter().fold((1u32, 0u32), |(a, b), &x| {
            let a = (a + u32::from(x)) % 65521;
            (a, (b + a) % 65521)
        });
        let len = raw.len() as u16;
        let mut out = vec![0x78, 0x01, 0x01];
        out.extend_from_slice(&len.to_le_bytes());
        out.extend_from_slice(&(!len).to_le_bytes());
        out.extend_from_slice(raw);
        out.extend_from_slice(&((b << 16) | a).to_be_bytes());
        out
    }

    fn encode_png(
        width: u32,
        height: u32,
        colour_type: u8,
        pixel: impl Fn(u32, u32) -> Vec<u8>,
        trns: Option<&[u8]>,
    ) -> Vec<u8> {
        let mut raw = Vec::new();
        for y in 0..height {
            raw.push(0);
            for x in 0..width {
                raw.extend(pixel(x, y));
            }
        }
        let mut ihdr = width.to_be_bytes().to_vec();
        ihdr.extend_from_slice(&height.to_be_bytes());
        ihdr.extend_from_slice(&[8, colour_type, 0, 0, 0]);
        let mut out = vec![0x89, 0x50, 0x4e, 0x47, 0x0d, 0x0a, 0x1a, 0x0a];
        out.extend(chunk(b"IHDR", &ihdr));
        if let Some(t) = trns {
            out.extend(chunk(b"tRNS", t));
        }
        out.extend(chunk(b"IDAT", &zlib_stored(&raw)));
        out.extend(chunk(b"IEND", &[]));
        out
    }

    fn info(bytes: &[u8]) -> (u32, u32, bool) {
        let i = inspect_png(bytes).unwrap();
        (i.width, i.height, i.has_alpha)
    }

    #[test]
    fn inspect_png_reads_size_and_alpha_from_the_green_fixture() {
        assert_eq!(info(GREEN), (720, 430, true));
    }

    #[test]
    fn inspect_png_sees_no_alpha_in_an_rgb_png() {
        assert_eq!(info(&rgb_png()), (4, 4, false));
    }

    #[test]
    fn inspect_png_sees_alpha_in_colour_type_4() {
        let png = encode_png(2, 1, 4, |x, _| vec![9, if x > 0 { 255 } else { 0 }], None);
        assert_eq!(info(&png), (2, 1, true));
    }

    #[test]
    fn inspect_png_sees_alpha_in_an_rgb_png_with_trns() {
        let png = encode_png(2, 1, 2, |_, _| vec![0, 0, 0], Some(&[0, 0, 0, 0, 0, 0]));
        assert_eq!(info(&png), (2, 1, true));
    }

    #[test]
    fn inspect_png_rejects_bytes_without_the_signature() {
        assert_eq!(
            inspect_png(b"<!doctype html><html></html>").unwrap_err(),
            SpriteError::NotPng
        );
    }

    #[test]
    fn inspect_png_rejects_a_truncated_png() {
        assert_eq!(inspect_png(&GREEN[..12]).unwrap_err(), SpriteError::NotPng);
    }

    #[test]
    fn inspect_png_rejects_junk() {
        assert_eq!(inspect_png(&[1, 2, 3]).unwrap_err(), SpriteError::NotPng);
    }

    #[test]
    fn has_transparent_pixel_looks_only_at_the_alpha_byte() {
        assert!(!has_transparent_pixel(&[0, 0, 0, 255, 0, 0, 0, 255]));
        assert!(has_transparent_pixel(&[0, 0, 0, 255, 255, 255, 255, 0]));
        // Semi-transparent is not "background removed".
        assert!(!has_transparent_pixel(&[0, 0, 0, 1]));
    }

    fn decode(bytes: &[u8]) -> Option<Vec<u8>> {
        decode_rgba(bytes).map(|(rgba, _)| rgba)
    }

    #[test]
    fn check_accepts_the_green_fixture() {
        assert_eq!(check_sprite_png(GREEN, decode), Ok([720, 430]));
    }

    #[test]
    fn check_accepts_a_synthetic_cutout() {
        assert_eq!(check_sprite_png(&cutout_rgba_png(), decode), Ok([4, 4]));
    }

    #[test]
    fn check_rejects_an_rgb_png_without_decoding_it() {
        let mut decoded = false;
        let result = check_sprite_png(&rgb_png(), |b| {
            decoded = true;
            decode(b)
        });
        assert_eq!(result, Err(SpriteError::NoAlpha));
        assert!(!decoded);
    }

    #[test]
    fn check_rejects_an_rgba_png_whose_every_pixel_is_opaque() {
        assert_eq!(
            check_sprite_png(&opaque_rgba_png(), decode),
            Err(SpriteError::Opaque)
        );
    }

    #[test]
    fn check_rejects_a_png_the_decoder_cannot_read() {
        assert_eq!(
            check_sprite_png(GREEN, |_| None),
            Err(SpriteError::Undecodable)
        );
    }

    #[test]
    fn check_rejects_a_non_png() {
        assert_eq!(
            check_sprite_png(&[1, 2, 3], decode),
            Err(SpriteError::NotPng)
        );
    }

    #[test]
    fn error_strings_are_the_product_copy() {
        assert_eq!(
            SpriteError::NoPath.to_string(),
            "Choose a PNG file for the sprite"
        );
        assert_eq!(
            SpriteError::NotFound(PathBuf::from("/a/b.png")).to_string(),
            "Sprite file not found: /a/b.png"
        );
        assert_eq!(SpriteError::NotPng.to_string(), "Sprite is not a PNG file");
        assert_eq!(
            SpriteError::NoAlpha.to_string(),
            "Sprite PNG has no alpha channel — it needs a transparent background"
        );
        assert_eq!(
            SpriteError::Opaque.to_string(),
            "Sprite PNG has no transparent pixels — it needs a transparent background"
        );
        assert_eq!(
            SpriteError::Undecodable.to_string(),
            "Sprite PNG could not be decoded"
        );
    }

    #[test]
    fn check_sprite_file_refuses_an_empty_path() {
        assert_eq!(check_sprite_file(Path::new("")), Err(SpriteError::NoPath));
    }

    #[test]
    fn check_sprite_file_names_a_missing_file() {
        let dir = tempfile::tempdir().unwrap();
        let missing = dir.path().join("nope.png");
        assert_eq!(
            check_sprite_file(&missing),
            Err(SpriteError::NotFound(missing.clone()))
        );
    }

    #[test]
    fn check_sprite_file_rejects_an_rgb_file() {
        let dir = tempfile::tempdir().unwrap();
        let file = dir.path().join("rgb.png");
        std::fs::write(&file, rgb_png()).unwrap();
        assert_eq!(check_sprite_file(&file), Err(SpriteError::NoAlpha));
    }

    #[test]
    fn check_sprite_file_accepts_the_green_fixture() {
        let dir = tempfile::tempdir().unwrap();
        let file = dir.path().join("green.png");
        std::fs::write(&file, GREEN).unwrap();
        assert_eq!(check_sprite_file(&file), Ok([720, 430]));
    }

    #[test]
    fn load_sprite_is_none_while_disabled_and_decodes_when_enabled() {
        let dir = tempfile::tempdir().unwrap();
        let file = dir.path().join("green.png");
        std::fs::write(&file, GREEN).unwrap();
        let cfg = SpriteConfig {
            enabled: false,
            path: file.to_string_lossy().into_owned(),
            ..SpriteConfig::default()
        };
        assert!(load_sprite(&cfg).is_none());

        let cfg = SpriteConfig {
            enabled: true,
            ..cfg
        };
        let sprite = load_sprite(&cfg).unwrap().unwrap();
        assert_eq!(sprite.px, [720, 430]);
        assert_eq!(sprite.rgba.len(), 720 * 430 * 4);

        let cfg = SpriteConfig {
            path: String::new(),
            ..cfg
        };
        assert_eq!(load_sprite(&cfg).unwrap().unwrap_err(), SpriteError::NoPath);
    }

    /// Guards against the real sprite being swapped in for the fixture: the
    /// green one is 720×430 with exactly two pixel values.
    #[test]
    fn the_green_fixture_is_a_test_fixture() {
        let (rgba, px) = decode_rgba(GREEN).unwrap();
        assert_eq!(px, [720, 430]);
        let mut seen = std::collections::BTreeSet::new();
        for p in rgba.chunks_exact(4) {
            seen.insert([p[0], p[1], p[2], p[3]]);
        }
        assert_eq!(
            seen.into_iter().collect::<Vec<_>>(),
            vec![[0, 255, 0, 0], [0, 255, 0, 255]]
        );
    }
}
