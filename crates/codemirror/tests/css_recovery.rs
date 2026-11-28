use gpui_editor_codemirror::{
    Change, ChangeSet, Text,
    autocomplete::session::SourceResult,
    language::{
        IndentOptions, StreamSettings, TagModifier,
        css::{CssDocument, CssNodeKind, css_completion_source},
    },
};
use serde_json::{Value, json};

// Captured with @codemirror/lang-css 6.3.1, @lezer/css 1.3.9, @lezer/lr 1.4.13,
// and @codemirror/autocomplete 6.20.4. The empty-attribute rows use Node with
// the same pinned packages and the 741 captured Chromium 159.0.8154.6 CSS
// property keys in document.body.style. This reproduces the prior 102 rows
// byte for byte. Styles cases use cssLanguage.configure({top: "Styles"}) and
// css().support.
// The state uses LF as its separator so CRLF stays in the input. The cursor
// marker is not part of the document. All positions are UTF-8 byte offsets.
#[test]
fn recovery_matches_pinned_tree_and_editor_services() {
    let cases: Vec<Value> =
        serde_json::from_str(include_str!("fixtures/css_recovery.json")).unwrap();
    for case in cases {
        let input = case["input"].as_str().unwrap();
        let (before, after) = input.split_once('§').unwrap();
        let source = [before, after].concat();
        let doc = match case["mode"].as_str().unwrap() {
            "styles" => CssDocument::parse_styles(Text::new(&source)),
            "css" => CssDocument::parse(Text::new(&source)),
            mode => panic!("unknown CSS parse mode: {mode}"),
        };
        assert_eq!(doc.text().as_str(), source);
        let nodes: Vec<_> = doc
            .nodes()
            .map(|node| {
                let depth = std::iter::successors(node.parent(), |node| node.parent()).count();
                let range = node.range();
                json!([node.kind().name(), range.start, range.end, depth])
            })
            .collect();
        assert_eq!(json!(nodes), case["nodes"], "tree for {input:?}");

        let highlights: Vec<_> = doc
            .highlights(0..source.len())
            .unwrap()
            .map(|span| {
                let [tag] = span.tags else {
                    panic!("expected one CSS tag")
                };
                let name = match tag.modifiers() {
                    [] => tag.name(),
                    [TagModifier::Constant] if tag.name() == "className" => "className.constant",
                    modifiers => panic!("unexpected CSS modifiers: {modifiers:?}"),
                };
                json!([span.range.start, span.range.end, name])
            })
            .collect();
        assert_eq!(json!(highlights), case["highlights"], "highlight {input:?}");

        let mut start = 0;
        let lines: Vec<_> = source
            .split('\n')
            .map(|line| {
                let end = start + line.len();
                let indent =
                    doc.indentation(start, StreamSettings::default(), IndentOptions::default());
                let fold = doc
                    .foldable(start, end)
                    .map(|range| [range.start, range.end]);
                start = end + 1;
                json!([indent, fold])
            })
            .collect();
        assert_eq!(json!(lines), case["lines"], "line services for {input:?}");

        let completions = [false, true].map(|explicit| {
            css_completion_source(&doc, before.len(), explicit).map(completion_signature)
        });
        assert_eq!(
            json!(completions),
            case["completions"],
            "completion {input:?}"
        );
    }
}

fn completion_signature(result: SourceResult) -> Value {
    assert_eq!(result.to, None);
    assert!(result.filter);
    // FNV-1a over the full ordered sets of labels, types, and apply text.
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
    let kind = result
        .options
        .first()
        .map_or("empty", |item| item.kind.as_deref().unwrap());
    json!([result.from, kind, result.options.len(), hash])
}

#[test]
fn reparse_recovers_a_missing_semicolon_after_important() {
    use CssNodeKind::{Declaration, Error, Semicolon};

    let source = "color: red !important;\r\nwidth: 4px;";
    let doc = CssDocument::parse_styles(Text::new(source));
    let semicolon = source.find(';').unwrap();
    let remove = ChangeSet::single(Change::new(semicolon..semicolon + 1, "")).unwrap();
    let recovered = doc.reparse(&remove).unwrap();
    let width = recovered.text().as_str().find("width").unwrap();
    let end = recovered.text().len();
    let children = |doc: &CssDocument| {
        doc.root()
            .children()
            .map(|node| (node.kind(), node.range()))
            .collect::<Vec<_>>()
    };
    assert_eq!(
        children(&recovered),
        [
            (Declaration, 0..semicolon),
            (Error, width..width),
            (Declaration, width..end - 1),
            (Semicolon, end - 1..end),
        ],
    );
    let completion = css_completion_source(&recovered, width + 2, false).unwrap();
    assert_eq!(
        completion_signature(completion),
        json!([width, "property", 741, 283478746])
    );

    let insert = ChangeSet::single(Change::new(semicolon..semicolon, ";")).unwrap();
    let restored = recovered.reparse(&insert).unwrap();
    assert_eq!(restored.text().as_str(), source);
    assert_eq!(children(&restored), children(&doc));
}

#[test]
fn reparse_skips_one_block_item_and_keeps_the_next_declaration() {
    use CssNodeKind::{Block, Declaration, Error};

    let source = "/*猫😀*/\r\na { color:red;\r\n  \r\n  width:2px;\r\n}";
    let doc = CssDocument::parse(Text::new(source));
    let insert = ChangeSet::single(Change::new(31..31, "?")).unwrap();
    let recovered = doc.reparse(&insert).unwrap();
    assert_eq!(recovered.text().slice(31..32), Some("?"));
    let items = |doc: &CssDocument, kind| {
        doc.nodes()
            .filter(|node| node.kind() == kind && node.parent().unwrap().kind() == Block)
            .map(|node| node.range())
            .collect::<Vec<_>>()
    };
    assert_eq!(items(&recovered, Error), std::slice::from_ref(&(31..32)));
    assert_eq!(items(&recovered, Declaration), [17..26, 36..45]);
    assert_eq!(recovered.foldable(13, 27), Some(16..48));
    let completion = css_completion_source(&recovered, 38, false).unwrap();
    assert_eq!(
        completion_signature(completion),
        json!([36, "property", 741, 283478746])
    );

    let remove = ChangeSet::single(Change::new(31..32, "")).unwrap();
    let restored = recovered.reparse(&remove).unwrap();
    assert_eq!(restored.text().as_str(), source);
    assert!(items(&restored, Error).is_empty());
    assert_eq!(items(&restored, Declaration), items(&doc, Declaration));
}

#[test]
fn reparsing_an_empty_attribute_name_keeps_the_rule_body_and_next_rule() {
    use CssNodeKind::{AttributeName, AttributeSelector, Block, CloseBracket, Declaration, Error};

    let source = "/*猫😀*/\r\n.猫😀[\r\n /*前😀*/\r\n ] {\r\n  color: red;\r\n  width: 2px;\r\n}\r\nb { height: 3px; }";
    let empty = CssDocument::parse(Text::new(source));
    let at = source.find(']').unwrap();
    let name = "名😀";
    let insert = ChangeSet::single(Change::new(at..at, name)).unwrap();
    let named = empty.reparse(&insert).unwrap();
    let remove = ChangeSet::single(Change::new(at..at + name.len(), "")).unwrap();
    let restored = named.reparse(&remove).unwrap();
    assert_eq!(
        named.text().as_str(),
        format!("{}{name}{}", &source[..at], &source[at..])
    );
    assert_eq!(restored.text().as_str(), source);

    for (doc, name_len) in [(&empty, 0), (&named, name.len()), (&restored, 0)] {
        let text = doc.text().as_str();
        let closer = at + name_len;
        let attribute = doc
            .nodes()
            .find(|node| node.kind() == AttributeSelector)
            .unwrap();
        let mut children = attribute.children().rev();
        let bracket = children.next().unwrap();
        assert_eq!(
            (bracket.kind(), bracket.range()),
            (CloseBracket, closer..closer + 1)
        );
        let name = children.next().unwrap();
        assert_eq!((name.kind(), name.range()), (AttributeName, at..closer));
        let errors: Vec<_> = doc.nodes().filter(|node| node.kind() == Error).collect();
        if name_len == 0 {
            let [error] = errors.as_slice() else {
                panic!("expected only the missing-name error")
            };
            assert_eq!(error.range(), at..at);
            assert_eq!(error.parent().unwrap().kind(), AttributeName);
            assert_eq!(name.children().count(), 1);
            assert_eq!(error.children().count(), 0);
        } else {
            assert!(errors.is_empty());
            assert_eq!(doc.text().slice(name.range()), Some("名😀"));
            assert!(
                doc.highlights(name.range())
                    .unwrap()
                    .any(|span| span.tags[0].name() == "attributeName")
            );
        }
        let declarations: Vec<_> = doc
            .nodes()
            .filter(|node| node.kind() == Declaration)
            .map(|node| {
                assert_eq!(node.parent().unwrap().kind(), Block);
                &text[node.range()]
            })
            .collect();
        assert_eq!(declarations, ["color: red", "width: 2px", "height: 3px"]);
        assert_eq!(
            doc.foldable(at - 1, closer + 4),
            Some(closer + 3..text.find('}').unwrap())
        );
        let width = text.find("width").unwrap();
        assert_eq!(
            doc.indentation(
                width - 2,
                StreamSettings::default(),
                IndentOptions::default()
            ),
            Some(3)
        );
        assert_eq!(
            completion_signature(css_completion_source(doc, width + 2, false).unwrap()),
            json!([width, "property", 741, 283478746])
        );
    }
}

#[test]
fn empty_attribute_recovery_keeps_skipped_block_items_local() {
    use CssNodeKind::{AttributeName, Block, Declaration, Error};

    let source = "a[] { ? color:red; width:2px; }";
    let doc = CssDocument::parse(Text::new(source));
    let errors: Vec<_> = doc
        .nodes()
        .filter(|node| node.kind() == Error)
        .map(|node| (node.parent().unwrap().kind(), node.range()))
        .collect();
    assert_eq!(errors, [(AttributeName, 2..2), (Block, 6..7)]);
    let declarations: Vec<_> = doc
        .nodes()
        .filter(|node| node.kind() == Declaration)
        .map(|node| &source[node.range()])
        .collect();
    assert_eq!(declarations, ["color:red", "width:2px"]);
}

#[test]
fn unrelated_header_errors_do_not_get_empty_attribute_recovery() {
    use CssNodeKind::AttributeName;

    for source in [
        "a[][] {}",
        "a[], b[] {}",
        "a:is([]) {}",
        "@supports selector(a[]) {}",
        "@keyframes x { a[] {} }",
        "a[svg|] {}",
        "a[=] {}",
        "a[title=] {}",
        "a[][title=] {}",
        "a[title=x z][] {}",
        "? a[] {}",
        "a[] ) {}",
        "a[(][] {}",
        "a[]",
        "a[];",
        "a[ {}",
    ] {
        let doc = CssDocument::parse(Text::new(source));
        assert!(
            !doc.nodes()
                .any(|node| node.kind() == AttributeName && node.range().is_empty()),
            "{source}"
        );
    }
    let inline = CssDocument::parse_styles(Text::new("a[] { color:red; } width:2px;"));
    assert!(!inline.nodes().any(|node| node.kind() == AttributeName));
}

#[test]
fn empty_attribute_recovery_is_bounded_by_header_bytes() {
    use CssNodeKind::{AttributeName, Declaration};

    for (bytes, recovered) in [(4095, true), (4096, true), (4097, false), (65_536, false)] {
        let padding = "x".repeat(bytes - "a[/*猫😀*/]".len());
        let source = format!("a[/*猫😀{padding}*/] {{ width:2px; }}");
        let doc = CssDocument::parse(Text::new(&source));
        assert_eq!(
            doc.nodes()
                .any(|node| node.kind() == AttributeName && node.range() == (bytes - 1..bytes - 1)),
            recovered,
            "{bytes} header bytes"
        );
        assert!(
            doc.nodes()
                .any(|node| node.kind() == Declaration && &source[node.range()] == "width:2px")
        );
    }
}

#[test]
fn a_missing_semicolon_does_not_change_unfinished_group_boundaries() {
    use CssNodeKind::{ArgList, Declaration, ValueName};

    let source = "color: red !important width: 4px; color: var(--tone height: 2px;";
    let doc = CssDocument::parse_styles(Text::new(source));
    let declarations: Vec<_> = doc
        .root()
        .children()
        .filter(|node| node.kind() == Declaration)
        .map(|node| &source[node.range()])
        .collect();
    assert_eq!(
        declarations,
        [
            "color: red !important width: 4px",
            "color: var(--tone height: 2px"
        ],
    );
    let height = source.find("height").unwrap();
    assert!(doc.nodes().any(|node| {
        node.kind() == ValueName
            && node.range().start == height
            && node.parent().unwrap().kind() == ArgList
    }));
}

#[test]
fn reparse_inserts_and_removes_a_closer_before_a_value() {
    use CssNodeKind::{CloseParen, Error, PropertyName};

    let source = "color: /*😀*/ red; width:1px;";
    let doc = CssDocument::parse_styles(Text::new(source));
    let at = source.find("red").unwrap();
    let insert = ChangeSet::single(Change::new(at..at, ") ")).unwrap();
    let recovered = doc.reparse(&insert).unwrap();
    assert!(recovered.nodes().any(|node| {
        node.kind() == CloseParen
            && node.range() == (at..at + 1)
            && node.parent().unwrap().kind() == Error
    }));
    let completion = css_completion_source(&recovered, at + 4, false).unwrap();
    assert_eq!(
        completion_signature(completion),
        json!([at + 2, "keyword", 561, 3770060376u32])
    );
    let remove = ChangeSet::single(Change::new(at..at + 2, "")).unwrap();
    let restored = recovered.reparse(&remove).unwrap();
    assert_eq!(restored.text().as_str(), source);
    assert!(
        restored
            .nodes()
            .any(|node| { node.kind() == PropertyName && &source[node.range()] == "width" })
    );
}

#[test]
fn reparse_replaces_a_value_with_a_closer_and_restores_it() {
    use CssNodeKind::{CloseParen, Declaration, Error};

    let source = "color: /*😀*/ red;\r\nwidth: 2px;";
    let doc = CssDocument::parse_styles(Text::new(source));
    let at = source.find("red").unwrap();
    let replace = ChangeSet::single(Change::new(at..at + 3, ")")).unwrap();
    let recovered = doc.reparse(&replace).unwrap();
    let text = recovered.text().as_str();
    assert_eq!(text, "color: /*😀*/ );\r\nwidth: 2px;");
    assert!(recovered.nodes().any(|node| {
        node.kind() == CloseParen
            && node.range() == (at..at + 1)
            && node.parent().unwrap().kind() == Error
    }));
    let width = text.find("width").unwrap();
    let result = css_completion_source(&recovered, width + 2, false).unwrap();
    assert_eq!(
        completion_signature(result),
        json!([width, "type", 78, 1288706251])
    );

    let restore = ChangeSet::single(Change::new(at..at + 1, "red")).unwrap();
    let restored = recovered.reparse(&restore).unwrap();
    assert_eq!(restored.text().as_str(), source);
    assert!(!restored.nodes().any(|node| node.kind() == Error));
    let width = source.find("width").unwrap();
    assert!(
        restored.root().children().any(|node| {
            node.kind() == Declaration && node.range() == (width..source.len() - 1)
        })
    );
    let result = css_completion_source(&restored, width + 2, false).unwrap();
    assert_eq!(
        completion_signature(result),
        json!([width, "property", 741, 283478746])
    );
}

#[test]
fn reparse_inserts_and_removes_a_later_stray_closer() {
    use CssNodeKind::{CloseBracket, Error};

    let source = "color: red;\r\n/*😀*/ width: 4px;";
    let doc = CssDocument::parse_styles(Text::new(source));
    let at = source.find("width").unwrap();
    let insert = ChangeSet::single(Change::new(at..at, "] ")).unwrap();
    let recovered = doc.reparse(&insert).unwrap();
    assert!(recovered.nodes().any(|node| {
        node.kind() == CloseBracket
            && node.range() == (at..at + 1)
            && node.parent().unwrap().kind() == Error
    }));
    let completion = css_completion_source(&recovered, at + 4, false).unwrap();
    assert_eq!(completion.from, at + 2);
    assert!(completion.options.iter().any(|item| item.label == "width"));

    let remove = ChangeSet::single(Change::new(at..at + 2, "")).unwrap();
    let restored = recovered.reparse(&remove).unwrap();
    assert_eq!(restored.text().as_str(), source);
    assert!(!restored.nodes().any(|node| node.kind() == Error));
}

#[test]
fn a_later_stray_closer_does_not_replace_other_local_recovery() {
    use CssNodeKind::{Declaration, Error};

    for margin in ["margin: 1px ] 2px", "margin: ]"] {
        for damaged in ["color: var( /*😀*/ ", "--tone /*😀*/ "] {
            let source = format!("{margin}; {damaged}; width: 4px;");
            let doc = CssDocument::parse_styles(Text::new(&source));
            let at = source.find(']').unwrap();
            let closer = doc
                .nodes()
                .find(|node| node.kind() == Error && node.range() == (at..at + 1))
                .unwrap();
            assert!(closer.children().next().is_none(), "{source}");
            let declarations: Vec<_> = doc
                .root()
                .children()
                .filter(|node| node.kind() == Declaration)
                .map(|node| &source[node.range()])
                .collect();
            assert_eq!(declarations, [margin, damaged, "width: 4px"]);
        }
    }
}

#[test]
fn a_closer_without_a_value_keeps_a_large_unfinished_group_local() {
    use CssNodeKind::{ArgList, Declaration, Error};

    let source = format!(
        "color: );\r\n{}color: var( /*😀*/ ; width: 4px;",
        "height: 2px;\r\n".repeat(512)
    );
    let doc = CssDocument::parse_styles(Text::new(&source));
    let at = source.find(')').unwrap();
    let closer = doc
        .nodes()
        .find(|node| node.kind() == Error && node.range() == (at..at + 1))
        .unwrap();
    assert!(closer.children().next().is_none());
    assert!(
        doc.nodes()
            .any(|node| { node.kind() == ArgList && &source[node.range()] == "( /*😀*/ " })
    );
    assert!(
        doc.root()
            .children()
            .any(|node| { node.kind() == Declaration && &source[node.range()] == "width: 4px" })
    );
}

#[test]
fn removing_a_call_closer_does_not_take_the_next_style() {
    let source = "--tone: tomato;\r\ncolor: var( /*😀*/ );\r\nwidth: 4px;";
    let doc = CssDocument::parse_styles(Text::new(source));
    let closer = source.find(')').unwrap();
    let remove = ChangeSet::single(Change::new(closer..closer + 1, "")).unwrap();
    let recovered = doc.reparse(&remove).unwrap();
    assert_eq!(
        recovered.text().as_str(),
        "--tone: tomato;\r\ncolor: var( /*😀*/ ;\r\nwidth: 4px;"
    );
    let result = css_completion_source(&recovered, closer, true).unwrap();
    assert_eq!(result.from, closer);
    assert_eq!(result.options.len(), 1);
    assert_eq!(result.options[0].label, "--tone");
    let width = recovered.text().as_str().find("width").unwrap();
    let result = css_completion_source(&recovered, width + 2, false).unwrap();
    assert_eq!(result.from, width);
    assert!(result.options.iter().any(|item| item.label == "width"));

    let restore = ChangeSet::single(Change::new(closer..closer, ")")).unwrap();
    let restored = recovered.reparse(&restore).unwrap();
    assert_eq!(restored.text().as_str(), source);
    assert!(!restored.nodes().any(|node| node.kind().name() == "⚠"));
}
