use std::fmt;

/// An invalid value or length in a Lezer base-46 array string.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum DecodeError {
    InvalidByte { offset: usize },
    TruncatedInteger { offset: usize },
    IntegerOverflow { offset: usize },
    LengthMismatch { declared: u32 },
}

impl fmt::Display for DecodeError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::InvalidByte { offset } => write!(f, "invalid encoded digit at byte {offset}"),
            Self::TruncatedInteger { offset } => write!(f, "incomplete integer at byte {offset}"),
            Self::IntegerOverflow { offset } => write!(f, "integer out of range at byte {offset}"),
            Self::LengthMismatch { declared } => write!(f, "expected {declared} encoded words"),
        }
    }
}

impl std::error::Error for DecodeError {}

pub(super) fn array<T: TryFrom<u32>>(input: &str) -> Result<Vec<T>, DecodeError> {
    let mut bytes = input.bytes().enumerate();
    let declared = integer(&mut bytes, 0)?;
    let len = declared as usize;
    if len > bytes.len() {
        return Err(DecodeError::LengthMismatch { declared });
    }
    let mut words = Vec::with_capacity(len);
    for _ in 0..len {
        let offset = input.len() - bytes.len();
        let value = integer(&mut bytes, offset)?;
        words.push(T::try_from(value).map_err(|_| DecodeError::IntegerOverflow { offset })?);
    }
    if bytes.len() != 0 {
        return Err(DecodeError::LengthMismatch { declared });
    }
    Ok(words)
}

fn integer(
    bytes: &mut impl Iterator<Item = (usize, u8)>,
    start: usize,
) -> Result<u32, DecodeError> {
    let mut value = 0_u32;
    for (offset, byte) in bytes {
        if byte == b'~' && offset == start {
            return Ok(u32::from(u16::MAX));
        }
        if !(b' '..=b'}').contains(&byte) || matches!(byte, b'"' | b'\\') {
            return Err(DecodeError::InvalidByte { offset });
        }
        // The alphabet omits quote and backslash. Its upper half ends a word.
        let digit = byte - b' ' - u8::from(byte > b'"') - u8::from(byte > b'\\');
        value = value
            .checked_mul(46)
            .and_then(|value| value.checked_add(u32::from(digit % 46)))
            .ok_or(DecodeError::IntegerOverflow { offset: start })?;
        if digit >= 46 {
            return Ok(value);
        }
    }
    Err(DecodeError::TruncatedInteger { offset: start })
}
