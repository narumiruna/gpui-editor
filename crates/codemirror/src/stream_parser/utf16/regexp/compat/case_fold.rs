// Inclusive code-point runs with a stride and signed folding delta.
const FOLDS: &[(u32, u32, u32, i32)] = include!("case_fold_ranges.rs");

pub(super) fn fold(code: u32) -> u32 {
    let index = FOLDS.partition_point(|&(_, end, _, _)| end < code);
    if let Some(&(start, _, stride, delta)) = FOLDS.get(index)
        && code >= start
        && (code - start).is_multiple_of(stride)
    {
        return code.wrapping_add_signed(delta);
    }
    code
}

/// Iterate over the canonical code point and every source that folds to it.
pub(super) fn equivalents(code: u32) -> impl Iterator<Item = u32> {
    let canonical = fold(code);
    std::iter::once(canonical).chain(FOLDS.iter().filter_map(
        move |&(start, end, stride, delta)| {
            let source = canonical.wrapping_sub_signed(delta);
            (source >= start && source <= end && (source - start).is_multiple_of(stride))
                .then_some(source)
        },
    ))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::stream_parser::utf16::RegExp;

    #[test]
    fn native_singletons_match_the_pinned_simple_fold_table() {
        for &(start, end, stride, delta) in FOLDS {
            for code in (start..=end).step_by(stride as usize) {
                let expected = code.wrapping_add_signed(delta);
                let original = char::from_u32(code).unwrap();
                let target = char::from_u32(expected).unwrap();
                let mut units = [0; 2];
                let mut positive =
                    RegExp::new(&format!(r"^[\q{{\u{{{code:x}}}}}]$"), "iv").unwrap();
                let mut negative =
                    RegExp::new(&format!(r"^[^\q{{\u{{{code:x}}}}}]$"), "iv").unwrap();
                assert!(positive.test(target.encode_utf16(&mut units)), "{code:x}");
                assert!(
                    !positive.test(original.encode_utf16(&mut units)),
                    "{code:x}"
                );
                assert!(negative.test(original.encode_utf16(&mut units)), "{code:x}");
                assert!(!negative.test(target.encode_utf16(&mut units)), "{code:x}");
            }
        }
    }
}
