//! Code point operations on UTF-16 input, separate from native byte positions.

/// Read at a UTF-16 code unit index. A valid pair returns one supplementary
/// code point. An unpaired surrogate, or a position on a low surrogate,
/// returns that unit's value. An index outside the input returns `None`.
pub fn code_point_at(text: &[u16], index: usize) -> Option<u32> {
    let [first, tail @ ..] = text.get(index..)? else {
        return None;
    };
    match (*first, tail.first().copied()) {
        (lead @ 0xd800..=0xdbff, Some(trail @ 0xdc00..=0xdfff)) => {
            Some(0x10000 + ((u32::from(lead) - 0xd800) << 10) + u32::from(trail) - 0xdc00)
        }
        (unit, _) => Some(u32::from(unit)),
    }
}

/// Return the number of UTF-16 units for a Unicode code point.
pub const fn code_point_size(code: u32) -> usize {
    if code < 0x10000 { 1 } else { 2 }
}

/// Encode one Unicode code point without allocating. This accepts surrogate
/// values and rejects values greater than `0x10ffff`.
pub fn from_code_point(code: u32) -> Option<impl ExactSizeIterator<Item = u16>> {
    let units = match code {
        0..=0xffff => [code as u16, 0],
        0x10000..=0x10ffff => {
            let offset = code - 0x10000;
            [
                0xd800 + (offset >> 10) as u16,
                0xdc00 + (offset & 0x3ff) as u16,
            ]
        }
        _ => return None,
    };
    Some(units.into_iter().take(code_point_size(code)))
}
