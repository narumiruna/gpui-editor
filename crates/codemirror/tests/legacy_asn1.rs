//! The fixture runs the SHA-512-verified legacy-modes 6.5.5 factory and
//! streamparser 6.0.0 in external scratch. Positions use UTF-8 slice lengths.

use std::num::NonZeroUsize;

use gpui_editor_codemirror::{
    Text,
    commands::CommentTokens,
    language_data::{LANGUAGES, LanguageDescription, LoadInvocation, NativeLanguage, NativeSyntax},
    legacy_modes::{
        LegacyMode,
        asn1::{Asn1Config, asn1, language_data},
    },
    stream_parser::{StreamParser, Tokenizer},
};
use serde_json::{Value, json};

#[test]
fn asn1_factory_tokenization_and_data_match_the_pinned_release() {
    let words = |items: &[&str]| Some(items.iter().map(|word| (*word).to_owned()).collect());
    for line in include_str!("fixtures/legacy_asn1.jsonl").lines() {
        let row: Value = serde_json::from_str(line).unwrap();
        let mode = asn1(if row["id"] == "custom" {
            Asn1Config {
                keywords: words(&["MARK"]),
                tags: words(&["TYPE9"]),
                status: words(&["MARK", "Value"]),
                access_types: words(&["INCLUDE"]),
                ..Default::default()
            }
        } else {
            Asn1Config::default()
        });
        let source = row["doc"].as_str().unwrap();
        let tokens: Vec<_> = Tokenizer::new(&mode, source, NonZeroUsize::new(4).unwrap(), 2)
            .map(|token| {
                let token = token.unwrap();
                json!([token.range.start, token.range.end, token.style])
            })
            .collect();
        assert_eq!(json!(tokens), row["out"], "{}", row["id"]);
        let metadata = language_data();
        assert_eq!(json!(mode.name()), row["meta"]["name"]);
        assert_eq!(
            json!(metadata.get::<CommentTokens>("commentTokens").unwrap().line),
            row["meta"]["comment"]
        );
        let indent_on_input = metadata.get::<fn(&str) -> bool>("indentOnInput").unwrap();
        assert_eq!(
            json!(["  {", "}", "{}", "{ ", "{\n", "\u{feff}}"].map(indent_on_input)),
            row["meta"]["input"]
        );
        assert_eq!(mode.indent(&mode.start_state(2), "}", 2), None);
    }
}

#[test]
fn catalog_calls_the_factory_instead_of_treating_it_as_a_stream_export() {
    const MODULE: &str = "@codemirror/legacy-modes/mode/asn1";
    assert!(LegacyMode::from_export(MODULE, "asn1").is_none());
    let entry = LanguageDescription::match_filename(LANGUAGES, "file.asn1").unwrap();
    assert_eq!(entry.source.invocation, LoadInvocation::EmptyOptions);
    let builtin = entry.built_in().unwrap();
    assert_eq!(builtin, NativeLanguage::Asn1);
    let source = "Example DEFINITIONS ::= BEGIN\r\nEND\r\n";
    let parsed = builtin.parse(Text::new(source)).unwrap();
    assert!(matches!(parsed, NativeSyntax::Asn1(_)));
    assert!(parsed.highlights(0..source.len()).unwrap().any(
        |span| parsed.text().slice(span.range) == Some("DEFINITIONS")
            && span.tags[0].name() == "keyword"
    ));
}
