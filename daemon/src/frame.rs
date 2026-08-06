//! Frame codec — 4-byte big-endian uint32 length + UTF-8 JSON payload (`IPC-01`).

use std::io::{self, Read, Write};

use crate::error::{ErrorCode, OxideError};

/// `IPC-02` / `IPC-05`: both sides' hard frame limit.
pub const MAX_FRAME: u32 = 64 * 1024 * 1024; // 64 MiB

/// Reads exactly one frame. Returns `Ok(None)` on clean EOF with no partial data.
///
/// Violations mapped per 001:
/// - declared length > `MAX_FRAME` → `FRAME_TOO_LARGE`, connection must be closed (`IPC-02`)
/// - body length ≠ declared length → the caller closes the connection (`IPC-03`)
pub fn read_frame<R: Read>(reader: &mut R) -> Result<Option<Vec<u8>>, OxideError> {
    let mut len_buf = [0u8; 4];
    match reader.read_exact(&mut len_buf) {
        Ok(()) => {}
        Err(e) if e.kind() == io::ErrorKind::UnexpectedEof => return Ok(None),
        Err(e) => return Err(OxideError::internal(e.to_string())),
    }
    let declared = u32::from_be_bytes(len_buf);
    if declared > MAX_FRAME {
        return Err(OxideError::new(
            ErrorCode::FrameTooLarge,
            format!("frame declares {declared} bytes; max is {MAX_FRAME}"),
        ));
    }
    let mut body = vec![0u8; declared as usize];
    reader
        .read_exact(&mut body)
        .map_err(|_| {
            OxideError::internal("connection closed mid-frame; body length ≠ declared length")
        })
        .map(|()| Some(body))
}

/// `IPC-03` on the client side: always writes `len == body.len()`.
pub fn write_frame<W: Write>(writer: &mut W, body: &[u8]) -> io::Result<()> {
    let len = u32::try_from(body.len())
        .map_err(|_| io::Error::new(io::ErrorKind::InvalidInput, "frame exceeds u32::MAX"))?;
    writer.write_all(&len.to_be_bytes())?;
    writer.write_all(body)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ipc_01_frame_roundtrip() {
        let mut buf = Vec::new();
        write_frame(&mut buf, b"{\"hello\":1}").unwrap();
        assert_eq!(&buf[..4], &[0, 0, 0, 11]);
        let mut cursor = std::io::Cursor::new(buf);
        let body = read_frame(&mut cursor).unwrap().unwrap();
        assert_eq!(body, b"{\"hello\":1}");
    }

    #[test]
    fn ipc_02_oversized_frame_rejected() {
        let mut buf = Vec::new();
        buf.extend_from_slice(&(MAX_FRAME + 1).to_be_bytes());
        buf.extend_from_slice(&[0u8; 16]);
        let mut cursor = std::io::Cursor::new(buf);
        let err = read_frame(&mut cursor).unwrap_err();
        assert_eq!(err.code, ErrorCode::FrameTooLarge);
    }

    #[test]
    fn ipc_03_truncated_body_is_error() {
        let mut buf = Vec::new();
        buf.extend_from_slice(&8u32.to_be_bytes());
        buf.extend_from_slice(b"abc");
        let mut cursor = std::io::Cursor::new(buf);
        assert!(read_frame(&mut cursor).is_err());
    }

    #[test]
    fn clean_eof_is_none_not_error() {
        let mut cursor = std::io::Cursor::new(Vec::<u8>::new());
        assert!(read_frame(&mut cursor).unwrap().is_none());
    }
}
