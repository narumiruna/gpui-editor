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
