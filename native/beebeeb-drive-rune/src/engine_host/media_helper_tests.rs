//! 095b: a HEIC and a DNG decoded end to end by the REAL helper, confined the way the app confines it
//! (`media::transport::SandboxedPictures`: the system-decoder profile, the helper's limits, `--image` mode), and read
//! back over the same pipes code the app uses ([`super::decode_over`]). Windows only: WIC is the decoder.
//!
//! The helper is `BB_PICTURE_HELPER` when set, else built here from `native/beebeeb-media-helper` (its own workspace)
//! into a target dir beside this test's. The app's global helper is never registered: the other tests of this binary
//! keep their "no helper" world.
//!
//! A PC without the codec answers `NoDecoder` (the honest refusal, never a blank picture). With
//! `BB_IMAGE_EXTENSIONS=1` (VM 401: HEIF, HEVC and Raw Image extensions installed) both must decode to pixels.

use std::path::PathBuf;
use std::sync::OnceLock;

use beebeeb_office_sandbox::{Mode, Profile, Sandbox};

use super::{DecodedPicture, HelperPicture, ImageRefusal, decode_over, looks_like_heif};
use crate::media::transport::HELPER_LIMITS;

/// 64 × 32, left half red, right half blue (made with libheif's `heif-enc` from a generated PNG; no photo).
const HEIC: &[u8] = include_bytes!("../../tests/fixtures/pictures/red-blue-64x32.heic");

fn strict() -> bool {
    std::env::var("BB_IMAGE_EXTENSIONS").is_ok_and(|v| v == "1")
}

/// The helper exe: `BB_PICTURE_HELPER`, else built once from its own workspace.
fn helper() -> PathBuf {
    static HELPER: OnceLock<PathBuf> = OnceLock::new();
    HELPER
        .get_or_init(|| {
            if let Some(path) = std::env::var_os("BB_PICTURE_HELPER") {
                return PathBuf::from(path);
            }
            // This test runs from `<target>/<profile>/deps/`.
            let exe = std::env::current_exe().expect("the test's own path");
            let target = exe.ancestors().nth(3).expect("a target dir").join("picture-helper");
            let manifest = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../beebeeb-media-helper/Cargo.toml");
            let cargo = std::env::var_os("CARGO").unwrap_or_else(|| "cargo".into());
            let built = std::process::Command::new(cargo)
                .args(["build", "--locked", "--quiet", "--manifest-path"])
                .arg(&manifest)
                .arg("--target-dir")
                .arg(&target)
                .status()
                .expect("cargo starts");
            assert!(built.success(), "the media helper builds");
            let path = target.join("debug").join("beebeeb-media-helper.exe");
            assert!(path.is_file(), "the helper is at {}", path.display());
            path
        })
        .clone()
}

/// One decode through the confined helper, as `SandboxedPictures` starts it.
fn decode(bytes: &[u8], max_px: u32) -> Result<DecodedPicture, ImageRefusal> {
    let sandbox = Sandbox::with_profile(
        HELPER_LIMITS,
        Profile::SystemDecoder {
            hevc: looks_like_heif(bytes),
        },
    )
    .expect("the sandbox is made");
    let mut child = sandbox
        .spawn_in_mode(&helper(), Mode::Image)
        .expect("the confined helper starts");
    let pipes = child.take_pipes().expect("the helper's pipes");
    let answer = decode_over(Box::new(pipes.from_helper), Box::new(pipes.to_helper), bytes, max_px);
    let _ = child.kill();
    answer
}

fn pixel(picture: &DecodedPicture, x: u32, y: u32) -> [u8; 4] {
    let at = ((y * picture.width + x) * 4) as usize;
    picture.rgba[at..at + 4].try_into().unwrap()
}

#[test]
fn heif_fixture_decodes_through_the_confined_helper_or_says_no_decoder() {
    assert_eq!(HelperPicture::of_bytes(HEIC), Some(HelperPicture::Heif));
    assert_eq!(HelperPicture::of_name("red-blue.heif"), Some(HelperPicture::Heif));
    let answer = decode(HEIC, 32);
    eprintln!("heic: {:?}", answer.as_ref().map(|p| (p.width, p.height)));
    match answer {
        Ok(picture) => {
            assert_eq!((picture.width, picture.height), (32, 16));
            let (left, right) = (pixel(&picture, 3, 8), pixel(&picture, 28, 8));
            assert!(left[0] > 150 && left[2] < 100 && left[3] == 255, "{left:?}");
            assert!(right[2] > 150 && right[0] < 100, "{right:?}");
        }
        Err(why) => {
            assert!(!strict(), "the HEIF and HEVC extensions are installed: {why:?}");
            assert_eq!(why, ImageRefusal::NoDecoder);
        }
    }
}

#[test]
fn raw_dng_fixture_decodes_upright_through_the_confined_helper_or_says_no_decoder() {
    // Stored 48 × 32 landscape, orientation 6: shown portrait.
    let dng = dng(48, 32, 6);
    assert_eq!(HelperPicture::of_bytes(&dng), Some(HelperPicture::Raw));
    for name in ["a.dng", "a.CR2", "a.nef", "a.arw"] {
        assert_eq!(HelperPicture::of_name(name), Some(HelperPicture::Raw), "{name}");
    }
    let answer = decode(&dng, 64);
    eprintln!("dng: {:?}", answer.as_ref().map(|p| (p.width, p.height)));
    match answer {
        Ok(picture) => {
            assert!(picture.width < picture.height, "{}x{}", picture.width, picture.height);
            assert!(picture.height <= 64);
        }
        Err(why) => {
            assert!(!strict(), "the Raw Image Extension is installed: {why:?}");
            assert_eq!(why, ImageRefusal::NoDecoder);
        }
    }
}

/// A minimal linear (demosaiced) 8-bit DNG, `width` × `height`, EXIF orientation `orientation`, left half red and
/// right half blue (the same builder as the helper's own `tests/image_decode.rs`).
fn dng(width: u32, height: u32, orientation: u16) -> Vec<u8> {
    const SHORT: u16 = 3;
    const LONG: u16 = 4;
    const ASCII: u16 = 2;
    const BYTE: u16 = 1;
    const RATIONAL: u16 = 5;
    const SRATIONAL: u16 = 10;
    let pixels: Vec<u8> = (0..width * height)
        .flat_map(|i| {
            if i % width < width / 2 {
                [200u8, 40, 40]
            } else {
                [40, 40, 200]
            }
        })
        .collect();
    let srational = |v: f64| {
        let mut b = ((v * 10_000.0).round() as i32).to_le_bytes().to_vec();
        b.extend_from_slice(&10_000i32.to_le_bytes());
        b
    };
    let matrix = [
        3.2406, -1.5372, -0.4986, -0.9689, 1.8758, 0.0415, 0.0557, -0.2040, 1.0570,
    ];
    let mut entries: Vec<(u16, u16, u32, Vec<u8>)> = vec![
        (254, LONG, 1, 0u32.to_le_bytes().to_vec()),
        (256, LONG, 1, width.to_le_bytes().to_vec()),
        (257, LONG, 1, height.to_le_bytes().to_vec()),
        (
            258,
            SHORT,
            3,
            [8u16, 8, 8].iter().flat_map(|v| v.to_le_bytes()).collect(),
        ),
        (259, SHORT, 1, 1u16.to_le_bytes().to_vec()),
        (262, SHORT, 1, 34892u16.to_le_bytes().to_vec()),
        (271, ASCII, 8, b"Beebeeb\0".to_vec()),
        (272, ASCII, 5, b"Test\0".to_vec()),
        (273, LONG, 1, vec![0; 4]), // strip offset, filled below
        (274, SHORT, 1, orientation.to_le_bytes().to_vec()),
        (277, SHORT, 1, 3u16.to_le_bytes().to_vec()),
        (278, LONG, 1, height.to_le_bytes().to_vec()),
        (279, LONG, 1, (pixels.len() as u32).to_le_bytes().to_vec()),
        (284, SHORT, 1, 1u16.to_le_bytes().to_vec()),
        (50706, BYTE, 4, vec![1, 4, 0, 0]),
        (50707, BYTE, 4, vec![1, 1, 0, 0]),
        (50708, ASCII, 13, b"Beebeeb Test\0".to_vec()),
        (50717, LONG, 1, 255u32.to_le_bytes().to_vec()),
        (50721, SRATIONAL, 9, matrix.iter().flat_map(|v| srational(*v)).collect()),
        (
            50728,
            RATIONAL,
            3,
            [1u32, 1, 1, 1, 1, 1].iter().flat_map(|v| v.to_le_bytes()).collect(),
        ),
        (50778, SHORT, 1, 21u16.to_le_bytes().to_vec()),
    ];
    let ifd_len = 2 + entries.len() * 12 + 4;
    let mut extra_at = 8 + ifd_len;
    let extra_len: usize = entries
        .iter()
        .filter(|e| e.3.len() > 4)
        .map(|e| e.3.len().div_ceil(2) * 2)
        .sum();
    let strip_at = (extra_at + extra_len) as u32;
    entries.iter_mut().find(|e| e.0 == 273).unwrap().3 = strip_at.to_le_bytes().to_vec();
    let mut out = b"II*\0".to_vec();
    out.extend_from_slice(&8u32.to_le_bytes());
    out.extend_from_slice(&(entries.len() as u16).to_le_bytes());
    let mut extra = Vec::new();
    for (tag, kind, count, data) in &entries {
        out.extend_from_slice(&tag.to_le_bytes());
        out.extend_from_slice(&kind.to_le_bytes());
        out.extend_from_slice(&count.to_le_bytes());
        if data.len() <= 4 {
            let mut inline = data.clone();
            inline.resize(4, 0);
            out.extend(inline);
        } else {
            out.extend_from_slice(&(extra_at as u32).to_le_bytes());
            let mut padded = data.clone();
            padded.resize(data.len().div_ceil(2) * 2, 0);
            extra_at += padded.len();
            extra.extend(padded);
        }
    }
    out.extend_from_slice(&0u32.to_le_bytes());
    out.extend(extra);
    assert_eq!(out.len(), strip_at as usize);
    out.extend(pixels);
    out
}
