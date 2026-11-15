//! media-mf-make: writes WMV 9 (WMV3), VC-1 (WVC1), WMA Pro and WMA Lossless test files with Windows' own encoders.
//!
//! ffmpeg has no encoder for these four, so the spike makes them here: synthetic content only (a moving gradient,
//! 320x180 at 25 fps for 2 s, and the fixtures' two tones at 48 kHz stereo). Run unconfined, once, into a folder of
//! the spike's; it writes `manifest.tsv` for the runner next to the files.
#![cfg(windows)]

use std::fmt::Write as _;
use std::ptr::null_mut;

use windows::Win32::Media::MediaFoundation::*;
use windows::Win32::System::Com::{COINIT_MULTITHREADED, CoInitializeEx};
use windows::core::{GUID, Interface, PCWSTR};

const W: u32 = 320;
const H: u32 = 180;
const FPS: u32 = 25;
const FRAMES: u32 = 50;
const RATE: u32 = 48000;
const FRAME_100NS: i64 = 10_000_000 / FPS as i64;
/// Audio is written in 20 ms chunks, two per video frame.
const CHUNK: u32 = RATE / 50;
const CHUNK_100NS: i64 = 200_000;

trait Stage<T> {
    fn at(self, stage: &str) -> Result<T, String>;
}

impl<T> Stage<T> for windows::core::Result<T> {
    fn at(self, stage: &str) -> Result<T, String> {
        self.map_err(|e| format!("{stage} 0x{:08X}", e.code().0 as u32))
    }
}

fn video_type(subtype: &GUID) -> windows::core::Result<IMFMediaType> {
    unsafe {
        let t = MFCreateMediaType()?;
        t.SetGUID(&MF_MT_MAJOR_TYPE, &MFMediaType_Video)?;
        t.SetGUID(&MF_MT_SUBTYPE, subtype)?;
        t.SetUINT64(&MF_MT_FRAME_SIZE, ((W as u64) << 32) | H as u64)?;
        t.SetUINT64(&MF_MT_FRAME_RATE, ((FPS as u64) << 32) | 1)?;
        t.SetUINT64(&MF_MT_PIXEL_ASPECT_RATIO, (1u64 << 32) | 1)?;
        t.SetUINT32(&MF_MT_INTERLACE_MODE, MFVideoInterlace_Progressive.0 as u32)?;
        Ok(t)
    }
}

fn pcm_type() -> windows::core::Result<IMFMediaType> {
    unsafe {
        let t = MFCreateMediaType()?;
        t.SetGUID(&MF_MT_MAJOR_TYPE, &MFMediaType_Audio)?;
        t.SetGUID(&MF_MT_SUBTYPE, &MFAudioFormat_PCM)?;
        t.SetUINT32(&MF_MT_AUDIO_SAMPLES_PER_SECOND, RATE)?;
        t.SetUINT32(&MF_MT_AUDIO_NUM_CHANNELS, 2)?;
        t.SetUINT32(&MF_MT_AUDIO_BITS_PER_SAMPLE, 16)?;
        t.SetUINT32(&MF_MT_AUDIO_BLOCK_ALIGNMENT, 4)?;
        t.SetUINT32(&MF_MT_AUDIO_AVG_BYTES_PER_SECOND, RATE * 4)?;
        Ok(t)
    }
}

/// One of the encoder's own output types for `subtype`: 48 kHz stereo when it offers one.
fn audio_out_type(subtype: &GUID) -> Result<(IMFMediaType, String), String> {
    unsafe {
        let list = MFTranscodeGetAudioOutputAvailableTypes(subtype, MFT_ENUM_FLAG_ALL.0 as u32, None::<&IMFAttributes>)
            .at("a-types")?;
        let count = list.GetElementCount().at("a-types")?;
        let mut first: Option<(IMFMediaType, String)> = None;
        for i in 0..count {
            let t: IMFMediaType = list.GetElement(i).at("a-type")?.cast().at("a-type")?;
            let rate = t.GetUINT32(&MF_MT_AUDIO_SAMPLES_PER_SECOND).unwrap_or(0);
            let ch = t.GetUINT32(&MF_MT_AUDIO_NUM_CHANNELS).unwrap_or(0);
            let bytes = t.GetUINT32(&MF_MT_AUDIO_AVG_BYTES_PER_SECOND).unwrap_or(0);
            let label = format!("{rate}Hz/{ch}ch/{}kbps", bytes * 8 / 1000);
            if rate == RATE && ch == 2 {
                return Ok((t, label));
            }
            if first.is_none() {
                first = Some((t, label));
            }
        }
        first.ok_or_else(|| format!("a-types none-of-{count}"))
    }
}

fn sample(bytes: &[u8], time: i64, duration: i64) -> Result<IMFSample, String> {
    unsafe {
        let buffer = MFCreateMemoryBuffer(bytes.len() as u32).at("buffer")?;
        let mut base: *mut u8 = null_mut();
        buffer.Lock(&mut base, None, None).at("buffer-lock")?;
        std::ptr::copy_nonoverlapping(bytes.as_ptr(), base, bytes.len());
        buffer.Unlock().at("buffer-unlock")?;
        buffer.SetCurrentLength(bytes.len() as u32).at("buffer-length")?;
        let s = MFCreateSample().at("sample")?;
        s.AddBuffer(&buffer).at("sample-buffer")?;
        s.SetSampleTime(time).at("sample-time")?;
        s.SetSampleDuration(duration).at("sample-duration")?;
        Ok(s)
    }
}

/// A moving gradient, top-down 32-bit BGRX.
fn frame(i: u32) -> Vec<u8> {
    let mut out = Vec::with_capacity((W * H * 4) as usize);
    for y in 0..H {
        for x in 0..W {
            out.extend_from_slice(&[((x + y) & 255) as u8, (y & 255) as u8, ((x + i * 4) & 255) as u8, 255]);
        }
    }
    out
}

/// 20 ms of the fixtures' tones: 440 Hz left, 660 Hz right, 14 kHz in both; 16-bit interleaved.
fn chunk(k: u32) -> Vec<u8> {
    let mut out = Vec::with_capacity((CHUNK * 4) as usize);
    for n in 0..CHUNK {
        let t = (k * CHUNK + n) as f64 / RATE as f64;
        let tau = std::f64::consts::TAU;
        let hf = 0.2 * (tau * 14000.0 * t).sin();
        let l = 0.4 * (tau * 440.0 * t).sin() + hf;
        let r = 0.4 * (tau * 660.0 * t).sin() + hf;
        out.extend_from_slice(&((l * 32767.0) as i16).to_le_bytes());
        out.extend_from_slice(&((r * 32767.0) as i16).to_le_bytes());
    }
    out
}

fn make(path: &str, video: Option<GUID>, audio: Option<GUID>) -> Result<String, String> {
    unsafe {
        let wide: Vec<u16> = path.encode_utf16().chain(std::iter::once(0)).collect();
        let writer = MFCreateSinkWriterFromURL(PCWSTR(wide.as_ptr()), None::<&IMFByteStream>, None::<&IMFAttributes>)
            .at("sink-writer")?;
        let mut notes = String::new();
        let mut v_stream = None;
        if let Some(subtype) = video {
            let target = video_type(&subtype).at("v-out-type")?;
            target.SetUINT32(&MF_MT_AVG_BITRATE, 800_000).at("v-out-type")?;
            let s = writer.AddStream(&target).at("v-add-stream")?;
            let input = video_type(&MFVideoFormat_RGB32).at("v-in-type")?;
            writer
                .SetInputMediaType(s, &input, None::<&IMFAttributes>)
                .at("v-in-type")?;
            v_stream = Some(s);
        }
        let mut a_stream = None;
        if let Some(subtype) = audio {
            let (target, label) = audio_out_type(&subtype)?;
            let _ = write!(notes, "audio={label}");
            let s = writer.AddStream(&target).at("a-add-stream")?;
            writer
                .SetInputMediaType(s, &pcm_type().at("a-in-type")?, None::<&IMFAttributes>)
                .at("a-in-type")?;
            a_stream = Some(s);
        }
        writer.BeginWriting().at("begin")?;
        for i in 0..FRAMES {
            if let Some(s) = v_stream {
                writer
                    .WriteSample(s, &sample(&frame(i), i as i64 * FRAME_100NS, FRAME_100NS)?)
                    .at("v-write")?;
            }
            if let Some(s) = a_stream {
                for k in [i * 2, i * 2 + 1] {
                    writer
                        .WriteSample(s, &sample(&chunk(k), k as i64 * CHUNK_100NS, CHUNK_100NS)?)
                        .at("a-write")?;
                }
            }
        }
        writer.Finalize().at("finalize")?;
        Ok(notes)
    }
}

fn main() {
    let Some(dir) = std::env::args().nth(1) else {
        eprintln!("usage: media-mf-make <out-dir>");
        std::process::exit(2);
    };
    let _ = std::fs::create_dir_all(&dir);
    unsafe {
        let _ = CoInitializeEx(None, COINIT_MULTITHREADED);
        if let Err(e) = MFStartup(MF_VERSION, MFSTARTUP_FULL) {
            println!("MAKE mfstartup 0x{:08X}", e.code().0 as u32);
            return;
        }
    }
    let files: [(&str, &str, Option<GUID>, Option<GUID>, &str); 4] = [
        (
            "wmv3-wmapro.wmv",
            "av",
            Some(MFVideoFormat_WMV3),
            Some(MFAudioFormat_WMAudioV9),
            "gap: WMV 9 (WMV3) + WMA Pro, ASF, made by the in-box encoders",
        ),
        (
            "wvc1-wmalossless.wmv",
            "av",
            Some(MFVideoFormat_WVC1),
            Some(MFAudioFormat_WMAudio_Lossless),
            "gap: VC-1 Advanced (WVC1) + WMA Lossless, ASF, made by the in-box encoders",
        ),
        (
            "wmapro.wma",
            "a",
            None,
            Some(MFAudioFormat_WMAudioV9),
            "gap: WMA Pro, made by the in-box encoder",
        ),
        (
            "wmalossless.wma",
            "a",
            None,
            Some(MFAudioFormat_WMAudio_Lossless),
            "gap: WMA Lossless, made by the in-box encoder",
        ),
    ];
    let mut manifest = String::from("# file\tkind\tcodec\twant\tframes\tref_y0\tnote\n");
    for (name, kind, video, audio, note) in files {
        let path = format!("{dir}\\{name}");
        let _ = std::fs::remove_file(&path);
        match make(&path, video, audio) {
            Ok(notes) => {
                let size = std::fs::metadata(&path).map(|m| m.len()).unwrap_or(0);
                println!("MAKE ok {name} bytes={size} {notes}");
                let frames = if video.is_some() {
                    FRAMES.to_string()
                } else {
                    "-".into()
                };
                let _ = writeln!(manifest, "{name}\t{kind}\t{name}\tnv12\t{frames}\t-\t{note}");
            }
            Err(e) => println!("MAKE fail {name} {e}"),
        }
    }
    let _ = std::fs::write(format!("{dir}\\manifest.tsv"), manifest);
    unsafe {
        let _ = MFShutdown();
    }
}
