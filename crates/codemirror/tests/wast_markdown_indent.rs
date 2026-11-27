use std::sync::{Arc, Mutex};

use gpui_editor_codemirror::{
    Assoc, Change, ChangeSet, Compartment, EditorSelection, EditorState, Extension, Transaction,
    language::{
        INDENT_UNIT, IndentDecision, IndentOptions, IndentStyle, MarkdownWastIndent,
        StreamSettings, get_indentation, indent_service,
        markdown::{
            MarkdownCodeLanguages, MarkdownCodeParser, MarkdownConfig, MarkdownParserConfig,
            markdown_snapshot, markdown_with_parser,
        },
        wast::{INDENT, WastIndentContext, WastIndentStrategy, WastNode},
    },
    language_data::{NativeSyntax, wast_extension, wast_language},
};
use serde_json::{Value, json};

fn slot(parser: MarkdownCodeParser) -> Extension {
    markdown_with_parser(
        MarkdownConfig::default(),
        MarkdownParserConfig {
            code_languages: Some(MarkdownCodeLanguages::Configured {
                resolve: Arc::new(move |info| (info == "wat").then(|| parser.clone())),
                fallback: None,
            }),
            ..Default::default()
        },
    )
}

fn span(node: WastNode<'_>) -> Value {
    let range = node.range();
    json!([node.node_type().name(), range.start, range.end])
}

fn record(cx: &WastIndentContext<'_>, next: Option<usize>) -> Value {
    let lines = cx.line_context();
    let position = lines.position();
    let line = lines.line_at(position, Assoc::After).unwrap();
    json!([
        span(cx.node()),
        cx.node().parent().map(span),
        position,
        [
            json!(line.range.start),
            json!(line.range.end),
            json!(line.text)
        ],
        cx.base_indent(),
        next,
        cx.state().unwrap().selection().main().head()
    ])
}

fn byte(value: &Value) -> usize {
    usize::try_from(value.as_u64().unwrap()).unwrap()
}

#[test]
fn mounted_indent_matches_pinned_columns_and_parent_context() {
    for line in include_str!("fixtures/wast_markdown_indent.jsonl").lines() {
        let row: Value = serde_json::from_str(line).unwrap();
        let source = row["source"].as_str().unwrap();
        let calls = Arc::new(Mutex::new(Vec::new()));
        let app_calls = Arc::clone(&calls);
        let app: WastIndentStrategy = Arc::new(move |cx| {
            let next = cx.continue_indent();
            app_calls.lock().unwrap().push(record(cx, next));
            Some(next.unwrap_or(0) + cx.base_indent()? + cx.line_context().unit())
        });
        let comment_calls = Arc::clone(&calls);
        let comment: WastIndentStrategy = Arc::new(move |cx| {
            comment_calls.lock().unwrap().push(record(cx, None));
            None
        });
        let stock = wast_language();
        let configured = stock.with_node_props(&[INDENT.add(move |node| match node.name() {
            "App" => Some(app.clone()),
            "BlockComment" => Some(comment.clone()),
            _ => None,
        })]);
        let create = |parser, extra| {
            let mut state = EditorState::with_extensions(
                source,
                Extension::group([
                    slot(MarkdownCodeParser::Wast(parser)),
                    INDENT_UNIT.of(IndentStyle::parse("   ").unwrap()),
                    extra,
                ]),
            )
            .unwrap();
            let at = byte(&row["selection"]);
            state
                .update(Transaction::default().with_selection(EditorSelection::single(at, at)))
                .unwrap();
            state
        };
        let state = create(configured.clone(), Extension::default());
        let control = create(stock.clone(), Extension::default());
        for query in row["queries"].as_array().unwrap() {
            let position = byte(&query["at"]);
            let overrides = |at| (query["overrideAt"].as_u64() == Some(at as u64)).then_some(9);
            let simulate = query["doubleBreak"].as_bool().unwrap();
            let options = IndentOptions {
                simulate_break: simulate.then_some(position),
                simulate_double_break: simulate,
                override_indentation: Some(&overrides),
            };
            let message = format!("{} {}", row["name"], query["label"]);
            assert_eq!(
                json!(get_indentation(&state, position, options, None).unwrap()),
                query["result"],
                "{message}"
            );
            assert_eq!(
                json!(std::mem::take(&mut *calls.lock().unwrap())),
                query["calls"],
                "{message}"
            );
            assert_eq!(
                json!(get_indentation(&control, position, options, None).unwrap()),
                query["stock"],
                "{message}"
            );
        }
        let at = source.find("12").unwrap();
        let with_service = create(
            configured,
            indent_service(move |state, context| {
                assert_eq!(context.position(), at);
                assert!(state.text().as_str().contains("Tail"));
                IndentDecision::Column(19)
            }),
        );
        assert_eq!(
            get_indentation(&with_service, at, IndentOptions::default(), None),
            Ok(Some(19))
        );
        assert!(calls.lock().unwrap().is_empty());
    }
}

#[test]
fn mounted_indent_uses_current_snapshot_and_preserves_explicit_syntax_precedence() {
    let source = "```wat\r\n(module\r  (func)\r\n)\r\n```";
    let compartment = Compartment::new();
    let stock = || slot(MarkdownCodeParser::Wast(wast_language().clone()));
    let observed: WastIndentStrategy = Arc::new(|cx| {
        let state = cx.state().unwrap();
        let node = cx.node();
        assert!(state.text().slice(node.range()).unwrap().starts_with('('));
        Some(if state.text().as_str().starts_with("<!--") {
            11
        } else {
            7
        })
    });
    let custom = wast_language()
        .with_node_props(&[INDENT.add(move |node| node.is("App").then(|| observed.clone()))]);
    let mut state = EditorState::with_extensions(source, compartment.of(stock())).unwrap();
    let retained = state.clone();
    let at = source.find("func").unwrap();
    let query =
        |state: &EditorState, at| get_indentation(state, at, IndentOptions::default(), None);
    assert_eq!(query(&state, at), Ok(Some(4)));
    let update = Transaction::default()
        .with_effects([compartment.reconfigure(slot(MarkdownCodeParser::Wast(custom)))]);
    state.update(update).unwrap();
    assert_eq!(query(&state, at), Ok(Some(7)));
    assert_eq!(query(&retained, at), Ok(Some(4)));
    let explicit = markdown_snapshot(&retained).unwrap();
    assert!(matches!(explicit.syntax(), NativeSyntax::Markdown(_)));
    assert_eq!(
        get_indentation(
            &state,
            at,
            IndentOptions::default(),
            Some(explicit.syntax())
        ),
        Ok(None)
    );
    let insert = "<!-- x -->\n\n";
    state
        .update(Transaction::new(
            ChangeSet::single(Change::new(0..0, insert)).unwrap(),
        ))
        .unwrap();
    assert_eq!(query(&state, at + insert.len()), Ok(Some(11)));
    // A standalone syntax attachment takes precedence over the Markdown mount.
    state
        .update(Transaction::default().with_effects([wast_extension().append()]))
        .unwrap();
    assert_ne!(query(&state, at + insert.len()), Ok(Some(11)));
}

#[test]
fn body_indent_requires_the_current_parser_and_exact_contiguous_ranges() {
    let body = "(module\r\n  (data \"😀\")\r\n)";
    let source = format!("前\r\n\r\n```wat\r\n{body}\r\n```\r\n");
    let from = source.find(body).unwrap();
    let range = from..from + body.len();
    let first = wast_language();
    let second = first.configure(None, None);
    let compartment = Compartment::new();
    let install = |language| slot(MarkdownCodeParser::Wast(language));
    let mut state =
        EditorState::with_extensions(&source, compartment.of(install(first.clone()))).unwrap();
    let original = markdown_snapshot(&state).unwrap();
    let context = MarkdownWastIndent::new(&state, &original, range.clone(), first).unwrap();
    assert_eq!(context.syntax().text().as_str(), body);
    let query = |position| {
        context.at(
            position,
            StreamSettings::default(),
            IndentOptions::default(),
        )
    };
    assert_eq!(query(1), Some(2));
    for invalid in [
        0,
        body.len(),
        body.find('\n').unwrap(),
        body.find('😀').unwrap() + 1,
        usize::MAX,
    ] {
        assert_eq!(query(invalid), None, "body position {invalid}");
    }
    assert!(MarkdownWastIndent::new(&state, &original, from..range.end - 1, first).is_none());
    assert!(MarkdownWastIndent::new(&state, &original, range.clone(), &second).is_none());
    let swap =
        Transaction::default().with_effects([compartment.reconfigure(install(second.clone()))]);
    state.update(swap).unwrap();
    assert_eq!(state.text().as_str(), source);
    let replacement = markdown_snapshot(&state).unwrap();
    assert!(MarkdownWastIndent::new(&state, &original, range.clone(), first).is_none());
    assert!(MarkdownWastIndent::new(&state, &replacement, range.clone(), first).is_none());
    assert!(MarkdownWastIndent::new(&state, &replacement, range, &second).is_some());

    let quoted = EditorState::with_extensions(
        "> ```wat\r\n> (module\r\n>   (func))\r\n> ```",
        install(first.clone()),
    )
    .unwrap();
    let snapshot = markdown_snapshot(&quoted).unwrap();
    let regions = snapshot.wast_regions();
    assert!(regions.len() > 1);
    let covering = regions[0].start..regions.last().unwrap().end;
    assert!(MarkdownWastIndent::new(&quoted, &snapshot, covering, first).is_none());
    assert!(MarkdownWastIndent::new(&quoted, &snapshot, regions[0].clone(), first).is_none());
}
