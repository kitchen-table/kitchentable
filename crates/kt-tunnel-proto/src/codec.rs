//! Getting one message off a byte stream and knowing where it ended.
//!
//! Every message here is a big-endian `u32` length followed by that many bytes
//! of JSON.
//!
//! **JSON**, because the rest of this product's wire is JSON, because a tunnel
//! spanning two repositories, a NAT and a datacentre will be debugged by
//! reading it, and because the messages are handshakes and request heads whose
//! bytes are lost in the noise beside the bodies they introduce. A binary
//! encoding would save nothing anyone would notice and cost the ability to see
//! what happened.
//!
//! **Length-prefixed** rather than newline-delimited, which is what the daemon's
//! local socket uses, because here a message is followed immediately by opaque
//! bytes - a request body, a response body - on the same stream. A reader
//! scanning for a newline has to buffer ahead and then hand back what it
//! over-read; a reader told the length consumes exactly the message and leaves
//! the body untouched for whoever wants it. The two protocols differ because
//! the situations do.

use serde::{de::DeserializeOwned, Serialize};

/// The largest message this will read or write.
///
/// The length prefix arrives from the other end, so the first thing done with
/// it is refusing to believe it: a peer that says four gigabytes must not get
/// four gigabytes allocated on the strength of saying so. 64 KiB is generous
/// for the largest message here - a request head whose headers are mostly
/// cookies - and far below anything worth worrying about per stream.
pub const MAX_FRAME_LEN: u32 = 64 * 1024;

const PREFIX_LEN: usize = 4;

#[derive(Debug, thiserror::Error)]
pub enum CodecError {
    #[error("a {len} byte frame is over the {MAX_FRAME_LEN} byte limit")]
    TooLong { len: u32 },
    #[error("the frame is not valid JSON for this message: {0}")]
    Malformed(#[from] serde_json::Error),
}

/// A message, ready to write.
pub fn encode<T: Serialize>(message: &T) -> Result<Vec<u8>, CodecError> {
    let body = serde_json::to_vec(message)?;
    let len = u32::try_from(body.len()).unwrap_or(u32::MAX);
    if len > MAX_FRAME_LEN {
        return Err(CodecError::TooLong { len });
    }

    let mut frame = Vec::with_capacity(PREFIX_LEN + body.len());
    frame.extend_from_slice(&len.to_be_bytes());
    frame.extend_from_slice(&body);
    Ok(frame)
}

/// Read one message from the front of a buffer.
///
/// `Ok(None)` means the buffer holds part of a message and the caller should
/// read more; it is not an error and it is the usual answer on a live socket.
/// On success the second half of the tuple is how many bytes were consumed, so
/// the caller can advance past it - and anything left is the caller's, which on
/// a request stream is the start of the body.
///
/// Pure on purpose: this crate does no I/O, so the buffer and the socket both
/// stay the caller's business and this stays testable without either.
pub fn try_decode<T: DeserializeOwned>(buf: &[u8]) -> Result<Option<(T, usize)>, CodecError> {
    let Some(prefix) = buf.get(..PREFIX_LEN) else {
        return Ok(None);
    };
    let mut len_bytes = [0u8; PREFIX_LEN];
    len_bytes.copy_from_slice(prefix);
    let len = u32::from_be_bytes(len_bytes);

    // Before any allocation, and before waiting for bytes that must not be
    // waited for: an oversized length is refused on sight.
    if len > MAX_FRAME_LEN {
        return Err(CodecError::TooLong { len });
    }

    let end = PREFIX_LEN + len as usize;
    let Some(body) = buf.get(PREFIX_LEN..end) else {
        return Ok(None);
    };
    Ok(Some((serde_json::from_slice(body)?, end)))
}

/// Read one message from a buffer that should hold exactly one.
///
/// For the handshake, where each side sends a single message and waits. A
/// stream carrying a body wants [`try_decode`] instead.
pub fn decode<T: DeserializeOwned>(buf: &[u8]) -> Result<Option<T>, CodecError> {
    Ok(try_decode(buf)?.map(|(message, _)| message))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::control::ControlFrame;

    #[test]
    fn a_message_survives_the_trip() {
        let frame = ControlFrame::Ping { seq: 7 };
        let bytes = encode(&frame).unwrap();
        let (back, consumed) = try_decode::<ControlFrame>(&bytes).unwrap().unwrap();
        assert_eq!(back, frame);
        assert_eq!(consumed, bytes.len());
    }

    #[test]
    fn a_partial_message_asks_for_more_rather_than_failing() {
        let bytes = encode(&ControlFrame::Ping { seq: 7 }).unwrap();
        for cut in 0..bytes.len() {
            assert!(
                try_decode::<ControlFrame>(&bytes[..cut]).unwrap().is_none(),
                "{cut} bytes should have been treated as a short read"
            );
        }
    }

    #[test]
    fn what_follows_a_message_is_left_alone() {
        // The reason for a length prefix: a request head is followed by body
        // bytes on the same stream, and those must arrive untouched.
        let mut bytes = encode(&ControlFrame::Pong { seq: 1 }).unwrap();
        let head_len = bytes.len();
        bytes.extend_from_slice(b"\x00\x01 a body with \n newlines in it");

        let (frame, consumed) = try_decode::<ControlFrame>(&bytes).unwrap().unwrap();
        assert_eq!(frame, ControlFrame::Pong { seq: 1 });
        assert_eq!(consumed, head_len);
        assert_eq!(
            &bytes[consumed..],
            b"\x00\x01 a body with \n newlines in it"
        );
    }

    #[test]
    fn a_peer_cannot_ask_for_a_large_allocation() {
        // Refused on the strength of the prefix alone, with no body present and
        // nothing allocated.
        let mut bytes = u32::MAX.to_be_bytes().to_vec();
        bytes.extend_from_slice(b"{}");
        assert!(matches!(
            try_decode::<ControlFrame>(&bytes),
            Err(CodecError::TooLong { len }) if len == u32::MAX
        ));
    }

    #[test]
    fn the_limit_is_refused_from_both_directions() {
        let too_long = MAX_FRAME_LEN + 1;
        let bytes = too_long.to_be_bytes().to_vec();
        assert!(matches!(
            try_decode::<ControlFrame>(&bytes),
            Err(CodecError::TooLong { .. })
        ));

        let huge = ControlFrame::Goodbye {
            reason: crate::CloseReason::ProtocolError {
                detail: "x".repeat(MAX_FRAME_LEN as usize),
            },
        };
        assert!(matches!(encode(&huge), Err(CodecError::TooLong { .. })));
    }

    #[test]
    fn a_frame_of_the_right_length_and_the_wrong_shape_is_malformed() {
        let body = br#"{"control":"ping"}"#;
        let mut bytes = (body.len() as u32).to_be_bytes().to_vec();
        bytes.extend_from_slice(body);
        // Right length, valid JSON, but ping has no seq.
        assert!(matches!(
            try_decode::<ControlFrame>(&bytes),
            Err(CodecError::Malformed(_))
        ));
    }

    #[test]
    fn an_empty_buffer_is_a_short_read_and_not_an_error() {
        assert!(try_decode::<ControlFrame>(&[]).unwrap().is_none());
    }

    #[test]
    fn a_zero_length_frame_is_malformed_rather_than_a_short_read() {
        // Nothing here has an empty encoding, so this is a broken peer.
        let bytes = 0u32.to_be_bytes();
        assert!(matches!(
            try_decode::<ControlFrame>(&bytes),
            Err(CodecError::Malformed(_))
        ));
    }

    #[test]
    fn decode_reads_a_lone_message() {
        let bytes = encode(&ControlFrame::Ping { seq: 3 }).unwrap();
        assert_eq!(
            decode::<ControlFrame>(&bytes).unwrap(),
            Some(ControlFrame::Ping { seq: 3 })
        );
    }
}
