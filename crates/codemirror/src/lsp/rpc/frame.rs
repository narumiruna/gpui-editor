use std::{fmt, io};

use serde_json::Value;

const DELIMITER: &[u8] = b"\r\n\r\n";
const MIN_HEADER_BYTES: usize = b"Content-Length: 0\r\n\r\n".len();

/// An error in the LSP byte stream or its configured limits.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum FrameError {
    InvalidLimits,
    HeaderTooLarge,
    BodyTooLarge,
    InvalidHeader(&'static str),
    Truncated,
    Poisoned,
}

impl fmt::Display for FrameError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::InvalidLimits => f.write_str("invalid LSP frame limits"),
            Self::HeaderTooLarge => f.write_str("LSP frame header exceeds its byte limit"),
            Self::BodyTooLarge => f.write_str("LSP frame body exceeds its byte limit"),
            Self::InvalidHeader(reason) => write!(f, "invalid LSP frame header: {reason}"),
            Self::Truncated => f.write_str("LSP byte stream ended within a frame"),
            Self::Poisoned => f.write_str("LSP frame decoder cannot continue after an error"),
        }
    }
}

impl std::error::Error for FrameError {}

#[derive(Clone, Copy, Debug)]
enum DecodeState {
    Header,
    Body(usize),
    Failed,
}

/// Incremental LSP `Content-Length` framing for a byte stream, such as stdio.
///
/// The limits apply to each frame, not to the input chunk size. The decoder
/// buffers at most one header or body at a time. It ignores headers other than
/// `Content-Length`; the caller can decode the body as UTF-8 JSON.
#[derive(Debug)]
pub struct StdioCodec {
    max_header_bytes: usize,
    max_body_bytes: usize,
    state: DecodeState,
    buffer: Vec<u8>,
}

impl StdioCodec {
    /// Header limits include the `\r\n\r\n` delimiter. Body limits count bytes.
    pub fn new(max_header_bytes: usize, max_body_bytes: usize) -> Result<Self, FrameError> {
        if max_header_bytes < MIN_HEADER_BYTES
            || max_body_bytes == 0
            || max_header_bytes > (isize::MAX as usize).saturating_sub(max_body_bytes)
        {
            return Err(FrameError::InvalidLimits);
        }
        Ok(Self {
            max_header_bytes,
            max_body_bytes,
            state: DecodeState::Header,
            buffer: Vec::new(),
        })
    }

    /// Encode one JSON value as a UTF-8 LSP frame. This does not change the
    /// decoder state or validate the JSON-RPC envelope.
    pub fn encode(&self, value: &Value) -> Result<Vec<u8>, FrameError> {
        let mut body = BoundedBody {
            bytes: Vec::new(),
            max: self.max_body_bytes,
        };
        serde_json::to_writer(&mut body, value).map_err(|_| FrameError::BodyTooLarge)?;
        let header = format!("Content-Length: {}\r\n\r\n", body.bytes.len());
        if header.len() > self.max_header_bytes {
            return Err(FrameError::HeaderTooLarge);
        }
        let mut frame = Vec::with_capacity(header.len() + body.bytes.len());
        frame.extend_from_slice(header.as_bytes());
        frame.extend_from_slice(&body.bytes);
        Ok(frame)
    }

    /// Deliver complete bodies in stream order, including empty bodies. The
    /// caller can discard a malformed JSON body and continue at the next frame.
    /// A framing error stops decoding; use a new codec for a new stream. Bodies
    /// delivered before the error are still valid receive events.
    pub fn feed(&mut self, input: &[u8], on_body: impl FnMut(&[u8])) -> Result<(), FrameError> {
        if matches!(self.state, DecodeState::Failed) {
            return Err(FrameError::Poisoned);
        }
        let result = self.decode(input, on_body);
        if result.is_err() {
            self.state = DecodeState::Failed;
            self.buffer.clear();
        }
        result
    }

    /// Check for a partial frame when the transport reaches EOF.
    pub fn finish(&mut self) -> Result<(), FrameError> {
        match self.state {
            DecodeState::Header if self.buffer.is_empty() => Ok(()),
            DecodeState::Failed => Err(FrameError::Poisoned),
            _ => {
                self.state = DecodeState::Failed;
                self.buffer.clear();
                Err(FrameError::Truncated)
            }
        }
    }

    fn decode(
        &mut self,
        mut input: &[u8],
        mut on_body: impl FnMut(&[u8]),
    ) -> Result<(), FrameError> {
        while !input.is_empty() {
            match self.state {
                DecodeState::Header => {
                    self.buffer.push(input[0]);
                    input = &input[1..];
                    if let Some(header) = self.buffer.strip_suffix(DELIMITER) {
                        let length = content_length(header)?;
                        if length > self.max_body_bytes {
                            return Err(FrameError::BodyTooLarge);
                        }
                        self.buffer.clear();
                        if length == 0 {
                            on_body(&[]);
                        } else {
                            self.state = DecodeState::Body(length);
                        }
                    } else if self.buffer.len() == self.max_header_bytes {
                        return Err(FrameError::HeaderTooLarge);
                    }
                }
                DecodeState::Body(length) => {
                    let take = (length - self.buffer.len()).min(input.len());
                    self.buffer.extend_from_slice(&input[..take]);
                    input = &input[take..];
                    if self.buffer.len() == length {
                        on_body(&self.buffer);
                        self.buffer.clear();
                        self.state = DecodeState::Header;
                    }
                }
                DecodeState::Failed => return Err(FrameError::Poisoned),
            }
        }
        Ok(())
    }
}

fn content_length(header: &[u8]) -> Result<usize, FrameError> {
    let header =
        std::str::from_utf8(header).map_err(|_| FrameError::InvalidHeader("non-ASCII bytes"))?;
    let mut length = None;
    for line in header.split("\r\n") {
        let (name, value) = line
            .split_once(':')
            .ok_or(FrameError::InvalidHeader("expected a header field"))?;
        if name.is_empty()
            || !name
                .bytes()
                .all(|byte| byte.is_ascii_alphanumeric() || b"!#$%&'*+-.^_`|~".contains(&byte))
            || !value
                .bytes()
                .all(|byte| byte == b'\t' || (b' '..=b'~').contains(&byte))
        {
            return Err(FrameError::InvalidHeader("invalid field name or value"));
        }
        if name.eq_ignore_ascii_case("Content-Length") {
            if length.is_some() {
                return Err(FrameError::InvalidHeader("duplicate Content-Length"));
            }
            let value = value.trim_matches([' ', '\t']);
            if value.is_empty() || !value.bytes().all(|byte| byte.is_ascii_digit()) {
                return Err(FrameError::InvalidHeader("invalid Content-Length"));
            }
            length = Some(value.parse().map_err(|_| FrameError::BodyTooLarge)?);
        }
    }
    length.ok_or(FrameError::InvalidHeader("missing Content-Length"))
}

struct BoundedBody {
    bytes: Vec<u8>,
    max: usize,
}

impl io::Write for BoundedBody {
    fn write(&mut self, bytes: &[u8]) -> io::Result<usize> {
        if bytes.len() > self.max - self.bytes.len() {
            return Err(io::Error::other("LSP frame body exceeds its byte limit"));
        }
        self.bytes.extend_from_slice(bytes);
        Ok(bytes.len())
    }

    fn flush(&mut self) -> io::Result<()> {
        Ok(())
    }
}
