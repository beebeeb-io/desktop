//! Fixed little-endian fields and length-prefixed UTF-8 for the Office messages; every declared length is checked
//! against the bytes that remain before anything is allocated.

use crate::frame::fits_one_frame;
use crate::office::{DecodeError, EncodeError};

/// Writes fields in order.
pub(crate) struct Out(Vec<u8>);

impl Out {
    pub(crate) fn new() -> Out {
        Out(Vec::new())
    }

    pub(crate) fn u8(&mut self, v: u8) {
        self.0.push(v);
    }

    pub(crate) fn u16(&mut self, v: u16) {
        self.0.extend_from_slice(&v.to_le_bytes());
    }

    pub(crate) fn u32(&mut self, v: u32) {
        self.0.extend_from_slice(&v.to_le_bytes());
    }

    pub(crate) fn u64(&mut self, v: u64) {
        self.0.extend_from_slice(&v.to_le_bytes());
    }

    pub(crate) fn f32(&mut self, v: f32) {
        self.0.extend_from_slice(&v.to_le_bytes());
    }

    pub(crate) fn f64(&mut self, v: f64) {
        self.0.extend_from_slice(&v.to_le_bytes());
    }

    pub(crate) fn bool(&mut self, v: bool) {
        self.0.push(u8::from(v));
    }

    /// A `u32` count for a vector. Anything near `u32::MAX` items could never fit a frame; the count saturates and the
    /// caller writes at most that many items, so the bytes always agree with the count.
    pub(crate) fn count(&mut self, len: usize) -> usize {
        let n = u32::try_from(len).unwrap_or(u32::MAX);
        self.u32(n);
        n as usize
    }

    /// A `u32` byte length, then the UTF-8 bytes.
    pub(crate) fn str(&mut self, s: &str) {
        let n = self.count(s.len());
        self.0.extend_from_slice(&s.as_bytes()[..n]);
    }

    /// The payload, or [`EncodeError::OverFrame`] when it would not fit one frame: the check
    /// [`crate::frame::write_frame`] holds.
    pub(crate) fn finish(self) -> Result<Vec<u8>, EncodeError> {
        if fits_one_frame(self.0.len()) {
            Ok(self.0)
        } else {
            Err(EncodeError::OverFrame)
        }
    }
}

/// Reads fields in order from a payload; never panics, never allocates more than the payload holds.
pub(crate) struct In<'a> {
    rest: &'a [u8],
}

impl<'a> In<'a> {
    pub(crate) fn new(payload: &'a [u8]) -> In<'a> {
        In { rest: payload }
    }

    fn take<const N: usize>(&mut self) -> Result<[u8; N], DecodeError> {
        let Some((head, tail)) = self.rest.split_first_chunk::<N>() else {
            return Err(DecodeError::Truncated);
        };
        self.rest = tail;
        Ok(*head)
    }

    pub(crate) fn u8(&mut self) -> Result<u8, DecodeError> {
        Ok(self.take::<1>()?[0])
    }

    pub(crate) fn u16(&mut self) -> Result<u16, DecodeError> {
        Ok(u16::from_le_bytes(self.take()?))
    }

    pub(crate) fn u32(&mut self) -> Result<u32, DecodeError> {
        Ok(u32::from_le_bytes(self.take()?))
    }

    pub(crate) fn u64(&mut self) -> Result<u64, DecodeError> {
        Ok(u64::from_le_bytes(self.take()?))
    }

    /// A finite `f32`: NaN and the infinities are refused here, so no later arithmetic sees one.
    pub(crate) fn f32(&mut self) -> Result<f32, DecodeError> {
        let v = f32::from_le_bytes(self.take()?);
        if v.is_finite() {
            Ok(v)
        } else {
            Err(DecodeError::NotFinite)
        }
    }

    /// A finite `f64`, by the same rule as [`In::f32`].
    pub(crate) fn f64(&mut self) -> Result<f64, DecodeError> {
        let v = f64::from_le_bytes(self.take()?);
        if v.is_finite() {
            Ok(v)
        } else {
            Err(DecodeError::NotFinite)
        }
    }

    /// 0 or 1; anything else is refused.
    pub(crate) fn bool(&mut self) -> Result<bool, DecodeError> {
        match self.u8()? {
            0 => Ok(false),
            1 => Ok(true),
            _ => Err(DecodeError::BadTag),
        }
    }

    /// A declared count of items that each take at least `min_item_bytes` (≥ 1) on the wire. Refused when the items
    /// could not fit in what remains, so a caller may allocate the returned count: it is bounded by the payload.
    pub(crate) fn count(&mut self, min_item_bytes: usize) -> Result<usize, DecodeError> {
        let declared = self.u32()? as usize;
        match declared.checked_mul(min_item_bytes.max(1)) {
            Some(need) if need <= self.rest.len() => Ok(declared),
            _ => Err(DecodeError::LengthPastEnd),
        }
    }

    /// A `u32` byte length, then that many bytes of UTF-8. The length is checked before anything is copied.
    pub(crate) fn str(&mut self) -> Result<String, DecodeError> {
        let declared = self.u32()? as usize;
        if declared > self.rest.len() {
            return Err(DecodeError::LengthPastEnd);
        }
        let (bytes, tail) = self.rest.split_at(declared);
        self.rest = tail;
        std::str::from_utf8(bytes)
            .map(str::to_owned)
            .map_err(|_| DecodeError::BadUtf8)
    }

    /// Every byte must have been used.
    pub(crate) fn finish(self) -> Result<(), DecodeError> {
        if self.rest.is_empty() {
            Ok(())
        } else {
            Err(DecodeError::TrailingBytes)
        }
    }
}
