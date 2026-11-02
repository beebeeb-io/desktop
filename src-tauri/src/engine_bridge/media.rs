//! `media` items split out of `engine_bridge.rs`.

use super::*;

pub(super) const THUMBNAIL_VARIANTS_FOR_UPLOAD: [ThumbnailUploadVariant; 2] =
    [ThumbnailUploadVariant::Medium, ThumbnailUploadVariant::Large];

pub(super) const MEDIUM_ENCRYPTED_THUMBNAIL_MAX_BYTES: usize = 128 * 1024;

pub(super) const LARGE_ENCRYPTED_THUMBNAIL_MAX_BYTES: usize = 512 * 1024;

pub(super) const BLURHASH_COMPONENTS_X: usize = 4;

pub(super) const BLURHASH_COMPONENTS_Y: usize = 3;

pub(super) const BLURHASH_SOURCE_MAX_DIMENSION: u32 = 64;

pub(super) const BLURHASH_BASE83: &[u8; 83] =
    b"0123456789ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz#$%*+,-.:;=?@[]^_{|}~";

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum ThumbnailUploadVariant {
    Medium,
    Large,
}

impl ThumbnailUploadVariant {
    pub(super) fn label(self) -> &'static str {
        match self {
            Self::Medium => "medium",
            Self::Large => "large",
        }
    }

    pub(super) fn config(self) -> beebeeb_core::thumbnail::ThumbnailConfig {
        match self {
            Self::Medium => beebeeb_core::thumbnail::ThumbnailConfig::medium(),
            Self::Large => beebeeb_core::thumbnail::ThumbnailConfig::large(),
        }
    }

    pub(super) fn encrypted_max_bytes(self) -> usize {
        match self {
            Self::Medium => MEDIUM_ENCRYPTED_THUMBNAIL_MAX_BYTES,
            Self::Large => LARGE_ENCRYPTED_THUMBNAIL_MAX_BYTES,
        }
    }
}

#[derive(Debug)]
pub(super) struct ThumbnailSource {
    pub(super) rgba: Zeroizing<Vec<u8>>,
    pub(super) width: u32,
    pub(super) height: u32,
    pub(super) is_video: bool,
}

/// Why a thumbnail, its blurhash or a thumbnail-carried base64 value could not be
/// built. Each variant spells the step's text exactly as the `anyhow!` it replaces
/// did, so a log line that carried the failure reads the same.
#[derive(Debug)]
pub(super) enum MediaError {
    EncryptThumbnail { variant: &'static str, detail: String },
    /// Opening or reading the file the image comes from.
    Io(std::io::Error),
    DecodeImage(image::ImageError),
    NoVideoFrame,
    SpawnFfmpeg(std::io::Error),
    FfmpegFailed(String),
    DecodeFrame(image::ImageError),
    ZeroDimensions,
    ResizeBlurhash(String),
    Base64 { label: String, detail: String },
}

impl std::fmt::Display for MediaError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            MediaError::EncryptThumbnail { variant, detail } => {
                write!(f, "encrypt thumbnail {variant}: {detail}")
            }
            MediaError::Io(error) => error.fmt(f),
            MediaError::DecodeImage(error) => write!(f, "decode image for thumbnail: {error}"),
            MediaError::NoVideoFrame => f.write_str("video thumbnail extraction failed"),
            MediaError::SpawnFfmpeg(error) => write!(f, "spawn ffmpeg for video thumbnail: {error}"),
            MediaError::FfmpegFailed(snippet) => write!(f, "ffmpeg video thumbnail failed: {snippet}"),
            MediaError::DecodeFrame(error) => write!(f, "decode ffmpeg thumbnail frame: {error}"),
            MediaError::ZeroDimensions => f.write_str("thumbnail source has zero dimensions"),
            MediaError::ResizeBlurhash(detail) => write!(f, "resize blurhash source: {detail}"),
            MediaError::Base64 { label, detail } => {
                write!(f, "{label} is not valid standard base64: {detail}")
            }
        }
    }
}

impl std::error::Error for MediaError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            MediaError::Io(error) | MediaError::SpawnFfmpeg(error) => Some(error),
            MediaError::DecodeImage(error) | MediaError::DecodeFrame(error) => Some(error),
            _ => None,
        }
    }
}

impl From<std::io::Error> for MediaError {
    fn from(error: std::io::Error) -> Self {
        MediaError::Io(error)
    }
}


#[derive(Debug)]
pub(super) struct PreparedThumbnailUpload {
    pub(super) variant: ThumbnailUploadVariant,
    pub(super) encrypted: Zeroizing<Vec<u8>>,
    pub(super) blurhash: Option<String>,
}

#[cfg(test)]
pub(super) fn prepare_thumbnail_uploads_for_plaintext_media(
    payload_path: &Path,
    mime_type: Option<&str>,
    file_key: &beebeeb_core::kdf::FileKey,
) -> Result<Vec<PreparedThumbnailUpload>, MediaError> {
    prepare_thumbnail_uploads_with_pin(payload_path, mime_type, file_key, beebeeb_core::chunk_stream::ExpectedChunkFormat::Unrecorded)
}

pub(super) fn prepare_thumbnail_uploads_with_pin(payload_path: &Path, mime_type: Option<&str>, file_key: &beebeeb_core::kdf::FileKey,
    pin: beebeeb_core::chunk_stream::ExpectedChunkFormat) -> Result<Vec<PreparedThumbnailUpload>, MediaError> {
    if !beebeeb_core::media::is_media(mime_type) {
        return Ok(Vec::new());
    }

    let source = decode_thumbnail_source(payload_path, mime_type)?;
    let blurhash = if source.is_video {
        None
    } else {
        blurhash_for_source(&source).ok().flatten()
    };

    let mut uploads = Vec::new();
    for variant in THUMBNAIL_VARIANTS_FOR_UPLOAD {
        let config = variant.config();
        let output =
            match beebeeb_core::thumbnail::generate_thumbnail(&source.rgba, source.width, source.height, &config) {
                Ok(output) => output,
                Err(e) => {
                    tracing::warn!(
                        variant = variant.label(),
                        error = %e,
                        "thumbnail generation skipped for variant"
                    );
                    continue;
                }
            };
        let plaintext = Zeroizing::new(output.data);
        let encrypted = Zeroizing::new(
            beebeeb_core::encrypt::encrypt_thumbnail_for(file_key, &plaintext, pin)
                .map_err(|e| MediaError::EncryptThumbnail {
                    variant: variant.label(),
                    detail: e.to_string(),
                })?,
        );
        if encrypted.len() > variant.encrypted_max_bytes() {
            tracing::warn!(
                variant = variant.label(),
                bytes = encrypted.len(),
                max_bytes = variant.encrypted_max_bytes(),
                "thumbnail generation skipped because encrypted payload is too large"
            );
            continue;
        }
        uploads.push(PreparedThumbnailUpload {
            variant,
            encrypted,
            blurhash: if variant == ThumbnailUploadVariant::Medium {
                blurhash.clone()
            } else {
                None
            },
        });
    }

    Ok(uploads)
}

pub(super) fn decode_thumbnail_source(payload_path: &Path, mime_type: Option<&str>) -> Result<ThumbnailSource, MediaError> {
    match mime_type {
        Some(mime) if mime.starts_with("video/") => decode_video_thumbnail_source(payload_path),
        _ => decode_image_thumbnail_source(payload_path),
    }
}

pub(super) fn decode_image_thumbnail_source(payload_path: &Path) -> Result<ThumbnailSource, MediaError> {
    let image = image::ImageReader::open(payload_path)?
        .with_guessed_format()?
        .decode()
        .map_err(MediaError::DecodeImage)?;
    dynamic_image_to_thumbnail_source(image, false)
}

pub(super) fn decode_video_thumbnail_source(payload_path: &Path) -> Result<ThumbnailSource, MediaError> {
    let mut last_error: Option<MediaError> = None;
    for seek in ["1", "0"] {
        match decode_video_frame_with_ffmpeg(payload_path, seek) {
            Ok(source) => return Ok(source),
            Err(e) => last_error = Some(e),
        }
    }
    Err(last_error.unwrap_or(MediaError::NoVideoFrame))
}

pub(super) fn decode_video_frame_with_ffmpeg(payload_path: &Path, seek: &str) -> Result<ThumbnailSource, MediaError> {
    let ffmpeg = std::env::var_os("BEEBEEB_FFMPEG_PATH").unwrap_or_else(|| "ffmpeg".into());
    let output = Command::new(ffmpeg)
        .arg("-nostdin")
        .arg("-hide_banner")
        .arg("-loglevel")
        .arg("error")
        .arg("-ss")
        .arg(seek)
        .arg("-i")
        .arg(payload_path)
        .arg("-frames:v")
        .arg("1")
        .arg("-f")
        .arg("image2pipe")
        .arg("-vcodec")
        .arg("png")
        .arg("-")
        .output()
        .map_err(MediaError::SpawnFfmpeg)?;

    if !output.status.success() || output.stdout.is_empty() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        let snippet: String = stderr.chars().take(240).collect();
        return Err(MediaError::FfmpegFailed(snippet));
    }

    let frame_png = Zeroizing::new(output.stdout);
    let image =
        image::load_from_memory(&frame_png).map_err(MediaError::DecodeFrame)?;
    dynamic_image_to_thumbnail_source(image, true)
}

pub(super) fn dynamic_image_to_thumbnail_source(image: image::DynamicImage, is_video: bool) -> Result<ThumbnailSource, MediaError> {
    let rgba = image.into_rgba8();
    let width = rgba.width();
    let height = rgba.height();
    if width == 0 || height == 0 {
        return Err(MediaError::ZeroDimensions);
    }
    Ok(ThumbnailSource {
        rgba: Zeroizing::new(rgba.into_raw()),
        width,
        height,
        is_video,
    })
}

pub(super) fn blurhash_for_source(source: &ThumbnailSource) -> Result<Option<String>, MediaError> {
    let (small_rgba, width, height) = beebeeb_core::thumbnail::resize_for_thumbnail(
        &source.rgba,
        source.width,
        source.height,
        BLURHASH_SOURCE_MAX_DIMENSION,
    )
    .map_err(|e| MediaError::ResizeBlurhash(e.to_string()))?;
    let small_rgba = Zeroizing::new(small_rgba);
    Ok(encode_blurhash_rgba(
        &small_rgba,
        width,
        height,
        BLURHASH_COMPONENTS_X,
        BLURHASH_COMPONENTS_Y,
    ))
}

pub(super) fn encode_blurhash_rgba(
    rgba: &[u8],
    width: u32,
    height: u32,
    components_x: usize,
    components_y: usize,
) -> Option<String> {
    if width == 0
        || height == 0
        || components_x == 0
        || components_x > 9
        || components_y == 0
        || components_y > 9
        || rgba.len() != width as usize * height as usize * 4
    {
        return None;
    }

    let mut factors = Vec::with_capacity(components_x * components_y);
    for y in 0..components_y {
        for x in 0..components_x {
            factors.push(multiply_blurhash_basis(rgba, width, height, x, y));
        }
    }

    let size_flag = (components_x - 1) + (components_y - 1) * 9;
    let mut encoded = String::with_capacity(4 + 2 * factors.len());
    encoded.push_str(&encode_base83(size_flag as u32, 1));

    let maximum_value = if factors.len() > 1 {
        let actual_max = factors[1..]
            .iter()
            .flat_map(|factor| factor.iter())
            .fold(0.0_f64, |max, value| max.max(value.abs()));
        let quantized = ((actual_max * 166.0 - 0.5).floor() as i32).clamp(0, 82) as u32;
        encoded.push_str(&encode_base83(quantized, 1));
        (quantized + 1) as f64 / 166.0
    } else {
        encoded.push_str(&encode_base83(0, 1));
        1.0
    };

    encoded.push_str(&encode_base83(encode_blurhash_dc(factors[0]), 4));
    for factor in factors.iter().skip(1) {
        encoded.push_str(&encode_base83(encode_blurhash_ac(*factor, maximum_value), 2));
    }

    if encoded.len() <= 64 { Some(encoded) } else { None }
}

pub(super) fn multiply_blurhash_basis(rgba: &[u8], width: u32, height: u32, component_x: usize, component_y: usize) -> [f64; 3] {
    let normalisation = if component_x == 0 && component_y == 0 { 1.0 } else { 2.0 };
    let width_f = width as f64;
    let height_f = height as f64;
    let mut r = 0.0;
    let mut g = 0.0;
    let mut b = 0.0;

    for y in 0..height {
        for x in 0..width {
            let basis = (std::f64::consts::PI * component_x as f64 * x as f64 / width_f).cos()
                * (std::f64::consts::PI * component_y as f64 * y as f64 / height_f).cos();
            let idx = ((y * width + x) as usize) * 4;
            let alpha = rgba[idx + 3] as f64 / 255.0;
            let sr = (rgba[idx] as f64 / 255.0) * alpha + (1.0 - alpha);
            let sg = (rgba[idx + 1] as f64 / 255.0) * alpha + (1.0 - alpha);
            let sb = (rgba[idx + 2] as f64 / 255.0) * alpha + (1.0 - alpha);
            r += basis * srgb_to_linear(sr);
            g += basis * srgb_to_linear(sg);
            b += basis * srgb_to_linear(sb);
        }
    }

    let scale = normalisation / (width_f * height_f);
    [r * scale, g * scale, b * scale]
}

pub(super) fn srgb_to_linear(value: f64) -> f64 {
    if value <= 0.04045 {
        value / 12.92
    } else {
        ((value + 0.055) / 1.055).powf(2.4)
    }
}

pub(super) fn linear_to_srgb(value: f64) -> u32 {
    let value = value.clamp(0.0, 1.0);
    let srgb = if value <= 0.0031308 {
        value * 12.92
    } else {
        1.055 * value.powf(1.0 / 2.4) - 0.055
    };
    (srgb * 255.0 + 0.5).floor().clamp(0.0, 255.0) as u32
}

pub(super) fn encode_blurhash_dc(value: [f64; 3]) -> u32 {
    (linear_to_srgb(value[0]) << 16) + (linear_to_srgb(value[1]) << 8) + linear_to_srgb(value[2])
}

pub(super) fn encode_blurhash_ac(value: [f64; 3], maximum_value: f64) -> u32 {
    let quant_r = quantize_blurhash_ac(value[0], maximum_value);
    let quant_g = quantize_blurhash_ac(value[1], maximum_value);
    let quant_b = quantize_blurhash_ac(value[2], maximum_value);
    quant_r * 19 * 19 + quant_g * 19 + quant_b
}

pub(super) fn quantize_blurhash_ac(value: f64, maximum_value: f64) -> u32 {
    let normalized = if maximum_value > 0.0 {
        value / maximum_value
    } else {
        0.0
    };
    (sign_pow(normalized.clamp(-1.0, 1.0), 0.5) * 9.0 + 9.5)
        .floor()
        .clamp(0.0, 18.0) as u32
}

pub(super) fn encode_base83(mut value: u32, length: usize) -> String {
    let mut chars = vec![0u8; length];
    for i in (0..length).rev() {
        chars[i] = BLURHASH_BASE83[(value % 83) as usize];
        value /= 83;
    }
    String::from_utf8(chars).expect("base83 alphabet is ASCII")
}

#[cfg(target_os = "linux")]
pub(super) fn linux_thumbnail_source_path_for_entry(entry: &FileEntry) -> Option<PathBuf> {
    let sync_root = crate::config::DesktopConfig::load().ok()?.sync_root?;
    crate::linux_thumbnail::source_path_under_sync_root(&sync_root, &entry.path)
}

pub(super) fn decode_standard_b64(label: &str, value: &str) -> Result<Vec<u8>, MediaError> {
    base64::engine::general_purpose::STANDARD
        .decode(value)
        .map_err(|e| MediaError::Base64 {
            label: label.to_string(),
            detail: e.to_string(),
        })
}
