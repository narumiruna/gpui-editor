use std::ops::Range;

use gpui_editor_codemirror::{
    Change, ChangeSet, Compartment, EditorState, Extension, Text, Transaction,
    language::{
        Color,
        HighlightLayer::{Active, Fallback},
        HighlightProperties, HighlightRule, HighlightSelector, HighlightSpan, HighlightStyle,
        HighlightTag,
        HighlightTheme::{Dark, Light},
        highlighting_for, highlighting_for_context, highlighting_for_native,
        wast::{
            LANGUAGE_DATA, NodeProp, SUBLANGUAGES, WastDataFacet, WastNodeType, WastSublanguage,
            WastSublanguageMode, parse_raw,
        },
    },
    language_data::{
        LANGUAGES, NativeLanguage, WastLanguage, attached_language_snapshot, wast_language,
    },
    theme_one_dark::{highlight_configuration, highlight_layer},
    view::EditorView,
};
use serde_json::{Value, json};

const INPUT: &str = " (module (func))  ";
const EMPTY: HighlightProperties = HighlightProperties {
    foreground: None,
    bold: false,
    italic: false,
    text_decoration: None,
};
const BOLD: HighlightProperties = HighlightProperties {
    bold: true,
    ..EMPTY
};
const KEYWORD: HighlightStyle<'static> = HighlightStyle::define(&[HighlightRule::new(
    &[HighlightSelector::new("keyword")],
    BOLD,
)]);

fn root(language: &WastLanguage) -> &WastNodeType {
    language.node_types().find(|row| row.is_top()).unwrap()
}

#[test]
fn parsed_wast_keywords_match_pinned_root_scopes() {
    let stock = wast_language();
    let marker = NodeProp::new();
    let on_module = || marker.add(|row| row.is_top().then_some(true));
    let configured = stock.with_node_props(&[on_module()]);
    let configured_renamed = configured.configure(None, Some("configured-renamed"));
    let again = stock.with_node_props(&[on_module()]);
    let app_only = stock.with_node_props(&[marker.add(|row| row.is("App").then_some(true))]);
    let renamed = stock.configure(None, Some("renamed"));
    let strict = stock.configure(Some(true), Some("strict-renamed"));
    assert_eq!(configured_renamed.name(), "configured-renamed");
    assert_eq!(renamed.name(), "renamed");
    assert_eq!(root(&configured_renamed).prop(&marker), Some(&true));
    assert_eq!(root(stock).term(), root(&configured).term());
    assert_eq!(root(stock).name(), root(&configured).name());

    let replacement = WastDataFacet::new(None);
    let with_facet = |facet: Option<WastDataFacet>| {
        stock.with_node_props(&[LANGUAGE_DATA.add(move |row| row.is_top().then(|| facet.clone()))])
    };
    let swapped = with_facet(Some(replacement.clone()));
    let absent = with_facet(None);
    let inner = WastDataFacet::new(None);
    let rule = WastSublanguage::new(WastSublanguageMode::Replace, inner.clone(), |_, _| {
        panic!("highlighting must not test sublanguages")
    });
    let sub = stock
        .with_node_props(&[SUBLANGUAGES.add(move |row| row.is_top().then(|| vec![rule.clone()]))]);
    let app = stock.node_types().find(|row| row.is("App")).unwrap();
    let styles = [
        ("L", KEYWORD.with_wast_facet_scope(stock.data_facet())),
        ("F", KEYWORD.with_wast_facet_scope(&replacement)),
        ("I", KEYWORD.with_wast_facet_scope(&inner)),
        ("S", KEYWORD.with_wast_node_type_scope(root(stock))),
        ("R", KEYWORD.with_wast_node_type_scope(root(&configured))),
        ("D", KEYWORD.with_wast_node_type_scope(root(&again))),
        ("A", KEYWORD.with_wast_node_type_scope(app)),
        ("X", KEYWORD.with_wast_node_type_scope(root(&absent))),
    ];
    let hits = |tags: &[HighlightTag], context| {
        styles
            .iter()
            .filter_map(|&(name, style)| {
                highlighting_for_context(&[Active(style)], tags, context, Light)
                    .bold
                    .then_some(name)
            })
            .collect::<Vec<_>>()
    };
    let keyword = |tags: &[HighlightTag]| tags.iter().any(|tag| tag.name() == "keyword");

    let slot = Compartment::new();
    let original = EditorState::with_extensions(INPUT, slot.of(stock.extension())).unwrap();
    let reconfigure = |state: &EditorState, language: &WastLanguage| {
        let mut next = state.clone();
        next.update(Transaction::default().with_effects([slot.reconfigure(language.extension())]))
            .unwrap();
        next
    };
    let next = reconfigure(&original, &configured);
    let mut edited = next.clone();
    edited
        .update(Transaction::new(
            ChangeSet::single(Change::new(14..14, " $x")).unwrap(),
        ))
        .unwrap();
    let after = reconfigure(&edited, &again);
    let sub = reconfigure(&original, &sub);
    let original_snapshot = attached_language_snapshot(&original).unwrap();

    for line in include_str!("fixtures/wast_highlight_scopes.jsonl").lines() {
        let row: Value = serde_json::from_str(line).unwrap();
        let name = row["name"].as_str().unwrap();
        if name == "fallback" {
            let fallback = Fallback(styles[0].1.with_theme(Dark));
            let misses_scope = Active(styles[4].1);
            let misses_theme = Active(styles[0].1.with_theme(Dark));
            let choices = [
                &[fallback][..],
                &[misses_scope, fallback],
                &[misses_theme, fallback],
            ]
            .map(|layers| {
                highlighting_for_context(
                    layers,
                    &[HighlightTag::new("keyword")],
                    original_snapshot.highlight_context(&(2..8)),
                    Light,
                )
                .bold
                .then_some("fallback")
            });
            assert_eq!(json!(choices), row["choices"]);
            continue;
        }
        let text = row["text"].as_str().unwrap();
        let [start, end]: [usize; 2] = serde_json::from_value(row["range"].clone()).unwrap();
        let record = |range: Range<usize>, tags, context| {
            json!([range.start, range.end, &text[range], hits(tags, context)])
        };
        let actual = if name.starts_with("raw-") {
            let language = match name {
                "raw-stock" => stock,
                "raw-configured" => &configured,
                "raw-configured-renamed" => &configured_renamed,
                "raw-app-only" => &app_only,
                "raw-renamed" => &renamed,
                "raw-strict" => &strict,
                "raw-swapped" => &swapped,
                "raw-absent" => &absent,
                _ => panic!("unknown case {name}"),
            };
            let tree = language.parse(text).unwrap();
            let context = tree.wast_highlight_context();
            assert_eq!(context.unwrap().language(), NativeLanguage::Wast);
            let mut nodes = vec![tree.root()];
            let mut records = Vec::new();
            while let Some(node) = nodes.pop() {
                // These cases only configure markers and data, not style paths.
                if let Some(style) = node
                    .wast_style_tags(&[])
                    .filter(|style| keyword(style.tags))
                {
                    records.push(record(node.range(), style.tags, context));
                }
                nodes.extend(node.children().rev());
            }
            records
        } else {
            let snapshot = match name {
                "attached-original" | "attached-retained" => original_snapshot.clone(),
                "attached-next" => attached_language_snapshot(&next).unwrap(),
                "attached-edited" => attached_language_snapshot(&edited).unwrap(),
                "attached-again" => attached_language_snapshot(&after).unwrap(),
                "attached-renamed" => {
                    attached_language_snapshot(&reconfigure(&next, &configured_renamed)).unwrap()
                }
                "attached-swapped" => {
                    attached_language_snapshot(&reconfigure(&original, &swapped)).unwrap()
                }
                "attached-absent" => {
                    attached_language_snapshot(&reconfigure(&original, &absent)).unwrap()
                }
                "attached-sub" => attached_language_snapshot(&sub).unwrap(),
                "markdown" | "markdown-clipped" => NativeLanguage::Markdown
                    .support()
                    .parse_with_code_languages(Text::new(text), Default::default(), LANGUAGES)
                    .unwrap()
                    .into(),
                _ => panic!("unknown case {name}"),
            };
            let markdown = name.starts_with("markdown");
            assert_eq!(snapshot.text().as_str(), text);
            if markdown {
                let gap = text.find("module").unwrap() + 5..text.find("func").unwrap() + 1;
                let unknown = text.rfind("module").unwrap();
                assert!(snapshot.highlight_context(&(5..10)).is_none()); // prose
                assert!(snapshot.highlight_context(&gap).is_none()); // quote gap
                assert!(snapshot.highlight_context(&(1..5)).is_none()); // UTF-8 interior
                assert!(
                    snapshot
                        .highlight_context(&(unknown..unknown + 6))
                        .is_none()
                );
            }
            snapshot
                .highlights(start..end)
                .unwrap()
                .into_iter()
                .filter(|span| keyword(span.tags))
                .map(|HighlightSpan { range, tags }| {
                    assert_eq!(
                        snapshot.highlight_scope(&range),
                        (!markdown).then_some(NativeLanguage::Wast)
                    );
                    let context = snapshot.highlight_context(&range);
                    record(range, tags, context)
                })
                .collect()
        };
        assert_eq!(json!(actual), row["hits"], "{name}");
    }
}

#[test]
fn owned_wast_scopes_compose_with_native_scope_and_remain_isolated() {
    const ITALIC: HighlightProperties = HighlightProperties {
        italic: true,
        ..EMPTY
    };
    const COLOR: HighlightProperties = HighlightProperties {
        foreground: Some(Color::from_rgba32(0x123456ff)),
        ..EMPTY
    };
    let replacement = |facet| {
        wast_language()
            .with_node_props(&[LANGUAGE_DATA
                .add(move |row| row.is_top().then(|| Some(WastDataFacet::clone(&facet))))])
    };
    let (context, layers) = {
        let facet = WastDataFacet::new(None);
        let language = replacement(facet.clone());
        let tree = language.parse("   ").unwrap();
        let style = HighlightStyle::define(&[]);
        let scoped = style.with_all(BOLD).with_wast_facet_scope(&facet);
        let copied = scoped;
        assert_eq!(scoped, copied);
        (
            tree.wast_highlight_context(),
            [
                Active(copied),
                Active(
                    style
                        .with_all(ITALIC)
                        .with_wast_node_type_scope(tree.root().wast_node_type().unwrap()),
                ),
                Active(
                    style
                        .with_all(COLOR)
                        .with_native_scope(NativeLanguage::Wast),
                ),
            ],
        )
    };
    // The style and context outlive the facet, row, and tree. The styles are
    // still Copy and can be installed as static layers without leaking values.
    let state =
        EditorState::with_extensions("", Extension::group(layers.map(highlight_layer))).unwrap();
    let (installed, theme) = highlight_configuration(&state);
    let resolve = |context| highlighting_for_context(&installed, &[], context, theme);
    assert_eq!(
        resolve(context),
        HighlightProperties {
            bold: true,
            italic: true,
            ..COLOR
        }
    );
    let other = replacement(WastDataFacet::new(None)).parse("   ").unwrap();
    assert_eq!(resolve(other.wast_highlight_context()), COLOR);
    assert_eq!(
        resolve(parse_raw(INPUT, false).unwrap().wast_highlight_context()),
        COLOR
    );
    assert_eq!(resolve(None), EMPTY);
    assert_eq!(
        highlighting_for_native(&layers, &[], Some(NativeLanguage::Wast), Light),
        COLOR
    );
    assert_eq!(
        highlighting_for(&layers, &[], Some(&HighlightTag::new("wast")), Light),
        EMPTY
    );

    let json = NativeLanguage::Json
        .support()
        .parse(Text::new("{}"))
        .unwrap();
    let json_context = json.highlight_context(&(0..2));
    assert_eq!(json_context.unwrap().language(), NativeLanguage::Json);
    assert_eq!(resolve(json_context), EMPTY);
    let empty = NativeLanguage::Wast.support().parse(Text::new("")).unwrap();
    assert_eq!(empty.highlight_context(&(0..0)), None);
}

#[test]
fn viewport_context_uses_only_the_published_prefix_and_retains_old_rows() {
    let marker = NodeProp::new();
    let stock = wast_language();
    let configured = stock.with_node_props(&[marker.add(|row| row.is_top().then_some(true))]);
    let source = format!("(module)\n;; 😀{}\n(module)", "a".repeat(4_000));
    let slot = Compartment::new();
    let state = EditorState::with_extensions(
        source.as_str(),
        slot.of(stock.viewport_extension().unwrap()),
    )
    .unwrap();
    let keyword_range = 1..7;
    let eof = source.len();
    assert_eq!(state.wast_viewport_highlight_context(&keyword_range), None);
    let mut view = EditorView::<()>::new(state);
    let old = view.state().clone();
    let published = old.wast_syntax_tree(0).unwrap();
    let old_context = old.wast_viewport_highlight_context(&keyword_range);
    assert_eq!(old_context, published.tree.wast_highlight_context());
    assert!(published.tree.root().range().end > published.parsed_to);
    assert_eq!(
        old.wast_viewport_highlight_context(&(published.parsed_to..published.parsed_to + 1)),
        None
    );
    assert_eq!(old.wast_viewport_highlight_context(&(13..17)), None); // UTF-8 interior
    assert_eq!(old.wast_viewport_highlight_context(&(0..0)), None);
    assert!(
        view.wast_viewport_highlights(keyword_range.clone())
            .unwrap()
            .any(|span| {
                span.range == keyword_range
                    && highlighting_for_context(
                        &[Active(KEYWORD.with_wast_node_type_scope(root(stock)))],
                        span.tags,
                        old_context,
                        Light,
                    )
                    .bold
            })
    );
    let last_range = eof - 7..eof - 1;
    assert!(
        view.ensure_wast_syntax_tree(eof, 1_200)
            .unwrap()
            .unwrap()
            .complete
    );
    assert_eq!(
        view.state().wast_viewport_highlight_context(&last_range),
        None
    );
    assert!(view.force_wast_parsing(eof, 0).unwrap());
    assert_eq!(
        view.state().wast_viewport_highlight_context(&last_range),
        old_context
    );
    view.update(
        Transaction::default()
            .with_effects([slot.reconfigure(configured.viewport_extension().unwrap())]),
        (),
        |post| {
            assert_eq!(post.wast_viewport_highlight_context(&keyword_range), None);
            (None, ())
        },
    )
    .unwrap();
    let new_context = view.state().wast_viewport_highlight_context(&keyword_range);
    assert_ne!(new_context, old_context);
    assert_eq!(
        new_context,
        configured.parse(INPUT).unwrap().wast_highlight_context()
    );
    assert_eq!(
        old.wast_viewport_highlight_context(&keyword_range),
        old_context
    );
}
