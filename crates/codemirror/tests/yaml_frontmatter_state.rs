use std::sync::Arc;

use gpui_editor_codemirror::{
    Assoc, Change, ChangePermission, ChangeSet, Compartment, EditError, EditorSelection,
    EditorState, Extension, Facet, StateField, Transaction, change_filter,
    commands::CommentTokens,
    language::yaml::frontmatter_language::YamlFrontmatterLanguage,
    language_data::{CloseBracketData, NativeLanguage, NativeSyntax},
};
use serde_json::{Value, json};

fn metadata(state: &EditorState, at: usize, side: Assoc) -> Value {
    let comments: Vec<_> = state
        .language_data_at::<CommentTokens>("commentTokens", at, side)
        .iter()
        .map(|tokens| (tokens.line, tokens.block))
        .collect();
    let brackets: Vec<_> = state
        .language_data_at::<CloseBracketData>("closeBrackets", at, side)
        .iter()
        .map(|data| data.brackets)
        .collect();
    json!([
        comments,
        brackets,
        state.language_data_at::<&str>("wordChars", at, side),
        state
            .language_data_at::<fn(&str) -> bool>("indentOnInput", at, side)
            .len(),
    ])
}

fn offset(value: &Value) -> usize {
    usize::try_from(value.as_u64().unwrap()).unwrap()
}

#[test]
fn state_matches_pinned_wrapper_across_edits_and_reconfiguration() {
    let label = Facet::collect();
    let edit_count = StateField::new(
        |_| Ok(0),
        |old, update| Ok(old + u64::from(!update.transaction().changes().is_empty())),
    );
    let html = YamlFrontmatterLanguage::new(NativeLanguage::Html).state_field(Extension::group([
        label.of("forwarded"),
        edit_count.extension(),
    ]));
    let json = YamlFrontmatterLanguage::new(NativeLanguage::Json).state_field(Extension::default());
    let extension = |name: &str| match name {
        "html" => html.extension(),
        "json" => json.extension(),
        "off" => Extension::default(),
        _ => panic!("unknown wrapper config: {name}"),
    };
    let compartment = Compartment::new();
    let mut state = EditorState::new("");
    let mut config = "off".to_owned();
    for line in include_str!("fixtures/yaml_frontmatter_state.jsonl").lines() {
        let step: Value = serde_json::from_str(line).unwrap();
        let source = step["source"].as_str().unwrap();
        if let Some(name) = step["start"].as_str() {
            config = name.to_owned();
            state = EditorState::with_extensions(source, compartment.of(extension(name))).unwrap();
        } else {
            let mut tr = Transaction::default();
            if let Some(edit) = step["edit"].as_array() {
                tr = Transaction::new(
                    ChangeSet::single(Change::new(
                        offset(&edit[0])..offset(&edit[1]),
                        edit[2].as_str().unwrap(),
                    ))
                    .unwrap(),
                );
            }
            if let Some(step_selection) = step["selection"].as_u64() {
                let at = usize::try_from(step_selection).unwrap();
                tr = tr.with_selection(EditorSelection::single(at, at));
            }
            if let Some(name) = step["config"].as_str() {
                config = name.to_owned();
                tr = tr.with_effects([compartment.reconfigure(extension(name))]);
            }
            state.update(tr).unwrap();
        }
        assert_eq!(state.text().as_str(), source);
        let context = state.context();
        let html = context.field(&html).unwrap();
        let json = context.field(&json).unwrap();
        assert_eq!(html.is_some(), config == "html");
        assert_eq!(json.is_some(), config == "json");
        if let Some(snapshot) = html.or(json) {
            let document = snapshot.as_ref().as_ref().unwrap();
            assert_eq!(document.text(), state.text());
            assert_eq!(
                matches!(document.body(), NativeSyntax::Html(_)),
                config == "html"
            );
            assert_eq!(
                matches!(document.body(), NativeSyntax::Json(_)),
                config == "json"
            );
        }
        assert_eq!(
            json!(context.facet(&label).unwrap().as_slice()),
            step["label"]
        );
        assert_eq!(
            json!(context.field(&edit_count).unwrap().as_deref()),
            step["editCount"]
        );
        for group in step["data"].as_array().unwrap() {
            for byte in group[2].as_array().unwrap() {
                let at = offset(byte);
                for (side, expected) in [(Assoc::Before, &group[0]), (Assoc::After, &group[1])] {
                    assert_eq!(
                        metadata(&state, at, side),
                        *expected,
                        "{source:?} at {at} {side:?}"
                    );
                }
            }
        }
    }
}

#[test]
fn tracks_accepted_edits_without_changing_native_body_boundaries() {
    let source = "---\r\ntitle: 😀\r\n---\r\n<script>/* probe */</script>";
    let closing = source[5..].find("---").unwrap() + 5;
    let marker = closing..closing + 3;
    let filter = change_filter(move |_, _| ChangePermission::Protect(vec![marker.clone()]));
    let field =
        YamlFrontmatterLanguage::new(NativeLanguage::Html).state_field(Extension::default());
    let mut state =
        EditorState::with_extensions(source, Extension::group([field.extension(), filter]))
            .unwrap();
    let snapshot = |state: &EditorState| state.context().field(&field).unwrap().unwrap();
    let previous = snapshot(&state);
    let old = state.clone();
    state
        .update(Transaction::default().with_selection(EditorSelection::single(5, 5)))
        .unwrap();
    assert!(Arc::ptr_eq(&previous, &snapshot(&state)));

    let partial = source.find('😀').unwrap() + 1;
    assert_eq!(
        metadata(&state, partial, Assoc::After),
        json!([[], [], [], 0])
    );
    assert_eq!(
        metadata(&state, source.len() + 1, Assoc::Before),
        json!([[], [], [], 0])
    );
    let invalid = ChangeSet::single(Change::new(partial..partial, "?")).unwrap();
    assert_eq!(
        state.update(Transaction::new(invalid)),
        Err(EditError::InvalidRange)
    );
    assert!(Arc::ptr_eq(&previous, &snapshot(&state)));

    let changes = ChangeSet::new(vec![
        Change::new(5..5, "# 文\r\n"),
        Change::new(closing..closing + 3, "xxx"),
    ])
    .unwrap();
    state.update(Transaction::new(changes)).unwrap();
    assert_eq!(
        state.text().as_str(),
        source.replacen("title:", "# 文\r\ntitle:", 1)
    );
    assert_eq!(
        snapshot(&old).as_ref().as_ref().unwrap().text().as_str(),
        source
    );
    assert!(!Arc::ptr_eq(&previous, &snapshot(&state)));
    let state_source = state.text().as_str();
    let script_start = state_source.find("<script>").unwrap() + "<script>".len();
    let script_end = state_source.find("</script>").unwrap();
    let words = |at, side| state.language_data_at::<&str>("wordChars", at, side);
    assert_eq!(words(script_start, Assoc::Before), ["-_"]);
    assert_eq!(words(script_start, Assoc::After), ["$"]);
    assert_eq!(words(script_end, Assoc::Before), ["$"]);
    assert_eq!(words(script_end, Assoc::After), ["-_"]);
}

#[test]
fn tracked_edits_share_unchanged_body_and_reuse_both_mounted_prefixes() {
    let language = YamlFrontmatterLanguage::new(NativeLanguage::Json);
    let field = language.state_field(Extension::default());
    let source = format!(
        "---\r\nentries:\r\n{}last: old\r\n---\r\n[\r\n{}  \"end😀\"\r\n]\r\n",
        "  - item: 😀\r\n".repeat(100),
        "  {\"value\": true},\r\n".repeat(100),
    );
    let mut state = EditorState::with_extensions(&source, field.extension()).unwrap();
    let snapshot = |state: &EditorState| state.context().field(&field).unwrap().unwrap();
    let original = snapshot(&state);
    let yaml = source.find("old").unwrap();
    let json = source.find("end😀").unwrap();
    let front_edit = ChangeSet::single(Change::new(yaml..yaml + 3, "名")).unwrap();
    state.update(Transaction::new(front_edit)).unwrap();
    let unchanged_body = snapshot(&state);
    let before_body_edit = unchanged_body.as_ref().as_ref().unwrap();
    let original = original.as_ref().as_ref().unwrap();
    assert!(std::ptr::eq(original.body(), before_body_edit.body()));
    assert!(
        before_body_edit
            .frontmatter()
            .unwrap()
            .yaml()
            .reuse_stats()
            .nodes_reused
            > 100
    );

    // Both replacement strings above are three bytes, so the old body offset
    // is still correct. This accepted edit also shifts the body after it.
    let changes = ChangeSet::new(vec![
        Change::new(yaml..yaml + 3, "新的值"),
        Change::new(json..json + 3, "끝"),
    ])
    .unwrap();
    state.update(Transaction::new(changes)).unwrap();
    let updated = snapshot(&state);
    let updated = updated.as_ref().as_ref().unwrap();
    assert_eq!(original.text().as_str(), source);
    assert_eq!(updated.text(), state.text());
    assert_eq!(updated.body_range().start, original.body_range().start + 6);
    assert!(
        updated
            .frontmatter()
            .unwrap()
            .yaml()
            .reuse_stats()
            .nodes_reused
            > 100
    );
    assert!(updated.body_reuse_stats().nodes_reused > 100);

    let fresh = language.parse(state.text().clone()).unwrap();
    let spans = |doc: &gpui_editor_codemirror::language::yaml::frontmatter_language::YamlFrontmatterDocument| {
        doc.highlights(0..doc.text().len())
            .unwrap()
            .map(|span| (span.range, span.tags.to_vec()))
            .collect::<Vec<_>>()
    };
    assert_eq!(spans(updated), spans(&fresh));
    assert_eq!(updated.body().text(), fresh.body().text());
}

#[test]
fn absent_snapshot_supplies_no_language_data_and_retries_after_edit() {
    let field =
        YamlFrontmatterLanguage::new(NativeLanguage::Json).state_field(Extension::default());
    // Simulate the absent snapshot left by a tokenization failure.
    let mut state =
        EditorState::with_extensions("---\nx: [\n---\n[]", field.init(|_| Ok(None))).unwrap();
    let at = 4;
    let before = state.context().field(&field).unwrap().unwrap();
    state
        .update(Transaction::default().with_selection(EditorSelection::single(at, at)))
        .unwrap();
    assert!(Arc::ptr_eq(
        &before,
        &state.context().field(&field).unwrap().unwrap()
    ));
    assert_eq!(metadata(&state, at, Assoc::After), json!([[], [], [], 0]));

    let changes = ChangeSet::single(Change::new(at..at, "# invalid YAML follows\n")).unwrap();
    state.update(Transaction::new(changes)).unwrap();
    let snapshot = state.context().field(&field).unwrap().unwrap();
    let snapshot = snapshot.as_ref().as_ref().unwrap();
    assert!(snapshot.frontmatter().unwrap().yaml().is_recovered());
    assert_eq!(
        metadata(&state, at, Assoc::After),
        json!([[["#", null]], [], [], 1])
    );
}
