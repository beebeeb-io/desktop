//! Wire framing for the desktop daemon's Unix-socket IPC (task 1670 issue 3).
//!
//! The full protocol contract lives in `docs/IPC_PROTOCOL.md`; this module is
//! the Rust half of the framing. The Swift half is
//! `BeebeebFileProvider/IPCFraming.swift` — keep the two in step.
//!
//! **Frame = one compact JSON value followed by a single `\n` (0x0A).**
//! `serde_json::to_vec` (compact) escapes every control character inside a
//! string as `\n`/`\u000a`, so a raw 0x0A byte can only ever be the delimiter.
//!
//! Before this module the daemon treated one `read()` of at most 64 KiB as one
//! whole request and wrote replies with no delimiter on a connection it kept
//! open. On a `SOCK_STREAM` socket neither holds: a request may arrive in
//! several reads, and a client cannot know where a reply ends.
//!
//! Version-skew tolerance (an app update can briefly pair an old extension
//! with a new daemon): [`FrameReader::next_frame`] also accepts a buffer that
//! is already one complete JSON value even though no newline has arrived (the
//! old extension wrote its request in one `write()` with no delimiter and
//! then waited for the reply), and a final unterminated value at EOF.

#![cfg(unix)]

use tokio::io::{AsyncRead, AsyncReadExt, AsyncWrite, AsyncWriteExt};

/// The frame delimiter.
pub const FRAME_DELIMITER: u8 = b'\n';

/// Largest request the daemon will buffer. Real requests are a few hundred
/// bytes (paths and ids); 1 MiB is far above any legitimate one and keeps an
/// unauthenticated-looking peer from making the daemon buffer without bound.
pub const MAX_REQUEST_BYTES: usize = 1024 * 1024;

const READ_CHUNK: usize = 64 * 1024;

#[derive(Debug)]
pub enum FrameError {
    /// More than the configured maximum arrived without a complete frame. The
    /// stream cannot be resynchronised, so the caller must reply (if it can)
    /// and close the connection.
    TooLarge { limit: usize },
    Io(std::io::Error),
}

impl std::fmt::Display for FrameError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            FrameError::TooLarge { limit } => write!(f, "message exceeds the {limit}-byte limit"),
            FrameError::Io(e) => write!(f, "{e}"),
        }
    }
}

/// Buffered reader that yields one delimited frame at a time.
pub struct FrameReader<R> {
    inner: R,
    buf: Vec<u8>,
    max: usize,
}

impl<R: AsyncRead + Unpin> FrameReader<R> {
    pub fn new(inner: R, max: usize) -> Self {
        Self {
            inner,
            buf: Vec::new(),
            max,
        }
    }

    /// Read once from the peer into the internal buffer. Returns the number of
    /// bytes read (0 = peer closed). Cancel-safe: dropping the future loses no
    /// data, so it may be raced inside `tokio::select!` (the hydrate handler
    /// uses it to notice the client hanging up mid-download).
    pub async fn fill_more(&mut self) -> std::io::Result<usize> {
        let mut chunk = vec![0u8; READ_CHUNK];
        let n = self.inner.read(&mut chunk).await?;
        self.buf.extend_from_slice(&chunk[..n]);
        Ok(n)
    }

    /// Bytes currently buffered but not yet consumed as a frame.
    pub fn buffered_len(&self) -> usize {
        self.buf.len()
    }

    /// The next non-empty frame (without its delimiter), or `Ok(None)` on a
    /// clean EOF with nothing pending.
    pub async fn next_frame(&mut self) -> Result<Option<Vec<u8>>, FrameError> {
        loop {
            if let Some(pos) = self.buf.iter().position(|b| *b == FRAME_DELIMITER) {
                let mut line: Vec<u8> = self.buf.drain(..=pos).collect();
                line.pop(); // the delimiter
                if is_blank(&line) {
                    continue; // tolerate empty lines / stray "\r\n"
                }
                return Ok(Some(line));
            }
            // Legacy peer: a complete JSON value with no delimiter yet.
            if ends_like_json_value(&self.buf) && serde_json::from_slice::<serde_json::Value>(&self.buf).is_ok() {
                return Ok(Some(std::mem::take(&mut self.buf)));
            }
            if self.buf.len() > self.max {
                return Err(FrameError::TooLarge { limit: self.max });
            }
            let n = self.fill_more().await.map_err(FrameError::Io)?;
            if n == 0 {
                // EOF: hand back a final unterminated value if there is one
                // (the caller's JSON parse decides whether it is usable).
                if is_blank(&self.buf) {
                    self.buf.clear();
                    return Ok(None);
                }
                return Ok(Some(std::mem::take(&mut self.buf)));
            }
        }
    }
}

fn is_blank(bytes: &[u8]) -> bool {
    bytes.iter().all(|b| b.is_ascii_whitespace())
}

/// Cheap pre-filter so the (linear) JSON parse above only runs when the
/// buffer could be a finished value: every request is an object or a bare
/// string, so the last non-whitespace byte must be `}` or `"`.
fn ends_like_json_value(buf: &[u8]) -> bool {
    matches!(buf.iter().rev().find(|b| !b.is_ascii_whitespace()), Some(b'}') | Some(b'"'))
}

/// Serialise `value` compactly and write it as ONE frame (`json` + `\n`) in a
/// single `write_all`, then flush.
pub async fn write_frame<W: AsyncWrite + Unpin, T: serde::Serialize>(w: &mut W, value: &T) -> std::io::Result<()> {
    let mut bytes = serde_json::to_vec(value).map_err(std::io::Error::other)?;
    debug_assert!(
        !bytes.contains(&FRAME_DELIMITER),
        "compact JSON must never contain a raw newline"
    );
    bytes.push(FRAME_DELIMITER);
    w.write_all(&bytes).await?;
    w.flush().await
}

#[cfg(test)]
mod tests {
    use super::*;
    use tokio::io::AsyncWriteExt;

    fn rt() -> tokio::runtime::Runtime {
        tokio::runtime::Builder::new_current_thread().enable_all().build().unwrap()
    }

    /// serde_json's compact output is the reason a raw 0x0A can serve as the
    /// delimiter. Pin it against the nastiest string we could carry.
    #[test]
    fn compact_json_never_contains_a_raw_newline() {
        let nasty = serde_json::json!({
            "path": "a\nb\r\nc\u{2028}d\u{0}e",
            "nested": { "k": ["x\ny", "\n\n"] },
        });
        let bytes = serde_json::to_vec(&nasty).unwrap();
        assert!(!bytes.contains(&b'\n'), "raw newline leaked into compact JSON");
    }

    #[test]
    fn a_frame_split_across_two_reads_is_reassembled() {
        rt().block_on(async {
            let (mut tx, rx) = tokio::io::duplex(64);
            let mut reader = FrameReader::new(rx, MAX_REQUEST_BYTES);
            let writer = tokio::spawn(async move {
                tx.write_all(br#"{"GetFileStatus":{"file_"#).await.unwrap();
                tx.flush().await.unwrap();
                tokio::time::sleep(std::time::Duration::from_millis(30)).await;
                tx.write_all(b"id\":\"abc\"}}\n").await.unwrap();
            });
            let frame = reader.next_frame().await.unwrap().expect("one frame");
            assert_eq!(frame, br#"{"GetFileStatus":{"file_id":"abc"}}"#.to_vec());
            writer.await.unwrap();
        });
    }

    #[test]
    fn two_frames_in_one_read_come_out_one_at_a_time() {
        rt().block_on(async {
            let (mut tx, rx) = tokio::io::duplex(1024);
            let mut reader = FrameReader::new(rx, MAX_REQUEST_BYTES);
            tx.write_all(b"{\"a\":1}\n{\"b\":2}\n").await.unwrap();
            drop(tx);
            assert_eq!(reader.next_frame().await.unwrap().unwrap(), b"{\"a\":1}".to_vec());
            assert_eq!(reader.next_frame().await.unwrap().unwrap(), b"{\"b\":2}".to_vec());
            assert!(reader.next_frame().await.unwrap().is_none(), "clean EOF must be None");
        });
    }

    #[test]
    fn blank_lines_are_skipped() {
        rt().block_on(async {
            let (mut tx, rx) = tokio::io::duplex(1024);
            let mut reader = FrameReader::new(rx, MAX_REQUEST_BYTES);
            tx.write_all(b"\n\r\n{\"a\":1}\n").await.unwrap();
            drop(tx);
            assert_eq!(reader.next_frame().await.unwrap().unwrap(), b"{\"a\":1}".to_vec());
        });
    }

    /// Old extension: one write, no delimiter, then waits for the reply (does
    /// NOT half-close). The reader must not wait for a newline that never comes.
    #[test]
    fn an_unframed_complete_value_is_accepted_without_waiting_for_eof() {
        rt().block_on(async {
            let (mut tx, rx) = tokio::io::duplex(1024);
            let mut reader = FrameReader::new(rx, MAX_REQUEST_BYTES);
            tx.write_all(br#"{"ListFileProviderItems":{"container_id":"x"}}"#).await.unwrap();
            // tx stays open on purpose.
            let frame = tokio::time::timeout(std::time::Duration::from_secs(2), reader.next_frame())
                .await
                .expect("must not block waiting for a delimiter")
                .unwrap()
                .unwrap();
            assert_eq!(frame, br#"{"ListFileProviderItems":{"container_id":"x"}}"#.to_vec());
        });
    }

    #[test]
    fn a_partial_value_at_eof_is_returned_for_the_caller_to_reject() {
        rt().block_on(async {
            let (mut tx, rx) = tokio::io::duplex(1024);
            let mut reader = FrameReader::new(rx, MAX_REQUEST_BYTES);
            tx.write_all(br#"{"GetFile"#).await.unwrap();
            drop(tx);
            let frame = reader.next_frame().await.unwrap().unwrap();
            assert_eq!(frame, br#"{"GetFile"#.to_vec());
            assert!(serde_json::from_slice::<serde_json::Value>(&frame).is_err());
        });
    }

    #[test]
    fn an_oversized_frame_is_refused_instead_of_buffered_forever() {
        rt().block_on(async {
            let (mut tx, rx) = tokio::io::duplex(4096);
            let mut reader = FrameReader::new(rx, 1000);
            let writer = tokio::spawn(async move {
                // 3000 bytes, no newline, never valid JSON.
                let _ = tx.write_all(&vec![b'x'; 3000]).await;
                tokio::time::sleep(std::time::Duration::from_millis(200)).await;
            });
            match reader.next_frame().await {
                Err(FrameError::TooLarge { limit }) => assert_eq!(limit, 1000),
                other => panic!("expected TooLarge, got {other:?}"),
            }
            writer.await.unwrap();
        });
    }

    #[test]
    fn write_frame_appends_exactly_one_delimiter() {
        rt().block_on(async {
            let mut out: Vec<u8> = Vec::new();
            write_frame(&mut out, &serde_json::json!({"Ok": {}})).await.unwrap();
            assert_eq!(out, b"{\"Ok\":{}}\n".to_vec());
        });
    }
}
