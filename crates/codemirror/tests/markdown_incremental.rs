use gpui_editor_codemirror::{
    Change, ChangeSet, EditError, Text,
    language::markdown::{MarkdownDialect, MarkdownDocument},
};

fn check(source: &str, changes: &ChangeSet, dialect: MarkdownDialect) -> MarkdownDocument {
    let old = MarkdownDocument::parse_with_dialect(Text::new(source), dialect);
    let incremental = old.reparse(changes).unwrap();
    let mut text = old.text().clone();
    changes.apply_to(&mut text).unwrap();
    let cold = MarkdownDocument::parse_with_dialect(text, dialect);
    assert_eq!(incremental.text(), cold.text());
    assert_eq!(
        incremental.root(),
        cold.root(),
        "{source:?} with {changes:?}"
    );
    let highlights = |doc: &MarkdownDocument| {
        doc.highlights(0..doc.text().len())
            .unwrap()
            .map(|span| (span.range, span.tags.to_vec()))
            .collect::<Vec<_>>()
    };
    assert_eq!(highlights(&incremental), highlights(&cold));
    assert_eq!(
        incremental.overlays().collect::<Vec<_>>(),
        cold.overlays().collect::<Vec<_>>()
    );
    incremental
}

#[test]
fn reuses_prefix_and_shifted_suffix_on_a_paragraph_edit() {
    let mut source = String::new();
    let mut begin = 0;
    for index in 0..2500 {
        use std::fmt::Write;
        if index == 1250 {
            begin = source.len();
        }
        write!(source, "Paragraph {index}: **café** and `λ`.\r\n\r\n").unwrap();
    }
    let range = begin..begin + "Paragraph".len();
    let changes = ChangeSet::single(Change::new(range, "Changed 😀")).unwrap();
    let doc = check(&source, &changes, MarkdownDialect::CommonMark);
    assert_eq!(doc.reused_blocks(), 2498);
    assert_eq!(doc.parsed_bytes(), 81);
    assert_eq!(doc.text().len(), 96_393);
}

#[test]
fn keeps_prefix_but_rejects_swallowed_suffix_anchor() {
    for insert in ["```markdown\n", "<!--\n", "<?xml\n"] {
        let source = "# Keep\n\nChange here\n\n# Anchor\n\nLast paragraph\n";
        let at = source.find("Change").unwrap();
        let changes = ChangeSet::single(Change::new(at..at, insert)).unwrap();
        let doc = check(source, &changes, MarkdownDialect::CommonMark);
        assert_eq!(doc.reused_blocks(), 1);
        assert!(doc.parsed_bytes() >= doc.text().len() - "# Keep\n".len());
    }
}

#[test]
fn multiple_changes_match_full_parse_in_both_dialects() {
    let source = "# Keep\n\nParagraph *one*\n\n# Middle\n\n- [x] task\n\n| head |\n| - |\n| ~~row~~ |\n\n# Later\nUnchanged\n\nOne more\n\nLast\n";
    let first = source.find("Paragraph").unwrap();
    let second = source.find("task").unwrap();
    let changes = ChangeSet::new(vec![
        Change::new(first..first, "> "),
        Change::new(second..second + 4, "é"),
    ])
    .unwrap();
    for dialect in [MarkdownDialect::CommonMark, MarkdownDialect::Extended] {
        let doc = check(source, &changes, dialect);
        assert!(doc.reused_blocks() >= 3);
        assert!(doc.parsed_bytes() < doc.text().len());
        let empty = doc.reparse(&ChangeSet::default()).unwrap();
        assert_eq!(empty.root(), doc.root());
        assert_eq!(empty.parsed_bytes(), 0);
        assert_eq!(empty.reused_blocks(), doc.root().children().len());
    }
}

#[test]
fn definitions_fall_back_and_invalid_ranges_are_rejected() {
    let source = "# A\n\n[title]: /old\n\n# B\n\n[title]\n\n# C\n\né\n";
    let at = source.find("old").unwrap();
    let changes = ChangeSet::single(Change::new(at..at + 3, "new-path")).unwrap();
    let doc = check(source, &changes, MarkdownDialect::Extended);
    assert_eq!(doc.reused_blocks(), 0);
    assert_eq!(doc.parsed_bytes(), doc.text().len());
    let middle = doc.text().as_str().find('é').unwrap() + 1;
    let invalid = ChangeSet::single(Change::new(middle..middle, "x")).unwrap();
    assert_eq!(doc.reparse(&invalid).unwrap_err(), EditError::InvalidRange);
}

#[test]
fn bounded_reuse_matches_cold_parse_across_boundary_edits() {
    let source = "# Keep\nParagraph.\n\n- list\n  continuation\n\n    code\n\n# Head\n\n| a |\n| - |\n| b |\n\n> quote\n> more\n\nAfter\n\n# Last\n\nTail\n";
    for at in 0..=source.len() {
        for insert in ["x", "\n", "\r\n", "    ", "---\n", "```\n"] {
            let changes = ChangeSet::single(Change::new(at..at, insert)).unwrap();
            check(source, &changes, MarkdownDialect::Extended);
        }
        if at < source.len() {
            let changes =
                ChangeSet::single(Change::new(at..(at + 5).min(source.len()), "")).unwrap();
            check(source, &changes, MarkdownDialect::Extended);
        }
    }
}
