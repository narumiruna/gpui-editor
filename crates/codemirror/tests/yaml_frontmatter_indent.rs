use gpui_editor_codemirror::{
    Change, ChangeSet, EditorState, Extension, Transaction,
    language::{
        INDENT_UNIT, IndentDecision, IndentOptions, IndentStyle, get_indentation, indent_service,
        yaml::frontmatter_language::YamlFrontmatterLanguage,
    },
    language_data::NativeLanguage,
};
use serde_json::{Value, json};

#[test]
fn indentation_services_run_before_the_wrapper_syntax() {
    let field =
        YamlFrontmatterLanguage::new(NativeLanguage::Json).state_field(Extension::default());
    let state = EditorState::with_extensions(
        "---\nfoo: [a,\n b]\n---\n[]",
        Extension::group([
            field.extension(),
            indent_service(|_, _| IndentDecision::Column(13)),
        ]),
    )
    .unwrap();
    assert_eq!(
        get_indentation(&state, 11, IndentOptions::default(), None),
        Ok(Some(13))
    );
}

#[test]
fn wrapper_dispatches_indentation_to_the_yaml_and_body_parsers() {
    for row in include_str!("fixtures/yaml_frontmatter_indent.jsonl").lines() {
        let row: Value = serde_json::from_str(row).unwrap();
        let source = row["source"].as_str().unwrap();
        let language = match row["name"].as_str().unwrap() {
            "json" => NativeLanguage::Json,
            "lezer" => NativeLanguage::LezerGrammar,
            _ => unreachable!(),
        };
        let field = YamlFrontmatterLanguage::new(language).state_field(Extension::default());
        let state = EditorState::with_extensions(
            source,
            Extension::group([
                field.extension(),
                INDENT_UNIT.of(IndentStyle::parse("   ").unwrap()),
            ]),
        )
        .unwrap();
        let probe = |state: &EditorState| {
            let source = state.text().as_str();
            for value in row["queries"].as_array().unwrap() {
                let at = value[0].as_u64().unwrap() as usize;
                let line = source[..at].rfind(['\r', '\n']).map_or(0, |at| at + 1);
                let override_indent = |pos| (pos == line).then_some(7);
                assert_eq!(
                    json!(get_indentation(state, at, IndentOptions::default(), None).unwrap()),
                    value[1],
                    "{source:?} at {at}"
                );
                let options = IndentOptions {
                    simulate_break: Some(at),
                    simulate_double_break: true,
                    override_indentation: Some(&override_indent),
                };
                assert_eq!(
                    json!(get_indentation(state, at, options, None).unwrap()),
                    value[2],
                    "{source:?} at {at} with a simulated break"
                );
            }
        };
        probe(&state);
        if let Some(at) = source.find('😀') {
            let changes = ChangeSet::single(Change::new(at..at + '😀'.len_utf8(), "🌱")).unwrap();
            let mut edited = state.clone();
            edited.update(Transaction::new(changes)).unwrap();
            assert_eq!(edited.text().as_str(), source.replace('😀', "🌱"));
            probe(&edited);
        }
    }
}
