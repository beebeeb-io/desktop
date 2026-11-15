//! media-mf-probe: the decoder side of the Media Foundation spike (M10).
//!
//! Started with no arguments (the sandbox gives none). It reads ONE request from its standard input:
//!
//! - `F <ext> <sw|hw> <nv12|p010> <len>\n` followed by `<len>` bytes: decode the first video stream (and, for `sw`,
//!   the first audio stream) of those bytes. `<ext>` is only a hint for Media Foundation's source resolver
//!   (`probe.<ext>`); no file name or path reaches the probe. The bytes reach Media Foundation through a byte stream
//!   of ours over memory: nothing is written to disk.
//! - `E\n`: list the decoders Media Foundation can find from inside this process.
//!
//! It writes lines to its standard output only (a confined helper has no standard error):
//!
//! - `T …` the process token, read BEFORE any Media Foundation call: proves whether this run was confined.
//! - `S <stage> <hresult> …` one line per stage.
//! - `V …` / `A …` the video and audio results.
//! - `M …` / `W …` modules loaded during the request; those from a Store package (`\WindowsApps\`).
//! - `END <ms>`.
//!
//! Throwaway spike code: no product file uses it.
#![cfg(windows)]

use std::io::{BufRead, Read, Write};
use std::ptr::null_mut;
use std::sync::Mutex;
use std::time::Instant;

use sha2::{Digest, Sha256};
use windows::Win32::Foundation::{CloseHandle, E_INVALIDARG, E_NOTIMPL, HANDLE, HMODULE, S_OK};
use windows::Win32::Graphics::Direct3D::D3D_DRIVER_TYPE_HARDWARE;
use windows::Win32::Graphics::Direct3D11::{
    D3D11_CREATE_DEVICE_BGRA_SUPPORT, D3D11_CREATE_DEVICE_FLAG, D3D11_CREATE_DEVICE_VIDEO_SUPPORT, D3D11_SDK_VERSION,
    D3D11CreateDevice, ID3D11Device, ID3D11Multithread,
};
use windows::Win32::Graphics::Dxgi::{IDXGIAdapter, IDXGIDevice};
use windows::Win32::Media::MediaFoundation::*;
use windows::Win32::Security::{
    GetTokenInformation, TOKEN_QUERY, TokenCapabilities, TokenIsAppContainer, TokenIsLessPrivilegedAppContainer,
};
use windows::Win32::System::Com::{COINIT_MULTITHREADED, CoInitializeEx, CoTaskMemFree};
use windows::Win32::System::JobObjects::IsProcessInJob;
use windows::Win32::System::LibraryLoader::GetModuleFileNameW;
use windows::Win32::System::ProcessStatus::EnumProcessModules;
use windows::Win32::System::Threading::{GetCurrentProcess, OpenProcessToken};
use windows::Win32::UI::Shell::PropertiesSystem::IPropertyStore;
use windows::core::{BOOL, GUID, HRESULT, IUnknown, Interface, PCWSTR, PWSTR, Ref, implement};

const ALL_STREAMS: u32 = MF_SOURCE_READER_ALL_STREAMS.0 as u32;
const FIRST_VIDEO: u32 = MF_SOURCE_READER_FIRST_VIDEO_STREAM.0 as u32;
const FIRST_AUDIO: u32 = MF_SOURCE_READER_FIRST_AUDIO_STREAM.0 as u32;
const FLAG_ERROR: u32 = MF_SOURCE_READERF_ERROR.0 as u32;
const FLAG_END: u32 = MF_SOURCE_READERF_ENDOFSTREAM.0 as u32;
const FLAG_TYPE_CHANGED: u32 = MF_SOURCE_READERF_CURRENTMEDIATYPECHANGED.0 as u32;
/// Where `BeginRead` leaves the byte count for `EndRead` (an attribute store is the async result's object).
const READ_COUNT_KEY: GUID = GUID::from_u128(0x6d1f0c2a_83b5_4f4e_9a51_3c0b6e2d7a11);
/// A run never reads more than this many samples per stream (the fixtures hold 50 frames / 2 s).
const MAX_READS: u32 = 2000;

fn out(line: &str) {
    let mut o = std::io::stdout().lock();
    let _ = writeln!(o, "{line}");
    let _ = o.flush();
}

fn hx(hr: HRESULT) -> String {
    format!("0x{:08X}", hr.0 as u32)
}

fn ex(e: &windows::core::Error) -> String {
    hx(e.code())
}

/// Attaches a stage name to a failed call: the error becomes `"<stage> <hresult>"`.
trait Stage<T> {
    fn at(self, stage: &str) -> Result<T, String>;
}

impl<T> Stage<T> for windows::core::Result<T> {
    fn at(self, stage: &str) -> Result<T, String> {
        self.map_err(|e| format!("{stage} {}", ex(&e)))
    }
}

// ---------------------------------------------------------------------------------------------------------------
// The byte stream: the whole file in memory, read-only, seekable. This is the shape the product would use (its
// reads would become range requests to the app instead of slices of a Vec).
// ---------------------------------------------------------------------------------------------------------------

#[implement(IMFByteStream)]
struct MemStream {
    data: Vec<u8>,
    pos: Mutex<u64>,
}

impl MemStream {
    /// Copies up to `cb` bytes from the current position into `pb` and advances the position.
    fn read_into(&self, pb: *mut u8, cb: u32) -> u32 {
        let mut pos = self.pos.lock().unwrap_or_else(|p| p.into_inner());
        let len = self.data.len() as u64;
        let start = (*pos).min(len) as usize;
        let n = (cb as usize).min(self.data.len() - start);
        if n > 0 {
            // SAFETY: the caller hands a buffer writable for `cb` bytes; `n <= cb` and the source range is in bounds.
            unsafe { std::ptr::copy_nonoverlapping(self.data.as_ptr().add(start), pb, n) };
        }
        *pos = (start + n) as u64;
        n as u32
    }
}

impl IMFByteStream_Impl for MemStream_Impl {
    fn GetCapabilities(&self) -> windows::core::Result<u32> {
        Ok(MFBYTESTREAM_IS_READABLE | MFBYTESTREAM_IS_SEEKABLE)
    }
    fn GetLength(&self) -> windows::core::Result<u64> {
        Ok(self.data.len() as u64)
    }
    fn SetLength(&self, _qwlength: u64) -> windows::core::Result<()> {
        Err(E_NOTIMPL.into())
    }
    fn GetCurrentPosition(&self) -> windows::core::Result<u64> {
        Ok(*self.pos.lock().unwrap_or_else(|p| p.into_inner()))
    }
    fn SetCurrentPosition(&self, qwposition: u64) -> windows::core::Result<()> {
        *self.pos.lock().unwrap_or_else(|p| p.into_inner()) = qwposition.min(self.data.len() as u64);
        Ok(())
    }
    fn IsEndOfStream(&self) -> windows::core::Result<BOOL> {
        let pos = *self.pos.lock().unwrap_or_else(|p| p.into_inner());
        Ok(BOOL::from(pos >= self.data.len() as u64))
    }
    fn Read(&self, pb: *mut u8, cb: u32, pcbread: *mut u32) -> windows::core::Result<()> {
        let n = self.read_into(pb, cb);
        if !pcbread.is_null() {
            // SAFETY: a non-null out pointer from the caller.
            unsafe { *pcbread = n };
        }
        Ok(())
    }
    fn BeginRead(
        &self,
        pb: *mut u8,
        cb: u32,
        pcallback: Ref<IMFAsyncCallback>,
        punkstate: Ref<IUnknown>,
    ) -> windows::core::Result<()> {
        // The read itself is synchronous (memory); the completion still goes through Media Foundation's own work
        // queue, as the interface requires.
        let n = self.read_into(pb, cb);
        unsafe {
            let mut store: Option<IMFAttributes> = None;
            MFCreateAttributes(&mut store, 1)?;
            let store = store.ok_or_else(|| windows::core::Error::from(E_INVALIDARG))?;
            store.SetUINT32(&READ_COUNT_KEY, n)?;
            let result = MFCreateAsyncResult(&store, pcallback.as_ref(), punkstate.as_ref())?;
            result.SetStatus(S_OK)?;
            MFInvokeCallback(&result)?;
        }
        Ok(())
    }
    fn EndRead(&self, presult: Ref<IMFAsyncResult>) -> windows::core::Result<u32> {
        let result = presult.ok()?;
        unsafe {
            result.GetStatus()?;
            let store: IMFAttributes = result.GetObject()?.cast()?;
            store.GetUINT32(&READ_COUNT_KEY)
        }
    }
    fn Write(&self, _pb: *const u8, _cb: u32) -> windows::core::Result<u32> {
        Err(E_NOTIMPL.into())
    }
    fn BeginWrite(
        &self,
        _pb: *const u8,
        _cb: u32,
        _pcallback: Ref<IMFAsyncCallback>,
        _punkstate: Ref<IUnknown>,
    ) -> windows::core::Result<()> {
        Err(E_NOTIMPL.into())
    }
    fn EndWrite(&self, _presult: Ref<IMFAsyncResult>) -> windows::core::Result<u32> {
        Err(E_NOTIMPL.into())
    }
    fn Seek(
        &self,
        seekorigin: MFBYTESTREAM_SEEK_ORIGIN,
        llseekoffset: i64,
        _dwseekflags: u32,
    ) -> windows::core::Result<u64> {
        let mut pos = self.pos.lock().unwrap_or_else(|p| p.into_inner());
        let base: i64 = if seekorigin == msoBegin {
            0
        } else if seekorigin == msoCurrent {
            *pos as i64
        } else {
            return Err(E_INVALIDARG.into());
        };
        let target = base.saturating_add(llseekoffset).clamp(0, self.data.len() as i64) as u64;
        *pos = target;
        Ok(target)
    }
    fn Flush(&self) -> windows::core::Result<()> {
        Ok(())
    }
    fn Close(&self) -> windows::core::Result<()> {
        Ok(())
    }
}

// ---------------------------------------------------------------------------------------------------------------
// The process: its token, its modules.
// ---------------------------------------------------------------------------------------------------------------

/// Reads this process's own confinement before anything else runs.
fn token_line() -> String {
    unsafe {
        let mut token = HANDLE::default();
        if let Err(e) = OpenProcessToken(GetCurrentProcess(), TOKEN_QUERY, &mut token) {
            return format!("T error open-token {}", ex(&e));
        }
        let mut returned = 0u32;
        let mut is_container = 0u32;
        let container = GetTokenInformation(
            token,
            TokenIsAppContainer,
            Some((&raw mut is_container).cast()),
            4,
            &mut returned,
        )
        .map(|_| is_container.to_string())
        .unwrap_or_else(|e| format!("err{}", ex(&e)));
        let mut is_lpac = 0u32;
        let lpac = GetTokenInformation(
            token,
            TokenIsLessPrivilegedAppContainer,
            Some((&raw mut is_lpac).cast()),
            4,
            &mut returned,
        )
        .map(|_| is_lpac.to_string())
        .unwrap_or_else(|e| format!("err{}", ex(&e)));
        let mut capabilities = [0u64; 512];
        let caps = GetTokenInformation(
            token,
            TokenCapabilities,
            Some(capabilities.as_mut_ptr().cast()),
            size_of_val(&capabilities) as u32,
            &mut returned,
        )
        .map(|_| (capabilities[0] as u32).to_string())
        .unwrap_or_else(|e| format!("err{}", ex(&e)));
        let mut in_job = BOOL(0);
        let job = IsProcessInJob(GetCurrentProcess(), None, &mut in_job)
            .map(|_| u8::from(in_job.as_bool()).to_string())
            .unwrap_or_else(|e| format!("err{}", ex(&e)));
        let _ = CloseHandle(token);
        format!("T appcontainer={container} lpac={lpac} caps={caps} injob={job}")
    }
}

/// Full paths of the modules loaded in this process.
fn modules() -> Vec<String> {
    unsafe {
        let mut handles = vec![HMODULE::default(); 2048];
        let mut needed = 0u32;
        let bytes = (handles.len() * size_of::<HMODULE>()) as u32;
        if EnumProcessModules(GetCurrentProcess(), handles.as_mut_ptr(), bytes, &mut needed).is_err() {
            return vec!["<enum-failed>".into()];
        }
        let count = (needed as usize / size_of::<HMODULE>()).min(handles.len());
        handles[..count]
            .iter()
            .map(|&m| {
                let mut buf = [0u16; 1024];
                let len = GetModuleFileNameW(Some(m), &mut buf) as usize;
                String::from_utf16_lossy(&buf[..len.min(buf.len())])
            })
            .collect()
    }
}

/// Prints the modules that appeared during the request: base names, then the Store-package ones with their package.
fn print_new_modules(before: &[String]) {
    let after = modules();
    let mut new: Vec<String> = Vec::new();
    let mut store: Vec<String> = Vec::new();
    for path in &after {
        if before.iter().any(|b| b.eq_ignore_ascii_case(path)) {
            continue;
        }
        let base = path.rsplit('\\').next().unwrap_or(path).to_ascii_lowercase();
        let lower = path.to_ascii_lowercase();
        if let Some(i) = lower.find("\\windowsapps\\") {
            let rest = &path[i + "\\windowsapps\\".len()..];
            let package = rest.split('\\').next().unwrap_or("?");
            store.push(format!("{package}/{base}"));
        }
        new.push(base);
    }
    new.sort();
    store.sort();
    out(&format!("M n={} {}", new.len(), new.join(",")));
    out(&format!("W n={} {}", store.len(), store.join(",")));
}

// ---------------------------------------------------------------------------------------------------------------
// Media Foundation.
// ---------------------------------------------------------------------------------------------------------------

/// A subtype as text: its four-character code when it has one, else the format tag in hexadecimal.
fn fourcc(g: &GUID) -> String {
    let b = g.data1.to_le_bytes();
    if b.iter().all(|c| c.is_ascii_alphanumeric() || *c == b' ') {
        String::from_utf8_lossy(&b).trim().to_string()
    } else {
        format!("0x{:04X}", g.data1)
    }
}

fn subtype_name(g: &GUID) -> &'static str {
    if *g == MFVideoFormat_NV12 {
        "nv12"
    } else if *g == MFVideoFormat_P010 {
        "p010"
    } else if *g == MFVideoFormat_YUY2 {
        "yuy2"
    } else if *g == MFVideoFormat_RGB32 {
        "rgb32"
    } else {
        "other"
    }
}

/// Builds a byte stream over `data`, resolves it to a media source and opens a source reader on it.
fn open_reader(
    data: &[u8],
    ext: &str,
    d3d: Option<&IMFDXGIDeviceManager>,
    tag: &str,
) -> Result<IMFSourceReader, String> {
    unsafe {
        let stream: IMFByteStream = MemStream {
            data: data.to_vec(),
            pos: Mutex::new(0),
        }
        .into();
        let resolver = MFCreateSourceResolver().at(&format!("{tag}-resolver"))?;
        let hint: Vec<u16> = format!("probe.{ext}")
            .encode_utf16()
            .chain(std::iter::once(0))
            .collect();
        let flags = (MF_RESOLUTION_MEDIASOURCE.0
            | MF_RESOLUTION_CONTENT_DOES_NOT_HAVE_TO_MATCH_EXTENSION_OR_MIME_TYPE.0
            | MF_RESOLUTION_READ.0) as u32;
        let mut kind = MF_OBJECT_TYPE::default();
        let mut object: Option<IUnknown> = None;
        resolver
            .CreateObjectFromByteStream(
                &stream,
                PCWSTR(hint.as_ptr()),
                flags,
                None::<&IPropertyStore>,
                &mut kind,
                &mut object,
            )
            .at(&format!("{tag}-resolve"))?;
        let source: IMFMediaSource = object
            .ok_or_else(|| format!("{tag}-resolve null"))?
            .cast()
            .at(&format!("{tag}-source"))?;
        let mut attributes: Option<IMFAttributes> = None;
        MFCreateAttributes(&mut attributes, 4).at(&format!("{tag}-attributes"))?;
        let attributes = attributes.ok_or_else(|| format!("{tag}-attributes null"))?;
        attributes
            .SetUINT32(&MF_SOURCE_READER_ENABLE_ADVANCED_VIDEO_PROCESSING, 1)
            .at(&format!("{tag}-attributes"))?;
        if let Some(manager) = d3d {
            attributes
                .SetUnknown(&MF_SOURCE_READER_D3D_MANAGER, manager)
                .at(&format!("{tag}-d3d-attr"))?;
            attributes
                .SetUINT32(&MF_READWRITE_ENABLE_HARDWARE_TRANSFORMS, 1)
                .at(&format!("{tag}-d3d-attr"))?;
        }
        out(&format!("S {tag}-resolve 0x00000000"));
        MFCreateSourceReaderFromMediaSource(&source, &attributes).at(&format!("{tag}-reader"))
    }
}

/// The visible picture of a decoded frame: the frame size and the display aperture inside it.
#[derive(Clone, Copy, Debug)]
struct Geometry {
    w: u32,
    h: u32,
    ax: u32,
    ay: u32,
    aw: u32,
    ah: u32,
    stride: i32,
}

impl Geometry {
    fn of(t: &IMFMediaType) -> Geometry {
        unsafe {
            let size = t.GetUINT64(&MF_MT_FRAME_SIZE).unwrap_or(0);
            let (w, h) = ((size >> 32) as u32, size as u32);
            let (mut ax, mut ay, mut aw, mut ah) = (0, 0, w, h);
            let mut blob = [0u8; 16];
            if t.GetBlob(&MF_MT_MINIMUM_DISPLAY_APERTURE, &mut blob, None).is_ok() {
                // MFVideoArea: OffsetX {fract u16, value i16}, OffsetY {fract u16, value i16}, Area {cx i32, cy i32}.
                ax = i16::from_le_bytes([blob[2], blob[3]]).max(0) as u32;
                ay = i16::from_le_bytes([blob[6], blob[7]]).max(0) as u32;
                aw = i32::from_le_bytes([blob[8], blob[9], blob[10], blob[11]]).max(0) as u32;
                ah = i32::from_le_bytes([blob[12], blob[13], blob[14], blob[15]]).max(0) as u32;
            }
            let stride = t.GetUINT32(&MF_MT_DEFAULT_STRIDE).map(|s| s as i32).unwrap_or(w as i32);
            Geometry {
                w,
                h,
                ax,
                ay,
                aw,
                ah,
                stride,
            }
        }
    }
}

fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}

/// SHA-256 of the visible luma plane (8-bit for NV12, 16-bit little-endian for P010) of one decoded frame.
fn luma_hash(sample: &IMFSample, g: &Geometry, subtype: &GUID) -> Result<String, String> {
    let bytes_per = if *subtype == MFVideoFormat_NV12 {
        1usize
    } else if *subtype == MFVideoFormat_P010 {
        2usize
    } else {
        return Ok("-".into());
    };
    if g.aw == 0 || g.ah == 0 || g.ax + g.aw > g.w || g.ay + g.ah > g.h {
        return Err(format!("v-geometry {g:?}"));
    }
    unsafe {
        let buffer = sample.GetBufferByIndex(0).at("v-buffer")?;
        let mut hasher = Sha256::new();
        if let Ok(two_d) = buffer.cast::<IMF2DBuffer>() {
            let mut line0: *mut u8 = null_mut();
            let mut pitch = 0i32;
            two_d.Lock2D(&mut line0, &mut pitch).at("v-lock2d")?;
            let row_bytes = g.aw as usize * bytes_per;
            if pitch <= 0 || (g.ax as usize * bytes_per + row_bytes) > pitch as usize {
                let _ = two_d.Unlock2D();
                return Err(format!("v-pitch {pitch}"));
            }
            for r in g.ay..g.ay + g.ah {
                let row = line0.offset(r as isize * pitch as isize).add(g.ax as usize * bytes_per);
                hasher.update(std::slice::from_raw_parts(row, row_bytes));
            }
            let _ = two_d.Unlock2D();
        } else {
            let mut base: *mut u8 = null_mut();
            let mut len = 0u32;
            buffer.Lock(&mut base, None, Some(&raw mut len)).at("v-lock")?;
            let pitch = g.stride;
            let row_bytes = g.aw as usize * bytes_per;
            let needed = (g.ay + g.ah - 1) as usize * pitch.max(0) as usize + g.ax as usize * bytes_per + row_bytes;
            if pitch <= 0 || needed > len as usize {
                let _ = buffer.Unlock();
                return Err(format!("v-stride {pitch} len={len}"));
            }
            for r in g.ay..g.ay + g.ah {
                let row = base.add(r as usize * pitch as usize + g.ax as usize * bytes_per);
                hasher.update(std::slice::from_raw_parts(row, row_bytes));
            }
            let _ = buffer.Unlock();
        }
        Ok(hex(&hasher.finalize()))
    }
}

/// Names the transforms the source reader put between the source and our output (decoder, video processor).
fn transforms(reader: &IMFSourceReader, stream: u32, tag: &str) {
    unsafe {
        let Ok(ex_reader) = reader.cast::<IMFSourceReaderEx>() else {
            out(&format!("S {tag}-mft no-reader-ex"));
            return;
        };
        for i in 0..6u32 {
            let mut category = GUID::zeroed();
            let mut transform: Option<IMFTransform> = None;
            if ex_reader
                .GetTransformForStream(stream, i, Some(&raw mut category), &mut transform)
                .is_err()
            {
                break;
            }
            let Some(transform) = transform else { break };
            let kind = if category == MFT_CATEGORY_VIDEO_DECODER {
                "video-decoder"
            } else if category == MFT_CATEGORY_AUDIO_DECODER {
                "audio-decoder"
            } else if category == MFT_CATEGORY_VIDEO_PROCESSOR {
                "video-processor"
            } else {
                "other"
            };
            let (hardware, d3d11) = match transform.GetAttributes() {
                Ok(a) => (
                    a.GetStringLength(&MFT_ENUM_HARDWARE_URL_Attribute).is_ok(),
                    a.GetUINT32(&MF_SA_D3D11_AWARE).unwrap_or(0),
                ),
                Err(_) => (false, 0),
            };
            out(&format!(
                "S {tag}-mft{i} {kind} hardware={} d3d11aware={d3d11}",
                u8::from(hardware)
            ));
        }
    }
}

fn video(data: &[u8], ext: &str, want: &str, d3d: Option<&IMFDXGIDeviceManager>) -> Result<(), String> {
    let reader = open_reader(data, ext, d3d, "v")?;
    unsafe {
        let _ = reader.SetStreamSelection(ALL_STREAMS, false);
        if let Err(e) = reader.SetStreamSelection(FIRST_VIDEO, true) {
            out(&format!("V none {}", ex(&e)));
            return Ok(());
        }
        let native = reader.GetNativeMediaType(FIRST_VIDEO, 0).at("v-native")?;
        let native_sub = native
            .GetGUID(&MF_MT_SUBTYPE)
            .map(|g| fourcc(&g))
            .unwrap_or_else(|e| format!("err{}", ex(&e)));
        let interlace = native
            .GetUINT32(&MF_MT_INTERLACE_MODE)
            .map(|v| v.to_string())
            .unwrap_or_else(|_| "-".into());
        out(&format!("S v-native {native_sub} interlace={interlace}"));

        let order: [GUID; 4] = if want == "p010" {
            [
                MFVideoFormat_P010,
                MFVideoFormat_NV12,
                MFVideoFormat_YUY2,
                MFVideoFormat_RGB32,
            ]
        } else {
            [
                MFVideoFormat_NV12,
                MFVideoFormat_P010,
                MFVideoFormat_YUY2,
                MFVideoFormat_RGB32,
            ]
        };
        let mut chosen: Option<GUID> = None;
        for subtype in order {
            let t = MFCreateMediaType().at("v-type")?;
            t.SetGUID(&MF_MT_MAJOR_TYPE, &MFMediaType_Video).at("v-type")?;
            t.SetGUID(&MF_MT_SUBTYPE, &subtype).at("v-type")?;
            match reader.SetCurrentMediaType(FIRST_VIDEO, None, &t) {
                Ok(()) => {
                    out(&format!("S v-set-{} 0x00000000", subtype_name(&subtype)));
                    chosen = Some(subtype);
                    break;
                }
                Err(e) => out(&format!("S v-set-{} {}", subtype_name(&subtype), ex(&e))),
            }
        }
        let Some(subtype) = chosen else {
            return Err("v-set-type none-accepted".into());
        };
        let mut geometry = Geometry::of(&reader.GetCurrentMediaType(FIRST_VIDEO).at("v-current")?);

        let started = Instant::now();
        let mut frames = 0u32;
        let mut hash = String::from("-");
        let mut first_read: Option<String> = None;
        let mut failure: Option<String> = None;
        for _ in 0..MAX_READS {
            let mut index = 0u32;
            let mut flags = 0u32;
            let mut time = 0i64;
            let mut sample: Option<IMFSample> = None;
            let read = reader.ReadSample(
                FIRST_VIDEO,
                0,
                Some(&raw mut index),
                Some(&raw mut flags),
                Some(&raw mut time),
                Some(&raw mut sample),
            );
            if first_read.is_none() {
                first_read = Some(match &read {
                    Ok(()) => "0x00000000".into(),
                    Err(e) => ex(e),
                });
            }
            if let Err(e) = read {
                failure = Some(format!("v-read {}", ex(&e)));
                break;
            }
            if flags & FLAG_ERROR != 0 {
                failure = Some("v-read flag-error".into());
                break;
            }
            if flags & FLAG_TYPE_CHANGED != 0 {
                if let Ok(t) = reader.GetCurrentMediaType(FIRST_VIDEO) {
                    geometry = Geometry::of(&t);
                }
            }
            if let Some(sample) = sample {
                frames += 1;
                if frames == 1 {
                    hash = luma_hash(&sample, &geometry, &subtype).unwrap_or_else(|e| format!("err:{e}"));
                }
            }
            if flags & FLAG_END != 0 {
                break;
            }
        }
        out(&format!("S v-first-read {}", first_read.unwrap_or_else(|| "-".into())));
        transforms(&reader, FIRST_VIDEO, "v");
        let ms = started.elapsed().as_millis();
        let state = if failure.is_none() && frames > 0 { "ok" } else { "fail" };
        out(&format!(
            "V {state} {} native={native_sub} out={} frames={frames} size={}x{} visible={}x{}+{}+{} hash={hash} ms={ms}",
            failure.unwrap_or_else(|| "-".into()),
            subtype_name(&subtype),
            geometry.w,
            geometry.h,
            geometry.aw,
            geometry.ah,
            geometry.ax,
            geometry.ay,
        ));
    }
    Ok(())
}

fn audio(data: &[u8], ext: &str) -> Result<(), String> {
    let reader = open_reader(data, ext, None, "a")?;
    unsafe {
        let _ = reader.SetStreamSelection(ALL_STREAMS, false);
        if let Err(e) = reader.SetStreamSelection(FIRST_AUDIO, true) {
            out(&format!("A none {}", ex(&e)));
            return Ok(());
        }
        let native = reader.GetNativeMediaType(FIRST_AUDIO, 0).at("a-native")?;
        let tag = native
            .GetGUID(&MF_MT_SUBTYPE)
            .map(|g| format!("0x{:04X}", g.data1))
            .unwrap_or_else(|_| "-".into());
        let native_rate = native.GetUINT32(&MF_MT_AUDIO_SAMPLES_PER_SECOND).unwrap_or(0);
        let native_ch = native.GetUINT32(&MF_MT_AUDIO_NUM_CHANNELS).unwrap_or(0);
        out(&format!("S a-native {tag} rate={native_rate} ch={native_ch}"));

        let t = MFCreateMediaType().at("a-type")?;
        t.SetGUID(&MF_MT_MAJOR_TYPE, &MFMediaType_Audio).at("a-type")?;
        t.SetGUID(&MF_MT_SUBTYPE, &MFAudioFormat_Float).at("a-type")?;
        reader.SetCurrentMediaType(FIRST_AUDIO, None, &t).at("a-set-float")?;
        out("S a-set-float 0x00000000");
        let current = reader.GetCurrentMediaType(FIRST_AUDIO).at("a-current")?;
        let rate = current.GetUINT32(&MF_MT_AUDIO_SAMPLES_PER_SECOND).unwrap_or(0);
        let ch = current.GetUINT32(&MF_MT_AUDIO_NUM_CHANNELS).unwrap_or(0).max(1);

        let one_second = rate as usize * ch as usize;
        let mut first: Vec<f32> = Vec::with_capacity(one_second);
        let mut total = 0u64;
        let mut failure: Option<String> = None;
        let mut first_read: Option<String> = None;
        for _ in 0..MAX_READS {
            let mut flags = 0u32;
            let mut sample: Option<IMFSample> = None;
            let read = reader.ReadSample(FIRST_AUDIO, 0, None, Some(&raw mut flags), None, Some(&raw mut sample));
            if first_read.is_none() {
                first_read = Some(match &read {
                    Ok(()) => "0x00000000".into(),
                    Err(e) => ex(e),
                });
            }
            if let Err(e) = read {
                failure = Some(format!("a-read {}", ex(&e)));
                break;
            }
            if flags & FLAG_ERROR != 0 {
                failure = Some("a-read flag-error".into());
                break;
            }
            if let Some(sample) = sample {
                let buffer = sample.ConvertToContiguousBuffer().at("a-buffer")?;
                let mut base: *mut u8 = null_mut();
                let mut len = 0u32;
                buffer.Lock(&mut base, None, Some(&raw mut len)).at("a-lock")?;
                let floats = std::slice::from_raw_parts(base.cast::<f32>(), len as usize / 4);
                total += (floats.len() / ch as usize) as u64;
                let room = one_second.saturating_sub(first.len());
                first.extend_from_slice(&floats[..room.min(floats.len())]);
                let _ = buffer.Unlock();
            }
            if flags & FLAG_END != 0 {
                break;
            }
        }
        out(&format!("S a-first-read {}", first_read.unwrap_or_else(|| "-".into())));
        transforms(&reader, FIRST_AUDIO, "a");

        let bytes: Vec<u8> = first.iter().flat_map(|f| f.to_le_bytes()).collect();
        let hash = hex(&Sha256::digest(&bytes));
        // Channel 0's RMS, the RMS of its first difference over it (high frequencies raise it: a 440 Hz tone alone
        // gives ~0.06, the fixtures' 14 kHz tone pushes it far up; an HE-AAC decode without SBR loses it), and the
        // RMS of left minus right over left (0 when the two channels are the same, e.g. HE-AAC v2 without PS).
        let left: Vec<f32> = first.iter().step_by(ch as usize).copied().collect();
        let rms = |v: &[f32]| -> f64 {
            if v.is_empty() {
                0.0
            } else {
                (v.iter().map(|x| (*x as f64) * (*x as f64)).sum::<f64>() / v.len() as f64).sqrt()
            }
        };
        let level = rms(&left);
        let diff: Vec<f32> = left.windows(2).map(|w| w[1] - w[0]).collect();
        let hf = if level > 0.0 { rms(&diff) / level } else { 0.0 };
        let lr = if ch >= 2 && level > 0.0 {
            let d: Vec<f32> = first.chunks(ch as usize).map(|c| c[0] - c[1]).collect();
            rms(&d) / level
        } else {
            0.0
        };
        let state = if failure.is_none() && total > 0 { "ok" } else { "fail" };
        out(&format!(
            "A {state} {} native={tag} rate={rate} ch={ch} samples={total} rms={level:.6} hf={hf:.4} lr={lr:.4} hash={hash}",
            failure.unwrap_or_else(|| "-".into()),
        ));
    }
    Ok(())
}

/// A D3D11 device and a DXGI device manager for the hardware column.
fn d3d_manager() -> Result<IMFDXGIDeviceManager, String> {
    unsafe {
        let mut device: Option<ID3D11Device> = None;
        let mut flags: D3D11_CREATE_DEVICE_FLAG = D3D11_CREATE_DEVICE_VIDEO_SUPPORT | D3D11_CREATE_DEVICE_BGRA_SUPPORT;
        let mut created = D3D11CreateDevice(
            None::<&IDXGIAdapter>,
            D3D_DRIVER_TYPE_HARDWARE,
            HMODULE::default(),
            flags,
            None,
            D3D11_SDK_VERSION,
            Some(&raw mut device),
            None,
            None,
        );
        out(&format!(
            "S d3d11-video-support {}",
            created.as_ref().map(|_| "0x00000000".to_string()).unwrap_or_else(ex)
        ));
        if created.is_err() {
            flags = D3D11_CREATE_DEVICE_BGRA_SUPPORT;
            created = D3D11CreateDevice(
                None::<&IDXGIAdapter>,
                D3D_DRIVER_TYPE_HARDWARE,
                HMODULE::default(),
                flags,
                None,
                D3D11_SDK_VERSION,
                Some(&raw mut device),
                None,
                None,
            );
            out(&format!(
                "S d3d11-plain {}",
                created.as_ref().map(|_| "0x00000000".to_string()).unwrap_or_else(ex)
            ));
        }
        let device = device.ok_or_else(|| "d3d11 no-device".to_string())?;
        let adapter = device
            .cast::<IDXGIDevice>()
            .and_then(|d| d.GetAdapter())
            .and_then(|a| a.GetDesc())
            .map(|desc| {
                let end = desc
                    .Description
                    .iter()
                    .position(|&c| c == 0)
                    .unwrap_or(desc.Description.len());
                format!(
                    "{} vendor=0x{:04X}",
                    String::from_utf16_lossy(&desc.Description[..end]),
                    desc.VendorId
                )
            })
            .unwrap_or_else(|e| format!("err{}", ex(&e)));
        out(&format!("S d3d11-adapter {adapter}"));
        if let Ok(multithread) = device.cast::<ID3D11Multithread>() {
            let _ = multithread.SetMultithreadProtected(true);
        }
        let mut token = 0u32;
        let mut manager: Option<IMFDXGIDeviceManager> = None;
        MFCreateDXGIDeviceManager(&mut token, &mut manager).at("dxgi-manager")?;
        let manager = manager.ok_or_else(|| "dxgi-manager null".to_string())?;
        manager.ResetDevice(&device, token).at("dxgi-reset")?;
        out("S dxgi-manager 0x00000000");
        Ok(manager)
    }
}

/// `X`: which registry keys this process can open for reading (an AppContainer that is a member of the "all
/// application packages" group can read much of the machine's registry; a less privileged one cannot).
fn registry_reads() {
    use windows::Win32::System::Registry::{
        HKEY, HKEY_CLASSES_ROOT, HKEY_CURRENT_USER, HKEY_LOCAL_MACHINE, KEY_READ, RegCloseKey, RegOpenKeyExW,
    };
    let keys: [(&str, HKEY, &str); 6] = [
        (
            "hklm-currentversion",
            HKEY_LOCAL_MACHINE,
            r"SOFTWARE\Microsoft\Windows NT\CurrentVersion",
        ),
        (
            "hklm-mf",
            HKEY_LOCAL_MACHINE,
            r"SOFTWARE\Microsoft\Windows Media Foundation",
        ),
        (
            "hklm-uninstall",
            HKEY_LOCAL_MACHINE,
            r"SOFTWARE\Microsoft\Windows\CurrentVersion\Uninstall",
        ),
        ("hkcr-clsid", HKEY_CLASSES_ROOT, "CLSID"),
        (
            "hkcu-explorer",
            HKEY_CURRENT_USER,
            r"Software\Microsoft\Windows\CurrentVersion\Explorer",
        ),
        ("hkcu-environment", HKEY_CURRENT_USER, "Environment"),
    ];
    for (label, root, path) in keys {
        let wide: Vec<u16> = path.encode_utf16().chain(std::iter::once(0)).collect();
        let mut key = HKEY(null_mut());
        let code = unsafe { RegOpenKeyExW(root, PCWSTR(wide.as_ptr()), Some(0), KEY_READ, &mut key) };
        if code.0 == 0 {
            unsafe {
                let _ = RegCloseKey(key);
            }
            out(&format!("X {label} ok"));
        } else {
            out(&format!("X {label} err {}", code.0));
        }
    }
}

/// A video or audio subtype GUID from its four-character code (the standard media subtype base).
fn subtype_from_fourcc(code: &[u8; 4]) -> GUID {
    GUID::from_values(
        u32::from_le_bytes(*code),
        0x0000,
        0x0010,
        [0x80, 0x00, 0x00, 0xaa, 0x00, 0x38, 0x9b, 0x71],
    )
}

/// The decoders Media Foundation lists from inside this process: per category, and per input subtype (the lookup
/// the helper would make to say "no decoder for this format on this PC" before it opens anything).
fn enumerate() {
    let both = MFT_ENUM_FLAG(MFT_ENUM_FLAG_ALL.0 | MFT_ENUM_FLAG_UNTRUSTED_STOREMFT.0);
    let sorted = MFT_ENUM_FLAG(MFT_ENUM_FLAG_SYNCMFT.0 | MFT_ENUM_FLAG_ASYNCMFT.0 | MFT_ENUM_FLAG_SORTANDFILTER.0);
    let flag_sets = [("all", MFT_ENUM_FLAG_ALL), ("all+store", both), ("default", sorted)];
    let video = MFMediaType_Video;
    let audio = MFMediaType_Audio;
    let inputs: [(&str, GUID, GUID, GUID); 18] = [
        ("hevc", MFT_CATEGORY_VIDEO_DECODER, video, MFVideoFormat_HEVC),
        ("h264", MFT_CATEGORY_VIDEO_DECODER, video, MFVideoFormat_H264),
        ("wmv1", MFT_CATEGORY_VIDEO_DECODER, video, MFVideoFormat_WMV1),
        ("wmv2", MFT_CATEGORY_VIDEO_DECODER, video, MFVideoFormat_WMV2),
        ("wmv3", MFT_CATEGORY_VIDEO_DECODER, video, MFVideoFormat_WMV3),
        ("wvc1", MFT_CATEGORY_VIDEO_DECODER, video, MFVideoFormat_WVC1),
        ("mp4v", MFT_CATEGORY_VIDEO_DECODER, video, MFVideoFormat_MP4V),
        ("m4s2", MFT_CATEGORY_VIDEO_DECODER, video, MFVideoFormat_M4S2),
        ("xvid", MFT_CATEGORY_VIDEO_DECODER, video, subtype_from_fourcc(b"XVID")),
        ("mp43", MFT_CATEGORY_VIDEO_DECODER, video, MFVideoFormat_MP43),
        ("mpeg2", MFT_CATEGORY_VIDEO_DECODER, video, MFVideoFormat_MPEG2),
        ("vp90", MFT_CATEGORY_VIDEO_DECODER, video, MFVideoFormat_VP90),
        ("av1", MFT_CATEGORY_VIDEO_DECODER, video, MFVideoFormat_AV1),
        ("aac", MFT_CATEGORY_AUDIO_DECODER, audio, MFAudioFormat_AAC),
        ("mp3", MFT_CATEGORY_AUDIO_DECODER, audio, MFAudioFormat_MP3),
        ("wma2", MFT_CATEGORY_AUDIO_DECODER, audio, MFAudioFormat_WMAudioV8),
        ("wmapro", MFT_CATEGORY_AUDIO_DECODER, audio, MFAudioFormat_WMAudioV9),
        (
            "wmalossless",
            MFT_CATEGORY_AUDIO_DECODER,
            audio,
            MFAudioFormat_WMAudio_Lossless,
        ),
    ];
    for (name, category, major, subtype) in inputs {
        let info = MFT_REGISTER_TYPE_INFO {
            guidMajorType: major,
            guidSubtype: subtype,
        };
        for (label, flags) in flag_sets {
            unsafe {
                let mut list: *mut Option<IMFActivate> = null_mut();
                let mut count = 0u32;
                match MFTEnumEx(category, flags, Some(&raw const info), None, &mut list, &mut count) {
                    Err(e) => out(&format!("H {name} {label} fail {}", ex(&e))),
                    Ok(()) => {
                        let mut names = Vec::new();
                        if !list.is_null() {
                            let entries = std::slice::from_raw_parts_mut(list, count as usize);
                            for entry in entries.iter_mut() {
                                if let Some(activate) = entry.take() {
                                    names.push(friendly_name(&activate));
                                }
                            }
                            CoTaskMemFree(Some(list as *const _));
                        }
                        out(&format!("H {name} {label} n={count} {}", names.join("|")));
                    }
                }
            }
        }
    }
    for (name, category) in [
        ("video", MFT_CATEGORY_VIDEO_DECODER),
        ("audio", MFT_CATEGORY_AUDIO_DECODER),
    ] {
        for (label, flags) in flag_sets {
            unsafe {
                let mut list: *mut Option<IMFActivate> = null_mut();
                let mut count = 0u32;
                match MFTEnumEx(category, flags, None, None, &mut list, &mut count) {
                    Err(e) => out(&format!("E {name} {label} fail {}", ex(&e))),
                    Ok(()) => {
                        let mut names = Vec::new();
                        if !list.is_null() {
                            let entries = std::slice::from_raw_parts_mut(list, count as usize);
                            for entry in entries.iter_mut() {
                                if let Some(activate) = entry.take() {
                                    names.push(friendly_name(&activate));
                                }
                            }
                            CoTaskMemFree(Some(list as *const _));
                        }
                        out(&format!("E {name} {label} n={count} {}", names.join("|")));
                    }
                }
            }
        }
    }
}

fn friendly_name(activate: &IMFActivate) -> String {
    unsafe {
        let mut text = PWSTR::null();
        let mut len = 0u32;
        if activate
            .GetAllocatedString(&MFT_FRIENDLY_NAME_Attribute, &mut text, &mut len)
            .is_ok()
            && !text.is_null()
        {
            let name = String::from_utf16_lossy(std::slice::from_raw_parts(text.0, len as usize));
            CoTaskMemFree(Some(text.0 as *const _));
            name.replace('|', "/")
        } else {
            "?".into()
        }
    }
}

fn main() {
    std::panic::set_hook(Box::new(|info| out(&format!("P {info}"))));
    let started = Instant::now();
    out(&token_line());
    let before = modules();

    let mut stdin = std::io::stdin().lock();
    let mut header = String::new();
    if let Err(e) = stdin.read_line(&mut header) {
        out(&format!("R error read-header {e}"));
        return;
    }
    let parts: Vec<&str> = header.split_whitespace().collect();
    if parts.as_slice() == ["X"] {
        // Registry reads need neither COM nor Media Foundation: answered also where those fail to start.
        registry_reads();
        out(&format!("END {}", started.elapsed().as_millis()));
        return;
    }

    unsafe {
        let hr = CoInitializeEx(None, COINIT_MULTITHREADED);
        out(&format!("S coinit {}", hx(hr)));
        let startup = MFStartup(MF_VERSION, MFSTARTUP_NOSOCKET);
        out(&format!(
            "S mfstartup {}",
            startup.as_ref().map(|_| "0x00000000".to_string()).unwrap_or_else(ex)
        ));
        if startup.is_err() {
            out("V fail mfstartup");
            out("A fail mfstartup");
            print_new_modules(&before);
            out(&format!("END {}", started.elapsed().as_millis()));
            return;
        }
    }

    match parts.as_slice() {
        ["E"] => enumerate(),
        ["F", ext, accel, want, len] => {
            let Ok(len) = len.parse::<usize>() else {
                out("R error length");
                return;
            };
            let mut data = vec![0u8; len];
            if let Err(e) = stdin.read_exact(&mut data) {
                out(&format!("R error read-body {e}"));
                return;
            }
            out(&format!("R ok bytes={len} ext={ext} accel={accel} want={want}"));
            let manager = if *accel == "hw" {
                match d3d_manager() {
                    Ok(m) => Some(m),
                    Err(line) => {
                        out(&format!("S hw-unavailable {line}"));
                        None
                    }
                }
            } else {
                None
            };
            if let Err(line) = video(&data, ext, want, manager.as_ref()) {
                out(&format!("V fail {line}"));
            }
            if *accel == "sw" {
                if let Err(line) = audio(&data, ext) {
                    out(&format!("A fail {line}"));
                }
            }
        }
        _ => out("R error request"),
    }

    print_new_modules(&before);
    unsafe {
        let _ = MFShutdown();
    }
    out(&format!("END {}", started.elapsed().as_millis()));
}
