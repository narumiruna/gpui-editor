use gpui_editor_codemirror::utf16::{code_point_at, code_point_size, from_code_point};
use serde_json::Value;

#[test]
fn utf16_code_point_helpers_match_pinned_state() {
    let oracle: Value =
        serde_json::from_str(include_str!("fixtures/state_code_points.json")).unwrap();
    let encode: Vec<(u32, usize, Vec<u16>)> =
        serde_json::from_value(oracle["encode"].clone()).unwrap();
    let decode: Vec<(Vec<u16>, Vec<Option<u32>>)> =
        serde_json::from_value(oracle["decode"].clone()).unwrap();
    for (code, size, units) in encode {
        assert_eq!(size, code_point_size(code));
        assert_eq!(units, from_code_point(code).unwrap().collect::<Vec<_>>());
    }
    for (units, expected) in decode {
        let actual: Vec<_> = (0..=units.len())
            .map(|index| code_point_at(&units, index))
            .collect();
        assert_eq!(expected, actual, "{units:x?}");
    }
    assert!(from_code_point(0x110000).is_none());
    assert_eq!(code_point_at(&[0xd800, 0xdc00], usize::MAX), None);
}
