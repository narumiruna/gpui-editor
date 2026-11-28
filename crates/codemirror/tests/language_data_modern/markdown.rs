use std::{
    cell::Cell,
    sync::{Arc, Mutex},
    time::Duration,
};

use gpui_editor_codemirror::{
    Assoc, Change, ChangeSet, Compartment, EditorSelection, EditorState, Extension, Precedence,
    READ_ONLY, Text, Transaction,
    autocomplete::session::SourceResult,
    commands::{CommentTokens, bindings::CommandId},
    language::{
        IndentDecision, IndentOptions,
        css::CssPropertySnapshot,
        fold::{fold_service, foldable_in_state},
        get_indentation, indent_service,
        javascript::javascript_completion_snippet,
        markdown::{MarkdownNodeKind, markdown_snapshot},
    },
    language_data::{
        CloseBracketData, NativeSyntax, ScriptCompletionSource, attached_language_snapshot,
    },
    setup::{SetupCommand, minimal_setup, setup_keymap},
    view::{
        ClipboardLinewiseMemory, EditorView,
        keymap::{KeyBinding, KeyEvent, KeyModifiers, KeyPlatform},
        plan_clipboard_paste, update_listener,
    },
};
use serde_json::{Value, json};

use super::{bundle, byte};

fn rows(kind: &'_ str) -> impl Iterator<Item = Value> + '_ {
    include_str!("../fixtures/language_data_markdown.jsonl")
        .lines()
        .map(|line| serde_json::from_str::<Value>(line).unwrap())
        .filter(move |row| row["kind"] == kind)
}

fn sources(state: &EditorState, at: usize, side: Assoc) -> Vec<ScriptCompletionSource> {
    state.language_data_at("autocomplete", at, side)
}

fn css() -> Extension {
    CssPropertySnapshot::from_style_keys([
        "zIndex",
        "display",
        "backgroundColor",
        "cssFloat",
        "cssText",
    ])
    .extension()
}

fn summary(id: &str, result: SourceResult, at: usize, state: &EditorState) -> Value {
    let to = result.to.unwrap_or(at);
    assert!(state.text().slice(result.from..to).is_some());
    let mut hash = 2166136261u32;
    let mut labels = Vec::new();
    for item in result.options.iter() {
        let apply = if id.ends_with("-fixed") && javascript_completion_snippet(item).is_some() {
            "@snippet"
        } else {
            item.insert.as_deref().unwrap_or("")
        };
        for b in format!(
            "{}\0{}\0{}\0{}\0{apply}\n",
            item.label,
            item.kind.as_deref().unwrap_or(""),
            item.detail.as_deref().unwrap_or(""),
            item.boost
        )
        .bytes()
        {
            hash = (hash ^ u32::from(b)).wrapping_mul(16777619);
        }
        if [
            "<span", "class", "div", "display", "local", "const", "--tone",
        ]
        .contains(&item.label.as_str())
        {
            labels.push(&item.label);
        }
    }
    json!({"range": [result.from, to], "count": result.options.len(), "hash": hash, "labels": labels})
}

fn selected_extension(names: &[&str]) -> Extension {
    Extension::group(names.iter().map(|name| match *name {
        "high HTML" => bundle("HTML").with_precedence(Precedence::High),
        name => bundle(name),
    }))
}

#[test]
fn stock_markdown_replays_tracked_catalog_and_mounted_data() {
    let attach = rows("state").find(|row| row["step"] == "attach").unwrap();
    let original = attach["source"].as_str().unwrap();
    let pos = |name: &str| {
        byte(
            &attach["queries"]
                .as_array()
                .unwrap()
                .iter()
                .find(|query| query["label"] == name)
                .unwrap()["at"],
        )
    };
    let md = EditorState::with_extensions(original, bundle("Markdown")).unwrap();
    let html = EditorState::with_extensions(original, bundle("HTML")).unwrap();
    let js = EditorState::with_extensions("", bundle("JavaScript")).unwrap();
    let standalone_css = EditorState::with_extensions("", bundle("CSS")).unwrap();
    let get = |state, at| sources(state, at, Assoc::Before);
    let [tag] = get(&md, pos("prose")).try_into().ok().unwrap();
    let [schema] = get(&md, pos("block attr")).try_into().ok().unwrap();
    let [html_schema] = get(&html, pos("block attr")).try_into().ok().unwrap();
    let [fixed, local] = get(&md, pos("JS raw")).try_into().ok().unwrap();
    let [html_fixed, html_local] = get(&html, pos("JS raw")).try_into().ok().unwrap();
    let [_, js_local] = get(&js, 0).try_into().ok().unwrap();
    let [mounted_css] = get(&md, pos("CSS raw")).try_into().ok().unwrap();
    let [html_css] = get(&html, pos("CSS raw")).try_into().ok().unwrap();
    let [css_source] = get(&standalone_css, 0).try_into().ok().unwrap();
    assert!(Arc::ptr_eq(&local, &js_local) && Arc::ptr_eq(&html_local, &js_local));
    assert!(Arc::ptr_eq(&mounted_css, &css_source) && Arc::ptr_eq(&html_css, &css_source));
    assert!(!Arc::ptr_eq(&schema, &html_schema) && !Arc::ptr_eq(&fixed, &html_fixed));
    let ids = [
        ("M-tag", tag),
        ("M-schema", schema),
        ("H-schema", html_schema),
        ("M-JS-fixed", fixed),
        ("H-JS-fixed", html_fixed),
        ("JS-local", local),
        ("CSS", css_source),
    ];
    let slot = Compartment::new();
    let mut state = EditorState::with_extensions(
        original,
        Extension::group([slot.of(Extension::default()), css()]),
    )
    .unwrap();
    let mut retained = None;
    let mut retained_tree = None;
    for row in rows("state").filter(|row| row["step"] != "bare") {
        let step = row["step"].as_str().unwrap();
        let names: Option<&[&str]> = match step {
            "attach" | "restore M" => Some(&["Markdown"]),
            "repeat" => Some(&["Markdown", "Markdown"]),
            "M then H" => Some(&["Markdown", "HTML"]),
            "high H" => Some(&["Markdown", "high HTML"]),
            "H then M" => Some(&["HTML", "Markdown"]),
            "old HTML only" => Some(&["HTML"]),
            "detach" => Some(&[]),
            _ => None,
        };
        let before = attached_language_snapshot(&state);
        let mut tr = Transaction::default();
        if let Some(names) = names {
            tr = tr.with_effects([slot.reconfigure(selected_extension(names))]);
        } else if step == "select" {
            tr = tr.with_selection(EditorSelection::single(4, 4));
        } else if step == "edit" {
            let at = original.find("plain 猫").unwrap();
            tr = Transaction::new(
                ChangeSet::single(Change::new(at..at + "plain 猫".len(), "edited 犬")).unwrap(),
            );
        }
        if step == "retained" {
            state = retained.take().unwrap();
        } else {
            state.update(tr).unwrap();
        }
        assert_eq!(state.text().as_str(), row["source"], "{step}");
        assert!(
            markdown_snapshot(&state).is_none(),
            "no explicit Markdown field: {step}"
        );
        let snapshot = attached_language_snapshot(&state);
        if step == "attach" {
            retained = Some(state.clone());
            retained_tree = snapshot.clone();
        } else if step == "retained" {
            assert!(Arc::ptr_eq(
                retained_tree.as_ref().unwrap(),
                snapshot.as_ref().unwrap()
            ));
        } else {
            let reused = before
                .as_ref()
                .zip(snapshot.as_ref())
                .is_some_and(|(a, b)| Arc::ptr_eq(a, b));
            assert_eq!(json!(reused), row["sameTree"], "{step}");
        }
        let active = match snapshot.as_deref().map(|active| active.syntax()) {
            Some(NativeSyntax::Markdown(doc)) => {
                assert_eq!(doc.root().kind().name(), row["root"]);
                assert!(
                    !doc.nodes()
                        .any(|node| node.kind() == MarkdownNodeKind::TaskMarker)
                );
                Some("M")
            }
            Some(NativeSyntax::Html(doc)) => {
                assert_eq!(doc.root().kind().name(), row["root"]);
                Some("H")
            }
            None => None,
            _ => unreachable!(),
        };
        assert_eq!(json!(active), row["active"], "{step}");
        for (index, heading) in ["# Top", "## Child", "# Next"].into_iter().enumerate() {
            let start = state.text().as_str().find(heading).unwrap();
            let end = start + state.text().as_str()[start..].find('\r').unwrap();
            let actual = json!(foldable_in_state(&state, start, end).map(|r| [r.start, r.end]));
            let expected = if active == Some("H") && heading == "# Next" {
                // Existing standalone HTML recovery folds across the malformed opening tag.
                assert_eq!(row["folds"][index][0], Value::Null);
                json!([106, 228])
            } else {
                row["folds"][index][0].clone()
            };
            assert_eq!(actual, expected, "{step} {heading}");
        }
        for query in row["queries"].as_array().unwrap() {
            let at = byte(&query["at"]);
            for (i, side) in [Assoc::Before, Assoc::After].into_iter().enumerate() {
                let wanted = &query["sides"][i];
                let label = format!("{step} {} {at} {side:?}", query["label"]);
                let available = sources(&state, at, side);
                let names: Vec<_> = available
                    .iter()
                    .map(|f| {
                        ids.iter()
                            .find(|(_, source)| Arc::ptr_eq(f, source))
                            .unwrap()
                            .0
                    })
                    .collect();
                assert_eq!(json!(names), wanted["sources"], "{label}");
                let comments: Vec<_> = state
                    .language_data_at::<CommentTokens>("commentTokens", at, side)
                    .iter()
                    .map(|tokens| {
                        let mut data = json!({});
                        if let Some(line) = tokens.line {
                            data["line"] = json!(line);
                        }
                        if let Some((open, close)) = tokens.block {
                            data["block"] = json!({"open": open, "close": close});
                        }
                        data
                    })
                    .collect();
                assert_eq!(json!(comments), wanted["comment"], "{label}");
                assert_eq!(
                    json!(state.language_data_at::<&str>("wordChars", at, side)),
                    wanted["word"],
                    "{label}"
                );
                let brackets: Vec<_> = state
                    .language_data_at::<CloseBracketData>("closeBrackets", at, side)
                    .iter()
                    .map(|data| json!({"brackets": data.brackets}))
                    .collect();
                assert_eq!(json!(brackets), wanted["brackets"], "{label}");
                let rules = state.language_data_at::<fn(&str) -> bool>("indentOnInput", at, side);
                let indent: Vec<_> = rules
                    .iter()
                    .map(|rule| [rule("  </div>"), rule("  }"), rule("  case ")])
                    .collect();
                let wanted_indent: Vec<_> = wanted["indent"]
                    .as_array()
                    .unwrap()
                    .iter()
                    .map(|pattern| match pattern.as_str().unwrap() {
                        r"^\s*<\/\w+\W$" => [true, false, false],
                        r"^\s*\}$" => [false, true, false],
                        r"^\s*(?:case |default:|\{|\}|<\/)$" => [false, true, true],
                        _ => panic!("unknown indent trigger {pattern}"),
                    })
                    .collect();
                assert_eq!(indent, wanted_indent, "{label}");
                for (mode, explicit) in [false, true].into_iter().enumerate() {
                    for (j, (f, id)) in available.iter().zip(&names).enumerate() {
                        let mut expected = wanted["results"][mode][j].clone();
                        if let Some(fields) = expected.as_object_mut() {
                            fields.remove("validFor");
                        }
                        let result = f(&state, at, explicit).map(|r| summary(id, r, at, &state));
                        assert_eq!(json!(result), expected, "{label} {id} explicit={explicit}");
                    }
                }
            }
        }
    }
}

#[test]
fn stock_markdown_uses_original_html_bytes_and_rejects_gap_spanning_edits() {
    let quote = rows("quote").next().unwrap();
    let source = quote["source"].as_str().unwrap();
    let state = EditorState::with_extensions(source, bundle("Markdown")).unwrap();
    let before_end = source.find('\r').unwrap();
    let fold = foldable_in_state(&state, 0, before_end).map(|r| [r.start, r.end]);
    assert_eq!(json!(fold), quote["fold"]);
    for probe in quote["probes"].as_array().unwrap() {
        let at = byte(&probe[1]);
        for (i, side) in [Assoc::Before, Assoc::After].into_iter().enumerate() {
            let actual = if state
                .language_data_at::<&str>("wordChars", at, side)
                .is_empty()
            {
                "M"
            } else {
                "H"
            };
            assert_eq!(actual, probe[2][i], "{} {side:?}", probe[0]);
        }
    }
    let quote_indent = source.find(">   text").unwrap() + 2;
    assert_eq!(
        json!(get_indentation(&state, quote_indent, IndentOptions::default(), None).unwrap()),
        quote["indent"]
    );

    let source =
        "> <input type=\"猫\r\n>   text\" cl>\r\n> <script>let local = 1;\r\n>   loc;</script>";
    let state = EditorState::with_extensions(source, bundle("Markdown")).unwrap();
    let at = source.find("text\"").unwrap() + 4;
    let [schema] = sources(&state, at, Assoc::Before).try_into().ok().unwrap();
    assert!(
        schema(&state, at, true).is_none(),
        "quoted value begins before a quote-marker gap"
    );
    let attr = source.find("cl>").unwrap();
    let result = schema(&state, attr + 2, false).unwrap();
    assert_eq!((result.from, result.to), (attr, Some(attr + 2)));
    assert!(result.options.iter().any(|option| option.label == "class"));
    let js = source.find("loc;").unwrap();
    let [fixed, local] = sources(&state, js + 3, Assoc::Before)
        .try_into()
        .ok()
        .unwrap();
    let result = local(&state, js + 3, false).unwrap();
    assert_eq!((result.from, result.to.unwrap_or(js + 3)), (js, js + 3));
    assert!(result.options.iter().any(|option| option.label == "local"));
    for at in [
        source.find('猫').unwrap() + 1,
        source.find('\r').unwrap() + 1,
    ] {
        for side in [Assoc::Before, Assoc::After] {
            assert!(sources(&state, at, side).is_empty(), "byte {at} {side:?}");
            assert!(
                state
                    .language_data_at::<CommentTokens>("commentTokens", at, side)
                    .is_empty()
            );
        }
        for f in [&schema, &fixed, &local] {
            assert!(f(&state, at, true).is_none());
        }
    }
}

#[test]
fn stock_markdown_closes_a_tag_on_the_original_quoted_line() {
    let source = "> <section>\r\n>   猫\r\n>   <>tail\r\n";
    let at = source.find("<>tail").unwrap() + 1;
    let state = EditorState::create_from_text(
        Text::new(source),
        Some(EditorSelection::single(at, at)),
        bundle("Markdown"),
    )
    .unwrap();
    let mut view = EditorView::<()>::new(state);
    view.apply_headless_input(
        at..at,
        "/",
        |state| {
            Ok(state
                .replace_selection_parsed("/")?
                .with_user_event("input.type"))
        },
        (),
    )
    .unwrap();
    let state = view.state();
    assert_eq!(
        state.text().as_str(),
        "> <section>\r\n>   猫\r\n>   </section>tail\r\n"
    );
    assert_eq!(state.selection().main().head(), at + "/section>".len());
}

#[test]
fn stock_markdown_dispatches_mounted_folds_and_indents() {
    let row = rows("mounted-syntax-services").next().unwrap();
    let source = row["source"].as_str().unwrap();
    let state = EditorState::with_extensions(source, bundle("Markdown")).unwrap();
    let mut start = 0;
    for (line, want) in source.split("\r\n").zip(row["lines"].as_array().unwrap()) {
        let end = start + line.len();
        let fold = foldable_in_state(&state, start, end).map(|r| [r.start, r.end]);
        assert_eq!(json!(fold), want["fold"], "{line}");
        assert_eq!(
            json!(get_indentation(&state, start, IndentOptions::default(), None).unwrap()),
            want["indent"],
            "{line}"
        );
        start = end + 2;
    }
}

#[test]
fn stock_mounted_html_folding_defers_to_user_services() {
    let row = rows("mounted-fold-precedence").next().unwrap();
    let source = row["source"].as_str().unwrap();
    let text = Text::new(source);
    let second = text.line(2).unwrap().range;
    let service =
        fold_service(move |_, start, end| (start == second.start).then_some(end - 1..end + 2));
    let markdown = bundle("Markdown");
    let observed = [
        Extension::group([service.clone(), markdown.clone()]),
        Extension::group([markdown.clone(), service]),
        markdown,
    ]
    .map(|extension| {
        let state = EditorState::with_extensions(source, extension).unwrap();
        [1, 2].map(|n| {
            let line = text.line(n).unwrap().range;
            foldable_in_state(&state, line.start, line.end).map(|r| [r.start, r.end])
        })
    });
    assert_eq!(json!(observed), row["folds"]);
}

#[test]
fn stock_heading_fold_follows_the_configured_service_order() {
    let row = rows("heading-fold-precedence").next().unwrap();
    let source = row["source"].as_str().unwrap();
    let service = fold_service(|_, start, end| (start == 0).then_some(end - 1..end + 2));
    let markdown = bundle("Markdown");
    let folds = [
        Extension::group([service.clone(), markdown.clone()]),
        Extension::group([markdown.clone(), service.clone()]),
        Extension::group([markdown.clone(), service.with_precedence(Precedence::High)]),
        markdown,
    ]
    .map(|extension| {
        let state = EditorState::with_extensions(source, extension).unwrap();
        let line = state.text().line(1).unwrap().range;
        foldable_in_state(&state, line.start, line.end).map(|r| [r.start, r.end])
    });
    assert_eq!(json!(folds), row["folds"]);
}

#[test]
fn stock_contiguous_html_projects_mounted_indent_options() {
    for row in rows("mounted-indent-options") {
        let source = row["source"].as_str().unwrap();
        let state = EditorState::with_extensions(source, bundle("Markdown")).unwrap();
        for query in row["queries"].as_array().unwrap() {
            let at = byte(&query[0]);
            let wanted_line = (!query[1].is_null()).then(|| byte(&query[1]));
            let lookup = |line| (Some(line) == wanted_line).then_some(7);
            let options = IndentOptions {
                override_indentation: Some(&lookup),
                simulate_break: (query[2] == true).then_some(at),
                simulate_double_break: query[3].as_bool().unwrap(),
            };
            let actual = get_indentation(&state, at, options, None).unwrap();
            assert_eq!(json!(actual), query[4], "{source:?}: {query:?}");
        }
    }
}

#[test]
fn stock_mounted_html_indentation_defers_to_user_services() {
    let row = rows("mounted-indent-precedence").next().unwrap();
    let source = row["source"].as_str().unwrap();
    let at = byte(&row["at"]);
    let service = indent_service(move |_, context| {
        if context.position() == at {
            IndentDecision::Column(11)
        } else {
            IndentDecision::Defer
        }
    });
    let stop = indent_service(|_, _| IndentDecision::Inherit);
    let markdown = bundle("Markdown");
    let columns = [
        Extension::group([service.clone(), markdown.clone()]),
        Extension::group([markdown.clone(), service]),
        Extension::group([markdown.clone(), stop]),
        markdown,
    ]
    .map(|extension| {
        let state = EditorState::with_extensions(source, extension).unwrap();
        get_indentation(&state, at, IndentOptions::default(), None).unwrap()
    });
    assert_eq!(json!(columns), row["columns"]);
}

fn input_summary(tr: &Transaction, after: &EditorState) -> Value {
    let edits: Vec<_> = tr
        .changes()
        .changes()
        .map(|(r, insert)| json!([r.start, r.end, insert]))
        .collect();
    let range = after.selection().main();
    let assoc = match range.assoc() {
        Some(Assoc::Before) => -1,
        Some(Assoc::After) => 1,
        None => 0,
    };
    json!({"edits":edits, "doc":after.text().as_str(), "selection":[range.anchor(), range.head(), assoc],
        "event":tr.user_event(), "scroll":tr.scroll_into_view()})
}

#[test]
fn stock_markdown_replays_key_paste_and_shared_html_input() {
    for row in rows("input") {
        let label = row["label"].as_str().unwrap();
        let source = row["source"].as_str().unwrap();
        let mode = row["mode"].as_str().unwrap();
        let action = row["action"].as_str().unwrap();
        let wanted = row["transactions"].as_array().unwrap();
        let names: &[&str] = match label {
            "no key opt-out" | "no paste opt-out" => &[],
            "inactive Markdown key" | "inactive Markdown paste" | "old HTML owns when first" => {
                &["HTML", "Markdown"]
            }
            "Markdown owns when first" => &["Markdown", "HTML"],
            "old HTML only" => &["HTML"],
            _ => &["Markdown", "Markdown"],
        };
        let highest = if mode == "highest" {
            setup_keymap(vec![KeyBinding::new(
                "Enter",
                SetupCommand::Editor(CommandId::SelectAll),
            )])
            .with_precedence(Precedence::Highest)
        } else {
            Extension::default()
        };
        let input_steps = Arc::new(Mutex::new(Vec::new()));
        let log = Arc::clone(&input_steps);
        let state = EditorState::create_from_text(
            Text::new(source),
            Some(EditorSelection::single(
                byte(&row["selection"][0]),
                byte(&row["selection"][1]),
            )),
            Extension::group([
                selected_extension(names),
                READ_ONLY.of(mode == "readOnly"),
                highest,
                update_listener(move |update| {
                    log.lock().unwrap().extend(
                        update
                            .input_steps
                            .iter()
                            .map(|step| input_summary(&step.transaction, &step.state)),
                    );
                    Ok(())
                }),
            ]),
        )
        .unwrap();
        if let Some(clipboard) = action.strip_prefix("paste:") {
            let tr = plan_clipboard_paste(&state, clipboard, &ClipboardLinewiseMemory::default())
                .unwrap();
            if state.read_only() {
                // The pinned direct handler has no host read-only check. Native dispatch does.
                assert_eq!(row["handled"], true);
                assert!(tr.is_none(), "{label}");
                continue;
            }
            let tr = tr.unwrap();
            let mut after = state.clone();
            after.update(tr.clone()).unwrap();
            if let Some(wanted) = wanted.first() {
                assert_eq!(input_summary(&tr, &after), *wanted, "{label}");
            } else {
                let range = state.selection().main().range();
                let mut expected = source.to_owned();
                expected.replace_range(range, clipboard);
                assert_eq!(after.text().as_str(), expected, "{label}");
            }
        } else if matches!(action, "Enter" | "Backspace") {
            let mut editor = minimal_setup::<()>(state, KeyPlatform::Linux);
            let (outcome, _) = editor
                .key_event(
                    KeyEvent::new(action, KeyModifiers::default()),
                    Duration::ZERO,
                    (),
                )
                .unwrap();
            let final_state = editor.view().state();
            if let Some(wanted) = wanted.first().filter(|_| !final_state.read_only()) {
                assert!(outcome.handled, "{label}");
                let mut doc = wanted["doc"].as_str().unwrap().to_owned();
                let mut selection = [byte(&wanted["selection"][0]), byte(&wanted["selection"][1])];
                if label == "continue" {
                    // The native command reuses the adjacent CRLF for the new line.
                    let at = byte(&row["selection"][0]);
                    assert_eq!(&doc[at..], "\n- ");
                    doc.insert(at, '\r');
                    selection.iter_mut().for_each(|at| *at += 1);
                }
                assert_eq!(final_state.text().as_str(), doc, "{label}");
                let range = final_state.selection().main();
                assert_eq!([range.anchor(), range.head()], selection, "{label}");
            } else if mode == "highest" || final_state.read_only() {
                assert_eq!(final_state.text().as_str(), source, "{label}");
            } else {
                assert_eq!(row["fallback"], true);
                assert!(outcome.handled, "{label}");
                assert_ne!(final_state.text().as_str(), "- item\n- ", "{label}");
            }
        } else {
            if state.read_only() || mode == "composition" {
                assert!(wanted.is_empty());
                continue;
            }
            let range = state.selection().main().range();
            let mut view = EditorView::<()>::new(state);
            let calls = Cell::new(0);
            view.apply_headless_input(
                range,
                action,
                |state| {
                    calls.set(calls.get() + 1);
                    Ok(state
                        .replace_selection_parsed(action)?
                        .with_user_event("input.type")
                        .with_scroll_into_view())
                },
                (),
            )
            .unwrap();
            assert_eq!(calls.get(), 1, "{label}");
            let actual = input_steps.lock().unwrap();
            if wanted.is_empty() {
                assert_eq!(actual.len(), 1, "{label}: host default only");
                assert_eq!(actual[0]["event"], "input.type", "{label}");
            } else {
                assert_eq!(*actual, *wanted, "{label}");
            }
        }
    }
}
