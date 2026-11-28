use std::{
    future::Future,
    pin::pin,
    sync::{Arc, Mutex},
    task::{Context, Poll, Waker},
};

use gpui_editor_codemirror::{
    EditorState, Text,
    language::{
        IndentOptions, StreamSettings, get_indentation,
        lezer::{INDENT, LrIndentStrategy, LrSyntaxNode},
        lezer_grammar::LezerDocument,
        markdown::{
            MarkdownCodeLanguages, MarkdownCodeParser, MarkdownConfig, MarkdownParserConfig,
            markdown_with_parser,
        },
    },
    language_data::{
        LANGUAGES, LanguageDescription, attached_language_snapshot, cpp_extension, json_language,
        lezer_language, rust_extension, wast_language,
    },
};
use serde_json::{Value, json};

#[test]
fn stock_rust_cpp_and_css_align_with_a_same_line_closer() {
    let css = LanguageDescription::match_name(LANGUAGES, "CSS", false).unwrap();
    let mut load = pin!(css.catalog_support_load().unwrap().load());
    let Poll::Ready(Ok(css)) = load.as_mut().poll(&mut Context::from_waker(Waker::noop())) else {
        panic!("expected native CSS support");
    };
    let css = css.extension().unwrap();
    for line in include_str!("fixtures/stock_delimited_indent.jsonl").lines() {
        let row: Value = serde_json::from_str(line).unwrap();
        let source = row["source"].as_str().unwrap();
        let at = usize::try_from(row["at"].as_u64().unwrap()).unwrap();
        let extension = match row["grammar"].as_str().unwrap() {
            "rust" => rust_extension(),
            "cpp" => cpp_extension(),
            "css" => css.clone(),
            grammar => panic!("unexpected grammar {grammar}"),
        };
        let state = EditorState::with_extensions(source, extension).unwrap();
        let syntax = attached_language_snapshot(&state).unwrap();
        for (index, (simulate_break, simulate_double_break)) in
            [(None, false), (Some(at), false), (Some(at), true)]
                .into_iter()
                .enumerate()
        {
            let options = IndentOptions {
                simulate_break,
                simulate_double_break,
                ..Default::default()
            };
            let value = get_indentation(&state, at, options, None).unwrap();
            assert_eq!(json!(value), row["columns"][index], "{line}: mode {index}");
            assert_eq!(
                syntax
                    .syntax()
                    .indentation(at, StreamSettings::default(), options),
                value,
                "{line}: direct syntax query"
            );
            assert_eq!(
                get_indentation(&state, at, options, Some(syntax.syntax())).unwrap(),
                value,
                "{line}: explicit syntax query"
            );
        }
    }
}

fn span(node: LrSyntaxNode<'_>) -> Value {
    let range = node.range();
    json!([node.name(), range.start, range.end])
}

#[test]
fn default_delimiter_alignment_matches_pinned_indentation() {
    for line in include_str!("fixtures/lr_delimited_indent.jsonl").lines() {
        let row: Value = serde_json::from_str(line).unwrap();
        let name = row["name"].as_str().unwrap();
        let grammar = row["grammar"].as_str().unwrap();
        let mode = row["mode"].as_str().unwrap();
        let source = row["source"].as_str().unwrap();
        let at = usize::try_from(row["at"].as_u64().unwrap()).unwrap();
        let mounted = row["mounted"].as_bool().unwrap();

        let calls = Arc::new(Mutex::new(Vec::new()));
        let trace = calls.clone();
        let rule: LrIndentStrategy = Arc::new(move |cx| {
            let node = cx.node();
            trace.lock().unwrap().push(json!([
                span(node),
                span(node.children().next().unwrap()),
                span(node.children().next_back().unwrap()),
                cx.line_context().position(),
            ]));
            Some(7)
        });
        let install = INDENT.add(move |ty| ty.is("Body").then(|| Some(rule.clone())));
        let remove_body = INDENT.add(|ty| ty.is("Body").then_some(None));
        let lezer = || match mode {
            "override" => lezer_language().with_node_props(&[install]),
            "remove" => lezer_language().with_node_props(&[install, remove_body]),
            _ => lezer_language().clone(),
        };
        let extension = match grammar {
            "lezer" if mounted => {
                let parser = MarkdownCodeParser::Lezer(lezer());
                markdown_with_parser(
                    MarkdownConfig::default(),
                    MarkdownParserConfig {
                        code_languages: Some(MarkdownCodeLanguages::Configured {
                            resolve: Arc::new(move |info| {
                                (info == "lezer").then(|| parser.clone())
                            }),
                            fallback: None,
                        }),
                        ..Default::default()
                    },
                )
            }
            "lezer" => lezer().extension(),
            "json" if mode == "remove" => json_language()
                .with_node_props(&[INDENT.add(|ty| ty.is("Array").then_some(None))])
                .extension(),
            "json" => json_language().extension(),
            "wast" => wast_language().extension(),
            _ => unreachable!("fixture grammar: {grammar}"),
        };
        let state = EditorState::with_extensions(source, extension).unwrap();
        let options = [
            IndentOptions::default(),
            IndentOptions {
                simulate_break: Some(at),
                ..Default::default()
            },
            IndentOptions {
                simulate_break: Some(at),
                simulate_double_break: true,
                ..Default::default()
            },
        ];
        let columns = options.map(|opt| get_indentation(&state, at, opt, None).unwrap());
        assert_eq!(json!(columns), row["columns"], "{name} at byte {at}");
        assert_eq!(json!(*calls.lock().unwrap()), row["calls"], "{name}");
        if grammar == "lezer" && mode == "stock" && !mounted {
            let doc = LezerDocument::parse(Text::new(source));
            let direct = options.map(|opt| doc.indentation(at, StreamSettings::default(), opt));
            assert_eq!(direct, columns, "{name}: direct syntax query");
        }
    }
}
