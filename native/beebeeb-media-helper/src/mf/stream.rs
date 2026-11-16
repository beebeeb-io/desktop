//! The byte stream Media Foundation reads the file from: the app's ranges ([`PipeReader`], the same reader FilmCraft
//! uses), each read through the session's [`Gate`]. Read-only and seekable; nothing is ever written, and no byte
//! reaches a file.
#![allow(unsafe_code)]

use std::sync::{Arc, Mutex, MutexGuard};

use filmcraft_media::ByteReader;
use windows::Win32::Foundation::{E_FAIL, E_INVALIDARG, E_NOTIMPL, S_OK};
use windows::Win32::Media::MediaFoundation::{
    IMFAsyncCallback, IMFAsyncResult, IMFAttributes, IMFByteStream, IMFByteStream_Impl, MFBYTESTREAM_IS_READABLE,
    MFBYTESTREAM_IS_SEEKABLE, MFBYTESTREAM_SEEK_ORIGIN, msoBegin, msoCurrent,
};
use windows::core::{BOOL, GUID, IUnknown, Interface, Ref, implement};

use super::api::Api;
use super::gate::Gate;
use crate::read::PipeReader;

/// Where `BeginRead` leaves the byte count for `EndRead`: an attribute of the asynchronous result's object. A key of
/// this module's own.
const READ_COUNT: GUID = GUID::from_u128(0x6d1f0c2a_83b5_4f4e_9a51_3c0b6e2d7a11);

/// See the module documentation.
#[implement(IMFByteStream)]
pub(crate) struct PipeStream {
    reader: Arc<PipeReader>,
    gate: Arc<Gate>,
    api: &'static Api,
    position: Mutex<u64>,
}

impl PipeStream {
    pub(crate) fn new(reader: Arc<PipeReader>, gate: Arc<Gate>, api: &'static Api) -> PipeStream {
        PipeStream {
            reader,
            gate,
            api,
            position: Mutex::new(0),
        }
    }

    fn position(&self) -> MutexGuard<'_, u64> {
        self.position.lock().unwrap_or_else(|e| e.into_inner())
    }

    /// Up to `cb` bytes from the current position into `pb`, never past the file's end; moves the position on.
    fn read_into(&self, pb: *mut u8, cb: u32) -> windows::core::Result<u32> {
        let mut position = self.position();
        let len = self.reader.len();
        let start = (*position).min(len);
        let n = u64::from(cb).min(len - start) as usize;
        if n > 0 {
            if pb.is_null() {
                return Err(E_INVALIDARG.into());
            }
            // SAFETY: Media Foundation hands a buffer writable for `cb` bytes and does not touch it during the call;
            // `n <= cb`.
            let buf = unsafe { std::slice::from_raw_parts_mut(pb, n) };
            match self.gate.through(|| self.reader.read_at(start, buf)) {
                Some(Ok(())) => {}
                // The app could not read the range, the pipe ended, or the session is over.
                Some(Err(_)) | None => return Err(E_FAIL.into()),
            }
        }
        *position = start + n as u64;
        Ok(n as u32)
    }
}

impl IMFByteStream_Impl for PipeStream_Impl {
    fn GetCapabilities(&self) -> windows::core::Result<u32> {
        Ok(MFBYTESTREAM_IS_READABLE | MFBYTESTREAM_IS_SEEKABLE)
    }

    fn GetLength(&self) -> windows::core::Result<u64> {
        Ok(self.reader.len())
    }

    fn SetLength(&self, _qwlength: u64) -> windows::core::Result<()> {
        Err(E_NOTIMPL.into())
    }

    fn GetCurrentPosition(&self) -> windows::core::Result<u64> {
        Ok(*self.position())
    }

    fn SetCurrentPosition(&self, qwposition: u64) -> windows::core::Result<()> {
        *self.position() = qwposition.min(self.reader.len());
        Ok(())
    }

    fn IsEndOfStream(&self) -> windows::core::Result<BOOL> {
        Ok(BOOL::from(*self.position() >= self.reader.len()))
    }

    fn Read(&self, pb: *mut u8, cb: u32, pcbread: *mut u32) -> windows::core::Result<()> {
        let n = self.read_into(pb, cb)?;
        if !pcbread.is_null() {
            // SAFETY: a non-null place for the count, from the caller.
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
        // The read itself happens now, on the caller's thread (through the gate); its completion goes through Media
        // Foundation's work queue, as the interface wants. A failed read is the result's status, read by `EndRead`.
        let (count, status) = match self.read_into(pb, cb) {
            Ok(n) => (n, S_OK),
            Err(e) => (0, e.code()),
        };
        let store = self.api.attributes(1)?;
        // SAFETY: an attribute of a store of this function's own.
        unsafe { store.SetUINT32(&READ_COUNT, count)? };
        let result = self.api.async_result(&store, pcallback.as_ref(), punkstate.as_ref())?;
        // SAFETY: the status of a result object of this function's own.
        unsafe { result.SetStatus(status)? };
        self.api.invoke(&result)
    }

    fn EndRead(&self, presult: Ref<IMFAsyncResult>) -> windows::core::Result<u32> {
        let result = presult.ok()?;
        // SAFETY: plain calls on the result object `BeginRead` made, and on its attribute store.
        unsafe {
            result.GetStatus()?;
            let store: IMFAttributes = result.GetObject()?.cast()?;
            store.GetUINT32(&READ_COUNT)
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
        let len = self.reader.len();
        let mut position = self.position();
        let base = if seekorigin == msoBegin {
            0i64
        } else if seekorigin == msoCurrent {
            i64::try_from(*position).unwrap_or(i64::MAX)
        } else {
            return Err(E_INVALIDARG.into());
        };
        let target = base
            .saturating_add(llseekoffset)
            .clamp(0, i64::try_from(len).unwrap_or(i64::MAX)) as u64;
        *position = target;
        Ok(target)
    }

    fn Flush(&self) -> windows::core::Result<()> {
        Ok(())
    }

    fn Close(&self) -> windows::core::Result<()> {
        Ok(())
    }
}

/// The stream as the interface Media Foundation takes.
pub(crate) fn byte_stream(reader: Arc<PipeReader>, gate: Arc<Gate>, api: &'static Api) -> IMFByteStream {
    PipeStream::new(reader, gate, api).into()
}
