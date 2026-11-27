use gpui_editor_codemirror::{
    Assoc, Change, ChangeSet, EditError, Text,
    language::{StreamSettings, javascript::JavascriptOptions, yaml::YamlNode},
    language_data::{
        LANGUAGES, LanguageDescription, NativeLanguage, NativeSyntax, SyntaxUpdateError,
    },
};

fn replace(at: std::ops::Range<usize>, insert: &str) -> ChangeSet {
    ChangeSet::single(Change::new(at, insert)).unwrap()
}

#[test]
fn built_in_support_reuses_validated_syntax_and_keeps_bad_edits_atomic() {
    let input = format!(
        "[\r\n{}  \"😀\",\r\n  1\r\n]\r\n",
        "  {\"name\": [true, false]},\r\n".repeat(100)
    );
    let support = NativeLanguage::Json.support();
    let mut document = support.parse(Text::new(&input)).unwrap();
    let old = document.text().clone();
    let at = input.rfind('1').unwrap();
    let change = replace(at..at + 1, "12");
    document.apply_changes(&change).unwrap();
    assert!(document.reuse_stats().nodes_reused > 200);
    let mut expected = old;
    change.apply_to(&mut expected).unwrap();
    let fresh = support.parse(expected.clone()).unwrap();
    let highlights = |doc: &gpui_editor_codemirror::language_data::SupportedDocument| {
        doc.highlights(0..doc.text().len())
            .unwrap()
            .into_iter()
            .map(|span| (span.range, span.tags.to_vec()))
            .collect::<Vec<_>>()
    };
    assert_eq!(document.text(), &expected);
    assert_eq!(highlights(&document), highlights(&fresh));

    let inside = input.find('😀').unwrap() + 1;
    assert_eq!(
        document.apply_changes(&replace(inside..inside + 1, "x")),
        Err(SyntaxUpdateError::Edit(EditError::InvalidRange))
    );
    assert_eq!(document.text(), &expected);
    assert!(document.reuse_stats().nodes_reused > 200);
    assert_eq!(highlights(&document), highlights(&fresh));
}

#[test]
fn yaml_support_keeps_strict_checkpoints_across_edits() {
    let input = format!("{}- last: old\r\n", "- key: 😀\r\n".repeat(128));
    let at = input.rfind("old").unwrap();
    let support = NativeLanguage::Yaml.support();
    let mut document = support.parse(Text::new(&input)).unwrap();
    document.apply_changes(&replace(at..at + 3, "新")).unwrap();
    assert!(document.reuse_stats().nodes_reused > 0);
    let rebuilt = support.parse(document.text().clone()).unwrap();
    let (NativeSyntax::Yaml(document), NativeSyntax::Yaml(rebuilt)) =
        (document.syntax(), rebuilt.syntax())
    else {
        panic!("expected YAML documents")
    };
    fn compare_nodes(actual: &YamlNode, expected: &YamlNode) {
        assert_eq!(actual.kind(), expected.kind());
        assert_eq!(actual.range(), expected.range());
        assert_eq!(actual.children().len(), expected.children().len());
        for (child, expected) in actual.children().iter().zip(expected.children()) {
            compare_nodes(child, expected);
        }
    }
    compare_nodes(document.root(), rebuilt.root());
}

#[test]
fn markdown_support_reuses_root_blocks_when_the_boundaries_survive() {
    let input = "Intro **kept**.\r\n\r\nOld 👋.\r\n\r\n# Same heading\r\n\r\nEnd.\r\n";
    let at = input.find("Old").unwrap();
    let support = NativeLanguage::Markdown.support();
    let mut document = support.parse(Text::new(input)).unwrap();
    document.apply_changes(&replace(at..at + 3, "New")).unwrap();
    let rebuilt = support.parse(document.text().clone()).unwrap();
    let (NativeSyntax::Markdown(document), NativeSyntax::Markdown(rebuilt)) =
        (document.syntax(), rebuilt.syntax())
    else {
        panic!("expected Markdown documents")
    };
    assert!(document.reused_blocks() > 0);
    assert_eq!(document.root(), rebuilt.root());
}

#[test]
fn edited_markdown_rebuilds_mounts_with_the_original_catalog() {
    let input = "Intro 😀.\r\n\r\n```js\r\nexport const value = 1\r\n```\r\n";
    let support = NativeLanguage::Markdown.support();
    let mut document = support
        .parse_with_code_languages(Text::new(input), StreamSettings::default(), LANGUAGES)
        .unwrap();
    let at = input.find("export").unwrap();
    assert_eq!(
        document.language_at(at, Assoc::After),
        Some(NativeLanguage::Javascript(JavascriptOptions::default()))
    );
    let fence = input.find("js").unwrap();
    let changes = ChangeSet::new(vec![
        Change::new(fence..fence + 2, "css"),
        Change::new(
            at..at + "export const value = 1".len(),
            ".x { color: red; }",
        ),
    ])
    .unwrap();
    document.apply_changes(&changes).unwrap();
    let text = document.text().as_str();
    assert_eq!(
        text,
        "Intro 😀.\r\n\r\n```css\r\n.x { color: red; }\r\n```\r\n"
    );
    let at = text.find("color").unwrap();
    assert_eq!(
        document.language_at(at, Assoc::After),
        Some(NativeLanguage::Css)
    );
    assert_eq!(document.parent_selection(at + 1..at + 1), Some(at..at + 5));
}

#[test]
fn legacy_support_reparses_suffix_with_saved_line_state() {
    let language = LanguageDescription::match_name(LANGUAGES, "Shell", false)
        .unwrap()
        .built_in()
        .unwrap();
    let input = "# comment\r\necho \"one\r\ntwo\"\r\necho done\r\n";
    let mut document = language.support().parse(Text::new(input)).unwrap();
    let at = input.find("two").unwrap();
    document.apply_changes(&replace(at..at + 3, "字")).unwrap();
    let current = document.text();
    let fresh = language.support().parse(current.clone()).unwrap();
    let updated = document.highlights(0..current.len()).unwrap();
    let rebuilt = fresh.highlights(0..current.len()).unwrap();
    assert_eq!(updated.len(), rebuilt.len());
    assert!(
        updated
            .iter()
            .zip(rebuilt)
            .all(|(a, b)| a.range == b.range && a.tags == b.tags)
    );
}
