use std::sync::Arc;

use gpui_editor_codemirror::{
    Compartment, EditorState, Extension, Precedence, Text, Transaction,
    autocomplete::session::SourceResult,
    language::{
        css::{
            CssDocument, CssPropertySnapshot, css_completion_source_with_properties,
            define_css_completion_source, define_css_completion_source_with_properties,
        },
        sass::SassDialect,
    },
    language_data::{NativeLanguage, NativeSyntax},
};
use serde_json::{Value, json};

// Chromium 159.0.8154.6, Playwright build 1255, with the repository npm pins.
const STOCK: (usize, u32) = (741, 283_478_746);

fn signature(result: &SourceResult) -> (usize, u32) {
    let mut hash = 2_166_136_261u32;
    for item in result.options.iter() {
        for byte in item
            .label
            .bytes()
            .chain([0])
            .chain(item.kind.as_deref().unwrap().bytes())
            .chain([0])
            .chain(item.insert.as_deref().unwrap_or_default().bytes())
            .chain([b'\n'])
        {
            hash = (hash ^ u32::from(byte)).wrapping_mul(16_777_619);
        }
    }
    (result.options.len(), hash)
}

fn summary(result: Option<&SourceResult>) -> Value {
    json!(result.map(|result| {
        let (len, hash) = signature(result);
        (result.from, len, hash)
    }))
}

fn names(result: &SourceResult) -> Vec<&str> {
    result
        .options
        .iter()
        .map(|item| item.label.as_str())
        .collect()
}

#[test]
fn installed_snapshots_match_pinned_sources_and_leave_stock_controls_unchanged() {
    for line in include_str!("fixtures/css_property_snapshot.jsonl").lines() {
        let fixture: Value = serde_json::from_str(line).unwrap();
        let keys = fixture["keys"]
            .as_array()
            .unwrap()
            .iter()
            .map(|key| key.as_str().unwrap());
        let state = EditorState::create(
            "",
            None,
            CssPropertySnapshot::from_style_keys(keys).extension(),
        )
        .unwrap();
        let configured = CssPropertySnapshot::source_for(&state).unwrap();
        let mut shared = None;
        for row in fixture["rows"].as_array().unwrap() {
            let language = match row["language"].as_str().unwrap() {
                "css" | "css-helper" => NativeLanguage::Css,
                "less" => NativeLanguage::Less,
                "scss" => NativeLanguage::Sass(SassDialect::Scss),
                "sass" => NativeLanguage::Sass(SassDialect::Indented),
                "html" => NativeLanguage::Html,
                other => panic!("unknown language: {other}"),
            };
            let marked = row["marked"].as_str().unwrap();
            let explicit = row["explicit"].as_bool().unwrap();
            let case = format!(
                "{} {language:?} {marked:?} explicit={explicit}",
                fixture["mode"]
            );
            let at = marked.find('§').unwrap();
            let doc = language
                .support()
                .parse(Text::new(marked.replace('§', "")))
                .unwrap();
            let (result, stock) = if row["language"] == "css-helper" {
                let NativeSyntax::Css(doc) = doc.syntax() else {
                    unreachable!()
                };
                (
                    define_css_completion_source_with_properties(|_| false, &configured)(
                        doc, at, explicit,
                    ),
                    define_css_completion_source(|_| false)(doc, at, explicit),
                )
            } else {
                let mut indexed =
                    doc.indexed_completions_with_css_properties(at, explicit, Some(&configured));
                let result = indexed.next().map(|(slot, result)| {
                    assert_eq!(slot, 0, "{case}");
                    result
                });
                assert!(indexed.next().is_none(), "{case}");
                (result, doc.completions(at, explicit).into_iter().next())
            };
            assert_eq!(summary(result.as_ref()), row["expected"], "{case}");
            if row["property"] == true {
                let result = result.unwrap();
                assert_eq!(json!(names(&result)), fixture["labels"], "{case}");
                assert_eq!(result.to, None, "{case}");
                assert!(
                    result.filter && result.commit_characters.is_empty(),
                    "{case}"
                );
                for item in result.options.iter() {
                    assert_eq!(item.kind.as_deref(), Some("property"), "{case}");
                    assert_eq!(item.insert, Some(format!("{}: ", item.label)), "{case}");
                }
                let prior = shared.get_or_insert_with(|| Arc::clone(&result.options));
                assert!(Arc::ptr_eq(prior, &result.options), "{case}");
                let stock = stock.unwrap();
                assert_eq!(stock.from, result.from, "{case}");
                assert_eq!(signature(&stock), STOCK, "{case}");
            } else {
                assert_eq!(summary(stock.as_ref()), row["expected"], "{case}");
            }
        }
    }
}

#[test]
fn snapshot_keeps_empty_keys_and_excludes_only_exact_matches() {
    let snapshot = CssPropertySnapshot::from_style_keys([
        "", "cssText", "cssFloat", "css-text", "cssTextX", "CssFloat", "zIndex", "z-index",
    ]);
    let doc = CssDocument::parse(Text::new("a { color: red }"));
    let result = css_completion_source_with_properties(&doc, 7, false, &snapshot).unwrap();
    assert_eq!(
        names(&result),
        ["", "-css-float", "css-text", "css-text-x", "z-index"]
    );
}

#[test]
fn state_precedence_replacement_and_removal_do_not_change_existing_snapshots() {
    let compartment = Compartment::new();
    let mut keys = [String::from("hostOnly")];
    let captured = CssPropertySnapshot::from_style_keys(&keys).extension();
    keys[0].replace_range(.., "newProperty");
    let mut state = EditorState::create(
        "",
        None,
        Extension::group([
            CssPropertySnapshot::from_style_keys(["fallbackOnly"]).extension(),
            compartment.of(captured.with_precedence(Precedence::High)),
        ]),
    )
    .unwrap();
    let first = CssPropertySnapshot::source_for(&state).unwrap();
    let doc = NativeLanguage::Css
        .support()
        .parse(Text::new("a { color: red }"))
        .unwrap();
    let complete = |snapshot: Option<&CssPropertySnapshot>| {
        doc.indexed_completions_with_css_properties(7, false, snapshot)
            .next()
            .unwrap()
            .1
    };
    assert_eq!(names(&complete(Some(&first))), ["host-only"]);
    state.update(Transaction::default()).unwrap();
    assert!(Arc::ptr_eq(
        &first,
        &CssPropertySnapshot::source_for(&state).unwrap()
    ));

    let empty = CssPropertySnapshot::from_style_keys([] as [&str; 0]).extension();
    state
        .update(
            Transaction::default()
                .with_effects([compartment.reconfigure(empty.with_precedence(Precedence::High))]),
        )
        .unwrap();
    let second = CssPropertySnapshot::source_for(&state).unwrap();
    assert!(!Arc::ptr_eq(&first, &second));
    assert!(complete(Some(&second)).options.is_empty());
    assert_eq!(names(&complete(Some(&first))), ["host-only"]);

    state
        .update(
            Transaction::default().with_effects([compartment.reconfigure(Extension::default())]),
        )
        .unwrap();
    assert_eq!(
        names(&complete(
            CssPropertySnapshot::source_for(&state).as_deref()
        )),
        ["fallback-only"]
    );
    state
        .update(Transaction::default().with_effects([Extension::default().reconfigure()]))
        .unwrap();
    let removed = CssPropertySnapshot::source_for(&state);
    assert!(removed.is_none());
    assert_eq!(signature(&complete(removed.as_deref())), STOCK);
}
