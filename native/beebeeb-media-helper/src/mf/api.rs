//! Media Foundation's functions, loaded from the system folder the first time a `System` stream needs them, never as
//! imports of the helper's executable: on Windows N and KN editions without the Media Feature Pack the libraries do
//! not exist, and an import would stop the helper from starting at all. COM and Media Foundation are started once for
//! the process. Only interface methods (calls through an object's table) and the functions below are used; no
//! `windows::Win32::Media::MediaFoundation::MF*` free function is called anywhere (that would be an import again).
//!
//! The process never shuts Media Foundation down: the helper serves one file and ends.
#![allow(unsafe_code)]

use std::ffi::c_void;
use std::ptr::{null, null_mut};
use std::sync::OnceLock;

use windows::Win32::Foundation::{E_POINTER, ERROR_MOD_NOT_FOUND, HMODULE};
use windows::Win32::Media::MediaFoundation::{
    IMFAsyncCallback, IMFAsyncResult, IMFAttributes, IMFMediaSource, IMFMediaType, IMFSourceReader, IMFSourceResolver,
    MF_VERSION, MFSTARTUP_NOSOCKET,
};
use windows::Win32::System::LibraryLoader::{GetProcAddress, LOAD_LIBRARY_SEARCH_SYSTEM32, LoadLibraryExW};
use windows::core::{HRESULT, IUnknown, Interface, PCSTR, PCWSTR};

use super::Unavailable;

/// `COINIT_MULTITHREADED`: Media Foundation's own threads call this process's byte stream.
const MULTITHREADED: u32 = 0;

type CoInitializeEx = unsafe extern "system" fn(*const c_void, u32) -> HRESULT;
type MfStartup = unsafe extern "system" fn(u32, u32) -> HRESULT;
type CreateAttributes = unsafe extern "system" fn(*mut *mut c_void, u32) -> HRESULT;
type CreateMediaType = unsafe extern "system" fn(*mut *mut c_void) -> HRESULT;
type CreateAsyncResult = unsafe extern "system" fn(*mut c_void, *mut c_void, *mut c_void, *mut *mut c_void) -> HRESULT;
type InvokeCallback = unsafe extern "system" fn(*mut c_void) -> HRESULT;
type CreateSourceResolver = unsafe extern "system" fn(*mut *mut c_void) -> HRESULT;
type CreateReaderFromSource = unsafe extern "system" fn(*mut c_void, *mut c_void, *mut *mut c_void) -> HRESULT;

/// The Media Foundation functions this module calls, once it has started.
pub(crate) struct Api {
    create_attributes: CreateAttributes,
    create_media_type: CreateMediaType,
    create_async_result: CreateAsyncResult,
    invoke_callback: InvokeCallback,
    create_source_resolver: CreateSourceResolver,
    create_reader_from_source: CreateReaderFromSource,
}

static STARTED: OnceLock<Result<Api, Unavailable>> = OnceLock::new();

/// The functions, after loading and starting Media Foundation once (on the first caller's thread: the session's).
pub(crate) fn api() -> Result<&'static Api, Unavailable> {
    match STARTED.get_or_init(start) {
        Ok(api) => Ok(api),
        Err(why) => Err(*why),
    }
}

/// A library of the system folder; only that folder is searched, so nothing else is ever loaded by this name.
fn library(name: &str) -> Result<HMODULE, Unavailable> {
    let wide: Vec<u16> = name.encode_utf16().chain([0]).collect();
    // SAFETY: a terminated UTF-16 name that lives past the call; no file handle; search the system folder only.
    match unsafe { LoadLibraryExW(PCWSTR(wide.as_ptr()), None, LOAD_LIBRARY_SEARCH_SYSTEM32) } {
        Ok(module) => Ok(module),
        Err(e) if e.code() == HRESULT::from_win32(ERROR_MOD_NOT_FOUND.0) => Err(Unavailable::NoSystemMedia),
        Err(_) => Err(Unavailable::NotHere),
    }
}

/// The export `$name` of `$module` as the function type `$ty`.
macro_rules! export {
    ($module:expr, $name:literal, $ty:ty) => {{
        // SAFETY: a terminated ASCII name, looked up in a library this process loaded.
        match unsafe { GetProcAddress($module, PCSTR(concat!($name, "\0").as_ptr())) } {
            // SAFETY: Windows documents this export with exactly the signature `$ty` describes.
            Some(found) => Ok(unsafe { std::mem::transmute::<unsafe extern "system" fn() -> isize, $ty>(found) }),
            None => Err(Unavailable::NotHere),
        }
    }};
}

fn start() -> Result<Api, Unavailable> {
    // The Media Feature Pack's libraries first: on an N or KN edition without it they do not exist.
    let mfplat = library("mfplat.dll")?;
    let readwrite = library("mfreadwrite.dll")?;
    let ole32 = library("ole32.dll").map_err(|_| Unavailable::NotHere)?;
    // The COM libraries are delay-loaded imports (`build.rs`): load them from the system folder now, so the first
    // call through an import finds them already here and never searches for them.
    library("combase.dll").map_err(|_| Unavailable::NotHere)?;
    library("oleaut32.dll").map_err(|_| Unavailable::NotHere)?;
    let co_initialize = export!(ole32, "CoInitializeEx", CoInitializeEx)?;
    let startup = export!(mfplat, "MFStartup", MfStartup)?;
    let api = Api {
        create_attributes: export!(mfplat, "MFCreateAttributes", CreateAttributes)?,
        create_media_type: export!(mfplat, "MFCreateMediaType", CreateMediaType)?,
        create_async_result: export!(mfplat, "MFCreateAsyncResult", CreateAsyncResult)?,
        invoke_callback: export!(mfplat, "MFInvokeCallback", InvokeCallback)?,
        create_source_resolver: export!(mfplat, "MFCreateSourceResolver", CreateSourceResolver)?,
        create_reader_from_source: export!(readwrite, "MFCreateSourceReaderFromMediaSource", CreateReaderFromSource)?,
    };
    // SAFETY: no reserved pointer; this thread joins the process's multithreaded apartment. The sandbox's default
    // profile refuses this (0x80070005, measured): Windows' decoders then do not run here.
    // Measured on VM 401 (ux-57, 2026-11-21): the default profile already fails at `ole32.dll` above, and the system
    // decoder's profile without the HEVC capabilities (`registryRead` alone) was refused HERE, so Media Foundation
    // did not start in it. That profile now holds `lpacCom` too (Guus, 2026-11-21), so COM starts there; where it
    // still does not, the session plays a video of ours without that sound (`routes::settled`).
    if unsafe { co_initialize(null(), MULTITHREADED) }.is_err() {
        return Err(Unavailable::NotHere);
    }
    // SAFETY: the version these bindings describe; no sockets (the helper has no network).
    if unsafe { startup(MF_VERSION, MFSTARTUP_NOSOCKET) }.is_err() {
        return Err(Unavailable::NotHere);
    }
    Ok(api)
}

/// Takes over the one reference a creating function handed out.
fn owned<T: Interface>(raw: *mut c_void) -> windows::core::Result<T> {
    if raw.is_null() {
        return Err(E_POINTER.into());
    }
    // SAFETY: a non-null pointer to an object of interface `T`, whose reference is now this value's.
    Ok(unsafe { T::from_raw(raw) })
}

impl Api {
    /// An empty attribute store.
    pub(crate) fn attributes(&self, size: u32) -> windows::core::Result<IMFAttributes> {
        let mut raw = null_mut();
        // SAFETY: `raw` receives the new store.
        unsafe { (self.create_attributes)(&mut raw, size) }.ok()?;
        owned(raw)
    }

    /// An empty media type.
    pub(crate) fn media_type(&self) -> windows::core::Result<IMFMediaType> {
        let mut raw = null_mut();
        // SAFETY: `raw` receives the new media type.
        unsafe { (self.create_media_type)(&mut raw) }.ok()?;
        owned(raw)
    }

    /// The source resolver, which finds the container reader for a byte stream.
    pub(crate) fn resolver(&self) -> windows::core::Result<IMFSourceResolver> {
        let mut raw = null_mut();
        // SAFETY: `raw` receives the new resolver.
        unsafe { (self.create_source_resolver)(&mut raw) }.ok()?;
        owned(raw)
    }

    /// A source reader over `source`.
    pub(crate) fn reader(
        &self,
        source: &IMFMediaSource,
        attributes: &IMFAttributes,
    ) -> windows::core::Result<IMFSourceReader> {
        let mut raw = null_mut();
        // SAFETY: both objects are alive for the call (the reader takes its own references); `raw` receives it.
        unsafe { (self.create_reader_from_source)(source.as_raw(), attributes.as_raw(), &mut raw) }.ok()?;
        owned(raw)
    }

    /// The result object of an asynchronous read: `object` carries the answer, `callback` is told, `state` is the
    /// caller's.
    pub(crate) fn async_result(
        &self,
        object: &impl Interface,
        callback: Option<&IMFAsyncCallback>,
        state: Option<&IUnknown>,
    ) -> windows::core::Result<IMFAsyncResult> {
        let mut raw = null_mut();
        let callback = callback.map_or(null_mut(), Interface::as_raw);
        let state = state.map_or(null_mut(), Interface::as_raw);
        // SAFETY: every object is alive for the call (the result takes its own references); a COM interface pointer
        // is a valid `IUnknown` pointer; `raw` receives the result.
        unsafe { (self.create_async_result)(object.as_raw(), callback, state, &mut raw) }.ok()?;
        owned(raw)
    }

    /// Hands `result` to its callback on Media Foundation's own work queue.
    pub(crate) fn invoke(&self, result: &IMFAsyncResult) -> windows::core::Result<()> {
        // SAFETY: a live result object made by `async_result`.
        unsafe { (self.invoke_callback)(result.as_raw()) }.ok()
    }
}
