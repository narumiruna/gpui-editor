use gpui_editor::document::{Document, EditError, SectionKind};

const ORIGINAL: &str = include_str!("fixtures/sample.md");

#[test]
fn updates_only_the_heading_and_formula() {
    let mut doc = Document::new(ORIGINAL.into());
    let heading = doc
        .sections()
        .into_iter()
        .find(|section| section.kind == SectionKind::Heading)
        .unwrap();
    assert_eq!(&ORIGINAL[heading.content.clone()], "原始標題");
    doc.replace(heading.content, "新標題").unwrap();

    let formula = doc
        .sections()
        .into_iter()
        .find(|section| section.kind == SectionKind::InlineMath)
        .unwrap();
    assert_eq!(&doc.source()[formula.content.clone()], "x^2");
    doc.replace(formula.content, "y^3").unwrap();

    assert_eq!(
        doc.source(),
        ORIGINAL
            .replacen("# 原始標題", "# 新標題", 1)
            .replacen("$x^2$", "$y^3$", 1)
    );
}

#[test]
fn source_positions_support_multiline_utf8_edits() {
    let mut doc = Document::new("前段\n\n次段\n\n後段\n".into());
    assert_eq!(doc.replace(1..2, "無效"), Err(EditError::InvalidRange));
    let start = "前".len();
    let end = "前段\n\n次".len();
    assert_eq!(&doc.source()[start..end], "段\n\n次");
    doc.replace(start..end, "中\n\n新").unwrap();
    assert_eq!(doc.source(), "前中\n\n新段\n\n後段\n");
}

#[test]
fn rejects_edits_that_touch_an_unsupported_table() {
    let mut doc = Document::new(ORIGINAL.into());
    let table = doc
        .sections()
        .into_iter()
        .find(|section| section.kind == SectionKind::ReadOnly)
        .unwrap();
    assert_eq!(
        doc.replace(table.source.clone(), ""),
        Err(EditError::ReadOnly)
    );
    let inside = table.source.start + "| ".len();
    assert_eq!(doc.replace(inside..inside, "欄"), Err(EditError::ReadOnly));
    assert_eq!(doc.source(), ORIGINAL);
}

#[test]
fn malformed_latex_has_a_source_range_after_reopening() {
    let source = "`$x{$` should stay code.\n\nFormula: $\\frac{$.\n\n$$\n\\sqrt{\n$$\n";
    let doc = Document::new(source.into());
    let sections = doc.sections();
    let inline = sections
        .iter()
        .find(|section| section.kind == SectionKind::InlineMath)
        .unwrap();
    let display = sections
        .iter()
        .find(|section| section.kind == SectionKind::DisplayMath)
        .unwrap();
    assert_eq!(&source[inline.content.clone()], r"\frac{");
    assert_eq!(source[display.content.clone()].trim(), r"\sqrt{");
}

#[test]
fn editing_a_fenced_body_preserves_its_closing_line_and_crlf() {
    let source = "---\r\ntitle: Old\r\n---\r\n\r\n```mermaid\r\nflowchart LR\r\nA --> B\r\n```\r\n\r\n正文\r\n";
    let mut doc = Document::new(source.into());
    for (kind, body) in [
        (SectionKind::FrontMatter, "title: New"),
        (SectionKind::Mermaid, "flowchart LR\r\nC --> D"),
    ] {
        let section = doc
            .sections()
            .into_iter()
            .find(|section| section.kind == kind)
            .unwrap();
        doc.replace(section.content, body).unwrap();
    }
    assert_eq!(
        doc.source(),
        source
            .replace("title: Old", "title: New")
            .replace("A --> B", "C --> D")
    );
}
