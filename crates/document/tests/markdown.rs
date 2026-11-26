use std::borrow::Cow;

use gpui_editor_document::{EditError, Markdown, SectionKind};

#[test]
fn deleting_formatted_content_expands_the_range_to_include_delimiters() {
    let prefix = "前段  ";
    for (formatted, body) in [
        ("**x**", "x"),
        ("_x_", "x"),
        ("***字***", "字"),
        ("`` 字 ``", "字"),
    ] {
        let source = format!("{prefix}{formatted}  後段\r\n");
        let start = prefix.len() + formatted.find(body).unwrap();
        let (range, replacement) = Markdown::new(&source)
            .plan_replacement(start..start + body.len(), "", None)
            .unwrap();
        assert_eq!(range, prefix.len()..prefix.len() + formatted.len());
        assert!(matches!(replacement, Cow::Borrowed("")));
    }
}

#[test]
fn inline_code_preserves_source_ranges_and_uses_the_parsed_text() {
    let prefix = "前段 ";
    for (span, content, display) in [
        ("`ls -la`", "ls -la", "ls -la"),
        ("``  ls `字`  ``", " ls `字` ", " ls `字` "),
        ("`  `", "  ", "  "),
        ("`` a\nb ``", " a\nb ", "a b"),
        ("`$x`", "$x", "$x"),
    ] {
        let source = format!("{prefix}{span} 後段\r\n");
        let sections = Markdown::new(&source).sections();
        assert_eq!(sections.len(), 1);
        let code = &sections[0];
        assert_eq!(code.kind, SectionKind::InlineCode);
        assert_eq!(code.source, prefix.len()..prefix.len() + span.len());
        assert_eq!(&source[code.content.clone()], content);
        assert_eq!(code.inline_code.as_deref().unwrap_or(content), display);
    }
    assert!(
        Markdown::new(r"Escaped \`x\` and `unclosed")
            .sections()
            .is_empty()
    );
}

#[test]
fn inserting_into_an_empty_mermaid_block_adds_only_a_missing_line_ending() {
    for ending in ["\n", "\r\n"] {
        let opener = format!("```mermaid{ending}");
        let body = format!("flowchart LR{ending}A --> B");
        let expected = format!("{body}{ending}");
        for empty_line in ["", ending] {
            let source = format!("{opener}{empty_line}```{ending}{ending}正文");
            let at = opener.len();
            let (range, replacement) = Markdown::new(&source)
                .plan_replacement(at..at, &body, None)
                .unwrap();
            assert_eq!(range, at..at);
            if empty_line.is_empty() {
                assert!(matches!(replacement, Cow::Owned(text) if text == expected));
            } else {
                assert!(matches!(replacement, Cow::Borrowed(text) if text == body));
            }
        }
    }
}

#[test]
fn planning_rejects_read_only_content_and_invalid_utf8_ranges() {
    let source = "前段\r\n\r\n> 原文\r\n";
    let start = source.find('>').unwrap();
    let end = start + "> 原文".len();
    let markdown = Markdown::new(source);
    assert_eq!(
        markdown.plan_replacement(start..end, "", None),
        Err(EditError::ReadOnly)
    );
    let inside = start + 1;
    assert_eq!(
        markdown.plan_replacement(inside..inside, "字", None),
        Err(EditError::ReadOnly)
    );
    assert_eq!(
        markdown.plan_replacement(1..2, "字", None),
        Err(EditError::InvalidRange)
    );
}

#[test]
fn planned_text_outlives_the_source_borrow_and_preserves_other_bytes() {
    let prefix = "---\r\ntitle: keep  \r\n---\r\n\r\n# ";
    let suffix = "\r\n\r\n`原文`  \r\n";
    let old = "舊標題";
    let text = String::from("新標題👋");
    let mut updated = format!("{prefix}{old}{suffix}");
    let (range, replacement) = {
        let source = updated.clone();
        let markdown = Markdown::new(&source);
        let heading = markdown
            .sections()
            .into_iter()
            .find(|section| section.kind == SectionKind::Heading)
            .unwrap();
        markdown
            .plan_replacement(heading.content, &text, None)
            .unwrap()
    };
    assert_eq!(range, prefix.len()..prefix.len() + old.len());
    assert!(matches!(replacement, Cow::Borrowed(s) if std::ptr::eq(s, text.as_str())));
    updated.replace_range(range, &replacement);
    assert_eq!(
        updated.as_bytes(),
        format!("{prefix}{text}{suffix}").as_bytes()
    );
}
