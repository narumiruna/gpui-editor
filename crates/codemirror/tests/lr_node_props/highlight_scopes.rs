use std::ops::Range;

use gpui_editor_codemirror::{
    language::{
        HighlightContext, HighlightLayer::Active, HighlightProperties, HighlightRule,
        HighlightSelector, HighlightStyle, HighlightTag, HighlightTheme::Light,
        highlighting_for_context, json,
    },
    language_data::wast_language,
};

use super::*;

const STRING: HighlightStyle<'static> = HighlightStyle::define(&[HighlightRule::new(
    &[HighlightSelector::new("string")],
    HighlightProperties {
        foreground: None,
        bold: true,
        italic: false,
        text_decoration: None,
    },
)]);

fn root(parser: &MarkdownCodeParser) -> &NodeType {
    with_lr!(parser, handle => handle.node_types().find(|row| row.is_top()).unwrap())
}

#[test]
fn parsed_lr_strings_match_exact_row_and_facet_scopes() {
    let rows: Vec<Value> = include_str!("../fixtures/lr_highlight_scopes.jsonl")
        .lines()
        .map(|line| serde_json::from_str(line).unwrap())
        .collect();
    assert_eq!(rows.len(), 26);
    for (grammar, leaf, raw_string) in [("json", "Array", "String"), ("lezer", "Body", "Literal")] {
        let grammar_rows: Vec<_> = rows.iter().filter(|row| row["name"] == grammar).collect();
        let [stock, other] = handles(grammar_rows[0]);
        let marker = NodeProp::new();
        let root_prop = || marker.add(|row| row.is_top().then_some(true));
        let changed = with_props(&stock, &[root_prop()]);
        let independent = with_props(&stock, &[root_prop()]);
        let renamed = match &changed {
            MarkdownCodeParser::Json(handle) => {
                MarkdownCodeParser::Json(handle.configure(Some(true), Some("renamed")))
            }
            MarkdownCodeParser::Lezer(handle) => {
                MarkdownCodeParser::Lezer(handle.configure(Some(true), Some("renamed")))
            }
            _ => unreachable!(),
        };
        let leaf_only = with_props(
            &stock,
            &[marker.add(move |row| row.is(leaf).then_some(true))],
        );
        let stock_facet = with_lr!(&stock, handle => handle.data_facet());
        let other_facet = with_lr!(&other, handle => handle.data_facet());
        let wast_facet = wast_language().data_facet();
        let with_facet = |facet: Option<&'static LanguageFacet>| {
            with_props(
                &stock,
                &[LANGUAGE_DATA.add(move |row| row.is_top().then(|| facet.cloned()))],
            )
        };
        let replace = with_facet(Some(other_facet));
        let absent = with_facet(None);
        let wast_facet_only = with_facet(Some(wast_facet));
        let styles = [
            ("S", STRING.with_node_type_scope(root(&stock))),
            ("R", STRING.with_node_type_scope(root(&changed))),
            ("D", STRING.with_node_type_scope(root(&independent))),
            ("L", STRING.with_data_facet_scope(stock_facet)),
            ("C", STRING.with_data_facet_scope(other_facet)),
            ("W", STRING.with_data_facet_scope(wast_facet)),
            ("B", STRING.with_node_type_scope(with_lr!(&stock, handle => handle.node_types().find(|row| row.is(leaf)).unwrap()))),
            ("A", STRING.with_node_type_scope(root(&absent))),
        ];
        let tags = [HighlightTag::new("string")];
        let record = |range: Range<usize>, context: Option<HighlightContext>| {
            assert_eq!(context.unwrap().language(), stock.language());
            let hits: Vec<_> = styles
                .iter()
                .filter_map(|&(label, style)| {
                    highlighting_for_context(&[Active(style)], &tags, context, Light)
                        .bold
                        .then_some(label)
                })
                .collect();
            assert!(
                !highlighting_for_context(
                    &[Active(STRING.with_wast_facet_scope(wast_facet))],
                    &tags,
                    context,
                    Light
                )
                .bold
            );
            assert!(
                !highlighting_for_context(
                    &[Active(STRING.with_wast_node_type_scope(root(&changed)))],
                    &tags,
                    context,
                    Light
                )
                .bold
            );
            json!([range.start, range.end, hits])
        };
        let source = grammar_rows[0]["source"].as_str().unwrap();
        let slot = Compartment::new();
        let original = EditorState::with_extensions(
            source,
            slot.of(with_lr!(&stock, handle => handle.extension())),
        )
        .unwrap();
        let swap =
            |state: &EditorState, parser: &MarkdownCodeParser| {
                let mut next = state.clone();
                next.update(Transaction::default().with_effects([
                    slot.reconfigure(with_lr!(parser, handle => handle.extension())),
                ]))
                .unwrap();
                next
            };
        let current = swap(&original, &renamed);
        let mut edited = current.clone();
        let at = source.find('字').unwrap();
        let change = ChangeSet::single(Change::new(at..at + '字'.len_utf8(), "new")).unwrap();
        edited.update(Transaction::new(change)).unwrap();
        let separate = swap(&current, &independent);
        for row in grammar_rows {
            let step = row["step"].as_str().unwrap();
            let source = row["source"].as_str().unwrap();
            let [from, to]: [usize; 2] = serde_json::from_value(row["range"].clone()).unwrap();
            let spans = if step.starts_with("raw-") {
                let parser = match step {
                    "raw-stock" => &stock,
                    "raw-changed" => &changed,
                    "raw-renamed" => &renamed,
                    "raw-leafOnly" => &leaf_only,
                    "raw-replace" => &replace,
                    "raw-absent" => &absent,
                    "raw-wastFacet" => &wast_facet_only,
                    _ => unreachable!(),
                };
                let tree = with_lr!(parser, handle => handle.parse(source).unwrap());
                let scope = tree.highlight_context();
                let mut pending = vec![tree.root()];
                let mut spans = Vec::new();
                while let Some(node) = pending.pop() {
                    if node.name() == raw_string {
                        spans.push(record(node.range(), scope));
                    }
                    pending.extend(node.children().rev());
                }
                spans
            } else {
                let doc = if step.starts_with("mount") {
                    let parser = changed.clone();
                    let mounted = EditorState::with_extensions(
                        source,
                        markdown_with_parser(
                            MarkdownConfig::default(),
                            MarkdownParserConfig {
                                code_languages: Some(MarkdownCodeLanguages::Configured {
                                    resolve: Arc::new(move |label| {
                                        (label == "key").then(|| parser.clone())
                                    }),
                                    fallback: None,
                                }),
                                ..Default::default()
                            },
                        ),
                    )
                    .unwrap();
                    let doc = markdown_snapshot(&mounted).unwrap();
                    let quotes: Vec<_> = source
                        .match_indices("\r\n> ")
                        .map(|(at, _)| at + 2)
                        .collect();
                    let gap = quotes[2];
                    assert_eq!(doc.highlight_context(&(gap..gap + 2)), None);
                    assert_eq!(doc.highlight_context(&(gap - 2..gap + 3)), None);
                    assert_eq!(doc.highlight_scope(&(from..to)), None);
                    doc
                } else {
                    let state = match step {
                        "editor-new" => &current,
                        "editor-retained" => &original,
                        "editor-edit" => &edited,
                        "editor-independent" => &separate,
                        _ => unreachable!(),
                    };
                    attached_language_snapshot(state).unwrap()
                };
                assert_eq!(doc.text().as_str(), source);
                let inside = source.find('😀').unwrap() + 1;
                assert_eq!(doc.highlight_context(&(inside..inside + 1)), None);
                assert_eq!(doc.highlight_context(&(0..0)), None);
                doc.highlights(from..to)
                    .unwrap()
                    .into_iter()
                    .filter(|span| span.tags.iter().any(|tag| tag.name() == "string"))
                    .map(|span| {
                        let context = doc.highlight_context(&span.range);
                        record(span.range, context)
                    })
                    .collect()
            };
            assert_eq!(json!(spans), row["spans"], "{grammar}: {step}");
        }
    }
    assert_eq!(
        json::parse_raw("0", true).unwrap().highlight_context(),
        None
    );
}
