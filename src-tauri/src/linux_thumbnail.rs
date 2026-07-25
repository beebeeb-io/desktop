#![cfg(target_os = "linux")]

use std::ffi::OsStr;
use std::fs;
use std::io::Cursor;
use std::os::unix::ffi::OsStrExt;
use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};

use image::imageops::FilterType;

const NORMAL_MAX_DIMENSION: u32 = 128;
const LARGE_MAX_DIMENSION: u32 = 256;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum FreedesktopThumbnailSize {
    Normal,
    Large,
}

impl FreedesktopThumbnailSize {
    #[cfg_attr(not(test), allow(dead_code))]
    pub(crate) fn for_max_dimension(max_dimension: u32) -> Option<Self> {
        match max_dimension {
            1..=NORMAL_MAX_DIMENSION => Some(Self::Normal),
            129..=LARGE_MAX_DIMENSION => Some(Self::Large),
            _ => None,
        }
    }

    fn dirname(self) -> &'static str {
        match self {
            Self::Normal => "normal",
            Self::Large => "large",
        }
    }

    fn max_dimension(self) -> u32 {
        match self {
            Self::Normal => NORMAL_MAX_DIMENSION,
            Self::Large => LARGE_MAX_DIMENSION,
        }
    }
}

pub(crate) fn source_path_under_sync_root(sync_root: &Path, rel_path: &str) -> Option<PathBuf> {
    let rel = rel_path.trim_matches('/');
    if rel.is_empty() {
        return None;
    }
    crate::reject_unsafe_rel_path(rel).ok()?;
    Some(sync_root.join(rel.replace('/', std::path::MAIN_SEPARATOR_STR)))
}

pub(crate) fn write_freedesktop_thumbnails(
    source_path: &Path,
    source_mtime_secs: i64,
    image_bytes: &[u8],
) -> anyhow::Result<Vec<PathBuf>> {
    let Some(cache_home) = thumbnail_cache_home() else {
        anyhow::bail!("could not resolve XDG cache home for thumbnail cache");
    };

    let mut written = Vec::new();
    for size in [FreedesktopThumbnailSize::Normal, FreedesktopThumbnailSize::Large] {
        written.push(write_freedesktop_thumbnail(
            &cache_home,
            source_path,
            source_mtime_secs,
            image_bytes,
            size,
        )?);
    }
    Ok(written)
}

pub(crate) fn remove_freedesktop_thumbnails(source_path: &Path) -> anyhow::Result<()> {
    let Some(cache_home) = thumbnail_cache_home() else {
        return Ok(());
    };

    for size in [FreedesktopThumbnailSize::Normal, FreedesktopThumbnailSize::Large] {
        let path = thumbnail_cache_path(&cache_home, source_path, size);
        match fs::remove_file(&path) {
            Ok(()) => {}
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
            Err(error) => return Err(anyhow::anyhow!("remove thumbnail {}: {error}", path.display())),
        }
    }
    Ok(())
}

fn thumbnail_cache_home() -> Option<PathBuf> {
    match std::env::var_os("XDG_CACHE_HOME") {
        Some(value) if !value.is_empty() => Some(PathBuf::from(value)),
        _ => std::env::var_os("HOME").map(|home| PathBuf::from(home).join(".cache")),
    }
}

fn write_freedesktop_thumbnail(
    cache_home: &Path,
    source_path: &Path,
    source_mtime_secs: i64,
    image_bytes: &[u8],
    size: FreedesktopThumbnailSize,
) -> anyhow::Result<PathBuf> {
    let png = encode_freedesktop_png(image_bytes, source_path, source_mtime_secs, size)?;
    let path = thumbnail_cache_path(cache_home, source_path, size);
    let parent = path
        .parent()
        .ok_or_else(|| anyhow::anyhow!("thumbnail path has no parent: {}", path.display()))?;
    fs::create_dir_all(parent).map_err(|e| anyhow::anyhow!("create thumbnail dir {}: {e}", parent.display()))?;
    fs::set_permissions(parent, fs::Permissions::from_mode(0o700))
        .map_err(|e| anyhow::anyhow!("chmod thumbnail dir {}: {e}", parent.display()))?;

    let hash = path.file_stem().and_then(OsStr::to_str).unwrap_or("thumbnail");
    let tmp = parent.join(format!(".beebeeb-{}-{hash}.tmp", std::process::id()));
    fs::write(&tmp, png).map_err(|e| anyhow::anyhow!("write thumbnail temp {}: {e}", tmp.display()))?;
    fs::set_permissions(&tmp, fs::Permissions::from_mode(0o600))
        .map_err(|e| anyhow::anyhow!("chmod thumbnail temp {}: {e}", tmp.display()))?;
    fs::rename(&tmp, &path)
        .map_err(|e| anyhow::anyhow!("rename thumbnail {} -> {}: {e}", tmp.display(), path.display()))?;
    fs::set_permissions(&path, fs::Permissions::from_mode(0o600))
        .map_err(|e| anyhow::anyhow!("chmod thumbnail {}: {e}", path.display()))?;
    Ok(path)
}

fn thumbnail_cache_path(cache_home: &Path, source_path: &Path, size: FreedesktopThumbnailSize) -> PathBuf {
    let uri = file_uri(source_path);
    let hash = md5_hex(uri.as_bytes());
    cache_home
        .join("thumbnails")
        .join(size.dirname())
        .join(format!("{hash}.png"))
}

fn encode_freedesktop_png(
    image_bytes: &[u8],
    source_path: &Path,
    source_mtime_secs: i64,
    size: FreedesktopThumbnailSize,
) -> anyhow::Result<Vec<u8>> {
    let image = image::load_from_memory(image_bytes).map_err(|e| anyhow::anyhow!("decode thumbnail source: {e}"))?;
    let rgba = image.to_rgba8();
    let (pixels, width, height) = resize_rgba_to_fit(rgba, size.max_dimension())?;

    let mut output = Vec::new();
    {
        let mut encoder = png::Encoder::new(Cursor::new(&mut output), width, height);
        encoder.set_color(png::ColorType::Rgba);
        encoder.set_depth(png::BitDepth::Eight);
        encoder.add_text_chunk("Thumb::URI".to_string(), file_uri(source_path))?;
        encoder.add_text_chunk("Thumb::MTime".to_string(), source_mtime_secs.max(0).to_string())?;
        encoder.add_text_chunk("Software".to_string(), "Beebeeb Desktop".to_string())?;
        let mut writer = encoder.write_header()?;
        writer.write_image_data(&pixels)?;
    }
    Ok(output)
}

fn resize_rgba_to_fit(rgba: image::RgbaImage, max_dimension: u32) -> anyhow::Result<(Vec<u8>, u32, u32)> {
    let width = rgba.width();
    let height = rgba.height();
    if width == 0 || height == 0 {
        anyhow::bail!("thumbnail source has zero dimensions");
    }

    let source_max = width.max(height);
    if source_max <= max_dimension {
        return Ok((rgba.into_raw(), width, height));
    }

    let resized_width = ((width as u64 * max_dimension as u64) / source_max as u64).max(1) as u32;
    let resized_height = ((height as u64 * max_dimension as u64) / source_max as u64).max(1) as u32;
    let resized = image::imageops::resize(&rgba, resized_width, resized_height, FilterType::Lanczos3);
    Ok((resized.into_raw(), resized_width, resized_height))
}

fn file_uri(path: &Path) -> String {
    let path = if path.is_absolute() {
        path.to_path_buf()
    } else {
        std::env::current_dir()
            .unwrap_or_else(|_| PathBuf::from("/"))
            .join(path)
    };

    let mut uri = String::from("file://");
    for &byte in path.as_os_str().as_bytes() {
        match byte {
            b'/' => uri.push('/'),
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'!' | b'~' | b'*' | b'\'' | b'(' | b')' => {
                uri.push(byte as char)
            }
            _ => {
                const HEX: &[u8; 16] = b"0123456789ABCDEF";
                uri.push('%');
                uri.push(HEX[(byte >> 4) as usize] as char);
                uri.push(HEX[(byte & 0x0f) as usize] as char);
            }
        }
    }
    uri
}

fn md5_hex(input: &[u8]) -> String {
    const S: [u32; 64] = [
        7, 12, 17, 22, 7, 12, 17, 22, 7, 12, 17, 22, 7, 12, 17, 22, 5, 9, 14, 20, 5, 9, 14, 20, 5, 9, 14, 20, 5, 9, 14,
        20, 4, 11, 16, 23, 4, 11, 16, 23, 4, 11, 16, 23, 4, 11, 16, 23, 6, 10, 15, 21, 6, 10, 15, 21, 6, 10, 15, 21, 6,
        10, 15, 21,
    ];
    const K: [u32; 64] = [
        0xd76aa478, 0xe8c7b756, 0x242070db, 0xc1bdceee, 0xf57c0faf, 0x4787c62a, 0xa8304613, 0xfd469501, 0x698098d8,
        0x8b44f7af, 0xffff5bb1, 0x895cd7be, 0x6b901122, 0xfd987193, 0xa679438e, 0x49b40821, 0xf61e2562, 0xc040b340,
        0x265e5a51, 0xe9b6c7aa, 0xd62f105d, 0x02441453, 0xd8a1e681, 0xe7d3fbc8, 0x21e1cde6, 0xc33707d6, 0xf4d50d87,
        0x455a14ed, 0xa9e3e905, 0xfcefa3f8, 0x676f02d9, 0x8d2a4c8a, 0xfffa3942, 0x8771f681, 0x6d9d6122, 0xfde5380c,
        0xa4beea44, 0x4bdecfa9, 0xf6bb4b60, 0xbebfbc70, 0x289b7ec6, 0xeaa127fa, 0xd4ef3085, 0x04881d05, 0xd9d4d039,
        0xe6db99e5, 0x1fa27cf8, 0xc4ac5665, 0xf4292244, 0x432aff97, 0xab9423a7, 0xfc93a039, 0x655b59c3, 0x8f0ccc92,
        0xffeff47d, 0x85845dd1, 0x6fa87e4f, 0xfe2ce6e0, 0xa3014314, 0x4e0811a1, 0xf7537e82, 0xbd3af235, 0x2ad7d2bb,
        0xeb86d391,
    ];

    let bit_len = (input.len() as u64) * 8;
    let mut msg = input.to_vec();
    msg.push(0x80);
    while msg.len() % 64 != 56 {
        msg.push(0);
    }
    msg.extend_from_slice(&bit_len.to_le_bytes());

    let mut a0 = 0x67452301u32;
    let mut b0 = 0xefcdab89u32;
    let mut c0 = 0x98badcfeu32;
    let mut d0 = 0x10325476u32;

    for chunk in msg.chunks_exact(64) {
        let mut m = [0u32; 16];
        for (i, word) in m.iter_mut().enumerate() {
            let offset = i * 4;
            *word = u32::from_le_bytes([chunk[offset], chunk[offset + 1], chunk[offset + 2], chunk[offset + 3]]);
        }

        let mut a = a0;
        let mut b = b0;
        let mut c = c0;
        let mut d = d0;

        for i in 0..64 {
            let (f, g) = if i < 16 {
                ((b & c) | (!b & d), i)
            } else if i < 32 {
                ((d & b) | (!d & c), (5 * i + 1) % 16)
            } else if i < 48 {
                (b ^ c ^ d, (3 * i + 5) % 16)
            } else {
                (c ^ (b | !d), (7 * i) % 16)
            };

            let next = a.wrapping_add(f).wrapping_add(K[i]).wrapping_add(m[g]);
            a = d;
            d = c;
            c = b;
            b = b.wrapping_add(next.rotate_left(S[i]));
        }

        a0 = a0.wrapping_add(a);
        b0 = b0.wrapping_add(b);
        c0 = c0.wrapping_add(c);
        d0 = d0.wrapping_add(d);
    }

    let mut digest = [0u8; 16];
    digest[0..4].copy_from_slice(&a0.to_le_bytes());
    digest[4..8].copy_from_slice(&b0.to_le_bytes());
    digest[8..12].copy_from_slice(&c0.to_le_bytes());
    digest[12..16].copy_from_slice(&d0.to_le_bytes());

    let mut out = String::with_capacity(32);
    for byte in digest {
        use std::fmt::Write;
        let _ = write!(out, "{byte:02x}");
    }
    out
}

#[cfg(test)]
mod tests {
    use std::io::Cursor;
    use std::path::Path;

    use image::{ImageBuffer, Rgba};

    use super::*;

    fn fixture_png(width: u32, height: u32) -> Vec<u8> {
        let image = ImageBuffer::from_fn(width, height, |x, y| {
            Rgba([(x % 251) as u8, (y % 241) as u8, ((x + y) % 239) as u8, 255])
        });
        let mut png = Cursor::new(Vec::new());
        image::DynamicImage::ImageRgba8(image)
            .write_to(&mut png, image::ImageFormat::Png)
            .unwrap();
        png.into_inner()
    }

    #[test]
    fn thumbnail_cache_path_hashes_canonical_file_uri_with_md5() {
        let cache_home = Path::new("/home/jens/.cache");
        let source = Path::new("/home/jens/photos/me.png");

        let path = thumbnail_cache_path(cache_home, source, FreedesktopThumbnailSize::Normal);

        assert_eq!(
            path,
            cache_home
                .join("thumbnails")
                .join("normal")
                .join("c6ee772d9e49320e97ec29a7eb5b1697.png")
        );
    }

    #[test]
    fn thumbnail_size_selection_uses_freedesktop_thresholds() {
        assert_eq!(
            FreedesktopThumbnailSize::for_max_dimension(128),
            Some(FreedesktopThumbnailSize::Normal)
        );
        assert_eq!(
            FreedesktopThumbnailSize::for_max_dimension(129),
            Some(FreedesktopThumbnailSize::Large)
        );
        assert_eq!(
            FreedesktopThumbnailSize::for_max_dimension(256),
            Some(FreedesktopThumbnailSize::Large)
        );
        assert_eq!(FreedesktopThumbnailSize::for_max_dimension(257), None);
    }

    #[test]
    fn png_output_is_rgba_resized_and_embeds_uri_and_mtime_text_chunks() {
        let source = Path::new("/home/jens/photos/me.png");
        let input = fixture_png(320, 160);

        let output = encode_freedesktop_png(&input, source, 1_700_000_123, FreedesktopThumbnailSize::Normal)
            .expect("encode thumbnail png");

        let decoder = png::Decoder::new(Cursor::new(output));
        let reader = decoder.read_info().expect("read png info");
        let info = reader.info();
        assert_eq!(info.width, 128);
        assert_eq!(info.height, 64);
        assert_eq!(info.color_type, png::ColorType::Rgba);
        assert_eq!(info.bit_depth, png::BitDepth::Eight);

        let mtime = info
            .uncompressed_latin1_text
            .iter()
            .find(|chunk| chunk.keyword == "Thumb::MTime")
            .map(|chunk| chunk.text.clone());
        assert_eq!(mtime.as_deref(), Some("1700000123"));

        let uri = info
            .uncompressed_latin1_text
            .iter()
            .find(|chunk| chunk.keyword == "Thumb::URI")
            .map(|chunk| chunk.text.clone());
        assert_eq!(uri.as_deref(), Some("file:///home/jens/photos/me.png"));
    }
}
