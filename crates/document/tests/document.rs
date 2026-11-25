use gpui_editor_document::{Alignment, BlockPreview, Document, EditError, Section, SectionKind};

const ORIGINAL: &str = include_str!("fixtures/sample.md");

fn find_section(doc: &Document, kind: SectionKind) -> Section {
    doc.sections()
        .into_iter()
        .find(|section| section.kind == kind)
        .unwrap()
}

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
fn rejects_edits_that_touch_a_table() {
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
    assert!(matches!(table.preview, Some(BlockPreview::Table { .. })));
    assert_eq!(doc.source(), ORIGINAL);
}

#[test]
fn projects_gfm_table_headers_alignment_and_cell_text() {
    let source = concat!(
        "| **功能** | `值` | 連結 |\r\n",
        "| :--- | :---: | ---: |\r\n",
        "| a\\|b | &amp; | [說明](notes.md) |\r\n",
        "| 少一欄 | |\r\n",
        "\r\n後段\r\n",
    );
    let doc = Document::new(source.into());
    let sections = doc.sections();
    assert_eq!(sections.len(), 1);
    let table = &sections[0];
    assert_eq!(table.kind, SectionKind::ReadOnly);
    assert_eq!(
        &source[table.source.clone()],
        &source[..source.find("\r\n後段").unwrap()]
    );
    assert_eq!(
        table.preview,
        Some(BlockPreview::Table {
            alignments: vec![Alignment::Left, Alignment::Center, Alignment::Right],
            rows: vec![
                vec!["功能".into(), "值".into(), "連結".into()],
                vec!["a|b".into(), "&".into(), "說明".into()],
                vec!["少一欄".into(), "".into(), "".into()],
            ],
        })
    );
    assert_eq!(doc.source(), source);
}

#[test]
fn projects_code_as_literal_text_without_fences_or_structural_indent() {
    for (raw, body) in [
        (
            "~~~~rust\r\nfn main() {\r\n\t$literal$\r\n}\r\n~~~~\r\n",
            "fn main() {\n\t$literal$\n}\n",
        ),
        (
            "    let x = 1;\n        next();\n",
            "let x = 1;\n    next();\n",
        ),
        ("```\n```", ""),
        ("```text\nstill code", "still code"),
    ] {
        let mut doc = Document::new(raw.into());
        let sections = doc.sections();
        assert_eq!(sections.len(), 1, "{raw:?}");
        let code = &sections[0];
        assert_eq!(code.kind, SectionKind::ReadOnly);
        assert_eq!(
            code.preview,
            Some(BlockPreview::Code { body: body.into() }),
            "{raw:?}"
        );
        assert_eq!(
            doc.replace(code.source.clone(), ""),
            Err(EditError::ReadOnly)
        );
        assert_eq!(doc.source(), raw);
    }
    let mermaid = Document::new("```mermaid\nflowchart LR\nA --> B\n```\n".into());
    assert!(
        find_section(&mermaid, SectionKind::Mermaid)
            .preview
            .is_none()
    );
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
fn inline_labels_end_before_the_destination() {
    let destination = "https://example.com/a](b)";
    for (kind, prefix, label) in [
        (SectionKind::Link, "[", "Label"),
        (SectionKind::Link, "[", ""),
        (SectionKind::Link, "[", "label\n  "),
        (SectionKind::Image, "![", r"圖 [例] \] \[ `](`"),
        (SectionKind::Link, "[", "![alt](image.png) `$](`"),
    ] {
        let source = format!("前段 {prefix}{label}]({destination} \"title ](\")");
        let doc = Document::new(source.clone());
        let section = doc
            .sections()
            .into_iter()
            .find(|section| section.kind == kind)
            .unwrap();
        assert_eq!(&source[section.content], label);
        assert_eq!(section.destination.as_deref(), Some(destination));
    }
}

#[test]
fn editing_a_fenced_body_preserves_its_closing_line_and_crlf() {
    for closing in ["---", "..."] {
        let source = format!(
            "---\r\ntitle: Old\r\n{closing}\r\n\r\n```mermaid\r\nflowchart LR\r\nA --> B\r\n```\r\n\r\n正文\r\n"
        );
        let mut doc = Document::new(source.clone());
        for (kind, body) in [
            (SectionKind::FrontMatter, "title: New"),
            (SectionKind::Mermaid, "flowchart LR\r\nC --> D"),
        ] {
            let section = find_section(&doc, kind);
            doc.replace(section.content, body).unwrap();
        }
        assert_eq!(
            doc.source(),
            source
                .replace("title: Old", "title: New")
                .replace("A --> B", "C --> D")
        );
    }
}

#[test]
fn mermaid_body_includes_a_final_line_that_is_not_a_closing_fence() {
    for (opening, line) in [
        ("````", "```"),
        ("~~~~", "~~~"),
        ("````", "~~~~"),
        ("````", "````not a fence"),
        ("````", "    ````"),
    ] {
        for ending in ["", "\n", "\r\n"] {
            let body = format!("{line}{ending}");
            let source = format!("{opening}mermaid\n{body}");
            let mut doc = Document::new(source);
            let section = find_section(&doc, SectionKind::Mermaid);
            assert_eq!(&doc.source()[section.content.clone()], body);
            doc.replace(section.content, "flowchart LR").unwrap();
            assert_eq!(doc.source(), format!("{opening}mermaid\nflowchart LR"));
        }
    }
}

#[test]
fn mermaid_closing_fence_can_be_longer_than_the_opener() {
    for (opening, closing) in [("````", "`````  "), ("~~~~", "~~~~~  ")] {
        let source = format!("{opening}mermaid\r\nflowchart LR\r\n{closing}\r\n\r\n正文");
        let mut doc = Document::new(source.clone());
        let section = find_section(&doc, SectionKind::Mermaid);
        assert_eq!(&doc.source()[section.content.clone()], "flowchart LR");
        doc.replace(section.content, "flowchart TB").unwrap();
        assert_eq!(doc.source(), source.replace("flowchart LR", "flowchart TB"));
    }
}

#[test]
fn inserts_into_empty_mermaid_blocks_without_moving_the_closing_fence() {
    for ending in ["\n", "\r\n"] {
        for empty_line in ["", ending] {
            let source = format!("```mermaid{ending}{empty_line}```{ending}{ending}正文");
            let mut doc = Document::new(source);
            let mermaid = |doc: &Document| find_section(doc, SectionKind::Mermaid);
            let section = mermaid(&doc);
            assert!(section.content.is_empty());
            let body = format!("flowchart LR{ending}A --> B");
            doc.replace(section.content, &body).unwrap();
            assert_eq!(
                doc.source(),
                format!("```mermaid{ending}{body}{ending}```{ending}{ending}正文")
            );

            let end = mermaid(&doc).content.end;
            doc.replace(end..end, " --> C").unwrap();
            assert_eq!(
                &doc.source()[mermaid(&doc).content],
                format!("{body} --> C")
            );
        }
    }
}

#[test]
fn removes_delimiters_when_deleting_the_last_formatted_text() {
    for (source, body) in [("**x**", "x"), ("_x_", "x"), ("***字***", "字")] {
        let mut doc = Document::new(source.into());
        let start = source.find(body).unwrap();
        doc.replace(start..start + body.len(), "").unwrap();
        assert_eq!(doc.source(), "");
        doc.replace(0..0, "next").unwrap();
        assert_eq!(doc.source(), "next");
    }

    let mut doc = Document::new("Before **xy** after".into());
    let x = doc.source().find('x').unwrap();
    doc.replace(x..x + 1, "").unwrap();
    assert_eq!(doc.source(), "Before **y** after");
    doc.replace(x..x + 1, "").unwrap();
    assert_eq!(doc.source(), "Before  after");
}
