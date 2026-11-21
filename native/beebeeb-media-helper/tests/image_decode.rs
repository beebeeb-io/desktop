//! The helper decodes one picture (task 048): the pure parts everywhere, the helper's process over its pipes with
//! pictures this file makes itself (no third-party photos), and on Windows the WIC decoders.
//!
//! With `BB_IMAGE_EXTENSIONS=1` (VM 401: HEIF, HEVC and Raw Image extensions installed) a HEIC and a DNG must decode
//! to pixels. Without it, a missing codec must answer `NoDecoder`.

mod image_decode {
    use std::io::{BufReader, BufWriter, Write};
    use std::process::{Command, Stdio};

    use beebeeb_media_helper::image_decode::{Picture, PictureKind, fit, orient, sniff_picture};
    use beebeeb_media_protocol::image::{
        DecodeImage, IMAGE_MODE_ARG, ImageAnswer, ImageFromHelper, ImageRefusal, MAX_IMAGE_FILE, read_image_to_app,
    };
    use beebeeb_media_protocol::{ToHelper, write_msg};

    /// Plays the app: starts the helper for a picture, serves its reads from `bytes`, returns its answer.
    fn ask_with_len(bytes: &[u8], len: u64, max_px: u32) -> ImageAnswer {
        let mut child = Command::new(env!("CARGO_BIN_EXE_beebeeb-media-helper"))
            .arg(IMAGE_MODE_ARG)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::null())
            .spawn()
            .expect("the helper starts");
        let mut to = BufWriter::new(child.stdin.take().unwrap());
        let mut from = BufReader::new(child.stdout.take().unwrap());
        write_msg(&mut to, &DecodeImage { max_px, len }).unwrap();
        let answer = loop {
            match read_image_to_app(&mut from).expect("a message the app reads") {
                ImageFromHelper::ReadAt { id, offset, len } => {
                    let start = offset as usize;
                    let end = start + len as usize;
                    assert!(end <= bytes.len(), "the helper reads inside the file");
                    write_msg(
                        &mut to,
                        &ToHelper::Data {
                            id,
                            bytes: bytes[start..end].to_vec(),
                        },
                    )
                    .unwrap();
                    to.flush().unwrap();
                }
                ImageFromHelper::Answer(answer) => break answer,
            }
        };
        drop(to);
        assert!(child.wait().unwrap().success());
        answer
    }

    fn ask(bytes: &[u8], max_px: u32) -> ImageAnswer {
        ask_with_len(bytes, bytes.len() as u64, max_px)
    }

    fn strict() -> bool {
        std::env::var("BB_IMAGE_EXTENSIONS").is_ok_and(|v| v == "1")
    }

    /// A 24-bit BMP, `width` × `height`, left half red, right half blue.
    fn bmp(width: u32, height: u32) -> Vec<u8> {
        let stride = (width * 3).div_ceil(4) * 4;
        let size = 54 + stride * height;
        let mut out = Vec::new();
        out.extend_from_slice(b"BM");
        out.extend_from_slice(&size.to_le_bytes());
        out.extend_from_slice(&0u32.to_le_bytes());
        out.extend_from_slice(&54u32.to_le_bytes());
        out.extend_from_slice(&40u32.to_le_bytes());
        out.extend_from_slice(&(width as i32).to_le_bytes());
        out.extend_from_slice(&(height as i32).to_le_bytes());
        out.extend_from_slice(&1u16.to_le_bytes());
        out.extend_from_slice(&24u16.to_le_bytes());
        out.extend_from_slice(&[0; 24]);
        for _ in 0..height {
            let mut row = Vec::new();
            for x in 0..width {
                row.extend_from_slice(if x < width / 2 { &[0, 0, 255] } else { &[255, 0, 0] }); // BGR
            }
            row.resize(stride as usize, 0);
            out.extend(row);
        }
        out
    }

    /// A minimal linear (demosaiced) 8-bit DNG, `width` × `height`, with EXIF orientation `orientation`.
    fn dng(width: u32, height: u32, orientation: u16) -> Vec<u8> {
        // (tag, type, count, value or offset placeholder, extra data)
        const SHORT: u16 = 3;
        const LONG: u16 = 4;
        const ASCII: u16 = 2;
        const BYTE: u16 = 1;
        const RATIONAL: u16 = 5;
        const SRATIONAL: u16 = 10;
        let pixels: Vec<u8> = (0..width * height)
            .flat_map(|i| {
                let x = i % width;
                if x < width / 2 { [200u8, 40, 40] } else { [40, 40, 200] }
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

    /// The first bytes of a HEIC (an `ftyp` box and the start of a `meta` box), and nothing a decoder can use.
    fn heic_head() -> Vec<u8> {
        let mut out = 24u32.to_be_bytes().to_vec();
        out.extend_from_slice(b"ftypheic\0\0\0\0mif1heic");
        out.extend_from_slice(&40u32.to_be_bytes());
        out.extend_from_slice(b"meta");
        out.extend_from_slice(&[0; 32]);
        out
    }

    fn pixel(width: u32, rgba: &[u8], x: u32, y: u32) -> [u8; 4] {
        let at = ((y * width + x) * 4) as usize;
        rgba[at..at + 4].try_into().unwrap()
    }

    #[test]
    fn picture_kinds_by_their_first_bytes() {
        assert_eq!(sniff_picture(&bmp(2, 2)), Some(PictureKind::Bmp));
        assert_eq!(sniff_picture(&dng(4, 2, 1)), Some(PictureKind::Tiff));
        assert_eq!(sniff_picture(&heic_head()), Some(PictureKind::Heif));
        assert_eq!(sniff_picture(b"\xFF\xD8\xFF\xE1"), Some(PictureKind::Jpeg));
        let mut cr3 = 24u32.to_be_bytes().to_vec();
        cr3.extend_from_slice(b"ftypcrx \0\0\0\x01crx isom");
        assert_eq!(sniff_picture(&cr3), Some(PictureKind::Raw));
        // An MP4 is not a picture.
        let mut mp4 = 24u32.to_be_bytes().to_vec();
        mp4.extend_from_slice(b"ftypisom\0\0\x02\0isommp41");
        assert_eq!(sniff_picture(&mp4), None);
        assert_eq!(sniff_picture(b"%PDF-1.7"), None);
        assert_eq!(sniff_picture(b""), None);
    }

    #[test]
    fn fit_keeps_the_aspect_within_max_px() {
        assert_eq!(fit(4000, 3000, 1000), (1000, 750));
        assert_eq!(fit(3000, 4000, 1000), (750, 1000));
        assert_eq!(fit(800, 600, 1000), (800, 600));
        assert_eq!(fit(10_000, 1, 100), (100, 1));
    }

    #[test]
    fn every_exif_orientation_turns_the_picture_upright() {
        // Stored:  A B C
        //          D E F
        let stored = Picture {
            width: 3,
            height: 2,
            rgba: b"AAAABBBBCCCCDDDDEEEEFFFF".to_vec(),
        };
        let shown = |o: u16| {
            let p = orient(stored.clone(), o);
            let letters: String = p.rgba.chunks(4).map(|c| c[0] as char).collect();
            (p.width, p.height, letters)
        };
        assert_eq!(shown(1), (3, 2, "ABCDEF".into()));
        assert_eq!(shown(2), (3, 2, "CBAFED".into()));
        assert_eq!(shown(3), (3, 2, "FEDCBA".into()));
        assert_eq!(shown(4), (3, 2, "DEFABC".into()));
        assert_eq!(shown(5), (2, 3, "ADBECF".into()));
        assert_eq!(shown(6), (2, 3, "DAEBFC".into()));
        assert_eq!(shown(7), (2, 3, "FCEBDA".into()));
        assert_eq!(shown(8), (2, 3, "CFBEAD".into()));
        assert_eq!(shown(0), (3, 2, "ABCDEF".into()));
        assert_eq!(shown(9), (3, 2, "ABCDEF".into()));
    }

    #[test]
    fn bytes_that_are_no_picture_are_unsupported() {
        let answer = ask(b"%PDF-1.7 not a picture at all", 256);
        assert_eq!(answer, ImageAnswer::Refused(ImageRefusal::Unsupported));
    }

    #[test]
    fn a_file_over_the_cap_is_too_large_before_a_byte_is_read() {
        let answer = ask_with_len(&[], MAX_IMAGE_FILE + 1, 256);
        assert_eq!(answer, ImageAnswer::Refused(ImageRefusal::TooLarge));
    }

    #[test]
    fn a_bmp_decodes_and_is_scaled_to_max_px() {
        let answer = ask(&bmp(8, 4), 4);
        if !cfg!(windows) {
            assert_eq!(answer, ImageAnswer::Refused(ImageRefusal::NoDecoder));
            return;
        }
        let ImageAnswer::Pixels { width, height, rgba } = answer else {
            panic!("{answer:?}");
        };
        assert_eq!((width, height), (4, 2));
        let left = pixel(width, &rgba, 0, 0);
        let right = pixel(width, &rgba, 3, 1);
        assert!(left[0] > 200 && left[2] < 50 && left[3] == 255, "{left:?}");
        assert!(right[2] > 200 && right[0] < 50, "{right:?}");
    }

    #[test]
    fn a_dng_decodes_upright_or_answers_no_decoder() {
        // Stored 48 × 32 landscape, orientation 6: shown portrait.
        let answer = ask(&dng(48, 32, 6), 64);
        eprintln!("dng: {answer:?}");
        match answer {
            ImageAnswer::Pixels { width, height, .. } => {
                assert!(cfg!(windows));
                assert!(width < height, "orientation 6 turns it portrait: {width}x{height}");
                assert!(height <= 64);
            }
            ImageAnswer::Refused(why) => {
                assert!(!strict(), "the Raw Image Extension is installed: {why:?}");
                assert_eq!(why, ImageRefusal::NoDecoder);
            }
        }
    }

    #[test]
    fn a_heic_decodes_or_answers_no_decoder() {
        #[cfg(windows)]
        if let Some(heic) = wic::encode_heic(64, 32) {
            let answer = ask(&heic, 32);
            eprintln!("heic: {answer:?}");
            let ImageAnswer::Pixels { width, height, rgba } = answer else {
                panic!("a HEIC this PC encoded does not decode: {answer:?}");
            };
            assert_eq!((width, height), (32, 16));
            let left = pixel(width, &rgba, 2, 8);
            let right = pixel(width, &rgba, 29, 8);
            assert!(left[0] > 150 && left[2] < 100, "{left:?}");
            assert!(right[2] > 150 && right[0] < 100, "{right:?}");
            return;
        }
        assert!(
            !strict(),
            "the HEIF and HEVC extensions are installed but no HEIC was encoded"
        );
        let answer = ask(&heic_head(), 32);
        eprintln!("heic head: {answer:?}");
        #[cfg(windows)]
        if wic::has_heif_component() {
            // The HEIF extension without HEVC: its decoder takes the head and fails on it, or finds no HEVC decoder.
            assert!(matches!(
                answer,
                ImageAnswer::Refused(ImageRefusal::NoDecoder | ImageRefusal::Damaged)
            ));
            return;
        }
        // No HEIF extension here: no decoder.
        assert_eq!(answer, ImageAnswer::Refused(ImageRefusal::NoDecoder));
    }

    /// The test's own HEIC, encoded by WIC when this PC has the HEIF and HEVC extensions.
    #[cfg(windows)]
    mod wic {
        use std::ptr::{null, null_mut};

        use windows::Win32::Graphics::Imaging::{
            CLSID_WICImagingFactory, GUID_ContainerFormatHeif, GUID_WICPixelFormat24bppBGR, IWICBitmapFrameEncode,
            IWICImagingFactory, WICBitmapEncoderNoCache,
        };
        use windows::Win32::System::Com::{
            CLSCTX_INPROC_SERVER, COINIT_MULTITHREADED, CoCreateInstance, CoInitializeEx,
        };
        use windows::core::{HSTRING, Result};

        pub fn has_heif_component() -> bool {
            // SAFETY (test): COM objects made and used on this thread.
            unsafe {
                let _ = CoInitializeEx(None, COINIT_MULTITHREADED);
                let factory: Result<IWICImagingFactory> =
                    CoCreateInstance(&CLSID_WICImagingFactory, None, CLSCTX_INPROC_SERVER);
                factory.is_ok_and(|f| f.CreateEncoder(&GUID_ContainerFormatHeif, null()).is_ok())
            }
        }

        pub fn encode_heic(width: u32, height: u32) -> Option<Vec<u8>> {
            let path = std::env::temp_dir().join(format!("bb-048-{}-{width}x{height}.heic", std::process::id()));
            let made = encode(&path, width, height);
            let bytes = made.ok().and_then(|()| std::fs::read(&path).ok());
            let _ = std::fs::remove_file(&path);
            bytes.filter(|b| !b.is_empty())
        }

        fn encode(path: &std::path::Path, width: u32, height: u32) -> Result<()> {
            // SAFETY (test): COM objects made and used on this thread; buffers outlive the calls.
            unsafe {
                let _ = CoInitializeEx(None, COINIT_MULTITHREADED);
                let factory: IWICImagingFactory =
                    CoCreateInstance(&CLSID_WICImagingFactory, None, CLSCTX_INPROC_SERVER)?;
                let encoder = factory.CreateEncoder(&GUID_ContainerFormatHeif, null())?;
                let stream = factory.CreateStream()?;
                stream.InitializeFromFilename(&HSTRING::from(path.as_os_str()), 0x4000_0000)?;
                encoder.Initialize(&stream, WICBitmapEncoderNoCache)?;
                let mut frame: Option<IWICBitmapFrameEncode> = None;
                encoder.CreateNewFrame(&mut frame, null_mut())?;
                let frame = frame.ok_or_else(windows::core::Error::empty)?;
                frame.Initialize(None)?;
                frame.SetSize(width, height)?;
                let stride = width * 3;
                let pixels: Vec<u8> = (0..width * height)
                    .flat_map(|i| {
                        if i % width < width / 2 {
                            [0u8, 0, 255]
                        } else {
                            [255, 0, 0]
                        }
                    })
                    .collect();
                let bitmap =
                    factory.CreateBitmapFromMemory(width, height, &GUID_WICPixelFormat24bppBGR, stride, &pixels)?;
                frame.WriteSource(&bitmap, null())?;
                frame.Commit()?;
                encoder.Commit()?;
                Ok(())
            }
        }
    }
}
