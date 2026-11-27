use std::{cell::Cell, ops::Range, sync::Mutex, time::Instant};

use futures_util::FutureExt;
use gpui_editor_codemirror::{
    Change, ChangePermission, ChangeSet, EditError, EditorState, Extension, READ_ONLY, Transaction,
    change_filter,
    language::{
        html::{HtmlDocument, HtmlEmbeddedLanguage},
        javascript::{EslintFinding, EslintFix, JavascriptOptions, es_lint},
        markdown::MarkdownDocument,
    },
    language_data::NativeLanguage,
    lint::{self, Diagnostic, LintConfig, LintSession, LintSource, LintState, Severity},
    view::EditorView,
};
use serde_json::{Value, json};

fn javascript(typescript: bool, jsx: bool) -> NativeLanguage {
    NativeLanguage::Javascript(JavascriptOptions { typescript, jsx })
}

fn regions_for(mode: &str, state: &EditorState) -> Vec<(NativeLanguage, Range<usize>)> {
    let text = state.text();
    let language = match mode {
        "none" => NativeLanguage::Json,
        "js" => javascript(false, false),
        "jsx" => javascript(false, true),
        "ts" => javascript(true, false),
        "tsx" => javascript(true, true),
        "html" => {
            use HtmlEmbeddedLanguage::*;
            return HtmlDocument::parse(text.clone())
                .mounts()
                .iter()
                .filter_map(|mount| {
                    let language = match mount.language() {
                        // The pinned HTML JSON route has JS language data.
                        JavaScript | Json => javascript(false, false),
                        Jsx => javascript(false, true),
                        TypeScript => javascript(true, false),
                        Tsx => javascript(true, true),
                        Css | CssStyles => NativeLanguage::Css,
                        Custom(_) => return None,
                    };
                    Some((language, mount.range()))
                })
                .collect();
        }
        "markdown" => {
            return MarkdownDocument::parse(text.clone())
                .mounts(|name| match name {
                    "js" => Some(javascript(false, false)),
                    "tsx" => Some(javascript(true, true)),
                    "css" => Some(NativeLanguage::Css),
                    _ => None,
                })
                .flat_map(|mount| {
                    let language = mount.language();
                    mount
                        .source_ranges()
                        .map(|range| (language, range))
                        .collect::<Vec<_>>()
                })
                .collect();
        }
        _ => panic!("unknown language: {mode}"),
    };
    vec![(language, 0..text.len())]
}

fn number(value: &Value) -> usize {
    usize::try_from(value.as_u64().unwrap()).unwrap()
}

fn byte_range(value: &Value) -> Range<usize> {
    number(&value[0])..number(&value[1])
}

fn finding(value: &Value) -> EslintFinding {
    EslintFinding {
        line: number(&value["line"]),
        column: number(&value["column"]),
        end_line: value.get("endLine").map(number),
        end_column: value.get("endColumn").map(number),
        severity: u8::try_from(number(&value["severity"])).unwrap(),
        message: value["message"].as_str().unwrap().into(),
        rule_id: value["ruleId"].as_str().map(str::to_owned),
        fix: value.get("fix").map(|fix| EslintFix {
            range: byte_range(&fix["range"]),
            text: fix["text"].as_str().unwrap().into(),
        }),
    }
}

fn payload(diagnostic: &Diagnostic) -> Value {
    json!({
        "range": [diagnostic.range.start, diagnostic.range.end],
        "severity": match diagnostic.severity {
            Severity::Warning => "warning",
            Severity::Error => "error",
            _ => panic!("unexpected severity"),
        },
        "source": diagnostic.source,
        "message": diagnostic.message,
        "markClass": diagnostic.mark_class,
        "actions": diagnostic.actions.iter().map(|action| action.name.as_str()).collect::<Vec<_>>(),
    })
}

fn run(state: &mut EditorState) -> Vec<Diagnostic> {
    let now = Instant::now();
    let mut session = LintSession::new(state, now);
    let (mut batch, mut futures) = lint::force_linting(&mut session, state, now).unwrap();
    assert_eq!(futures.len(), 1);
    let diagnostics = futures.pop().unwrap().now_or_never().unwrap().unwrap();
    state
        .update(
            batch
                .complete(0, Ok(diagnostics.clone()), now)
                .unwrap()
                .unwrap(),
        )
        .unwrap();
    diagnostics
}

fn change(range: Range<usize>, text: &str) -> Transaction {
    Transaction::new(ChangeSet::single(Change::new(range, text)).unwrap())
}

fn with_provider(source: &str, provider: LintSource, extra: Extension) -> EditorState {
    EditorState::with_extensions(
        source,
        Extension::group([lint::linter(Some(provider), LintConfig::default()), extra]),
    )
    .unwrap()
}

#[test]
fn host_verifier_matches_pinned_findings_and_relative_fixes() {
    for line in include_str!("fixtures/javascript_eslint.jsonl").lines() {
        let row: Value = serde_json::from_str(line).unwrap();
        let raw = row["doc"].as_str().unwrap();
        let mode = row["mode"].as_str().unwrap().to_owned();
        let expected_regions: Vec<_> = row["regions"]
            .as_array()
            .unwrap()
            .iter()
            .map(byte_range)
            .collect();
        let native_regions: Vec<_> = regions_for(&mode, &EditorState::new(raw))
            .into_iter()
            .filter_map(|(lang, range)| {
                matches!(lang, NativeLanguage::Javascript(_)).then_some(range)
            })
            .collect();
        assert_eq!(native_regions, expected_regions, "{mode}");
        let calls = row["calls"].as_array().unwrap();
        let responses: Vec<_> = calls
            .iter()
            .zip(row["findings"].as_array().unwrap())
            .map(|(call, findings)| {
                (
                    call["input"].as_str().unwrap().to_owned(),
                    findings
                        .as_array()
                        .unwrap()
                        .iter()
                        .map(finding)
                        .collect::<Vec<_>>(),
                )
            })
            .collect();
        let seen = std::sync::Arc::new(Mutex::new(Vec::new()));
        let rule_lookups = Cell::new(0);
        let provider = es_lint(
            move |state| regions_for(&mode, state),
            {
                let seen = seen.clone();
                move |input, config| {
                    seen.lock()
                        .unwrap()
                        .push(json!({"input": input, "config": config}));
                    responses
                        .iter()
                        .find(|(source, _)| source == input)
                        .unwrap()
                        .1
                        .clone()
                }
            },
            row.get("config").cloned(),
            || {
                rule_lookups.set(rule_lookups.get() + 1);
                row["recommended"]
                    .as_array()
                    .unwrap()
                    .iter()
                    .map(|name| name.as_str().unwrap().to_owned())
                    .collect()
            },
        );
        assert_eq!(rule_lookups.get(), number(&row["ruleLookups"]));
        let mut state = with_provider(raw, provider, Extension::default());
        let diagnostics = run(&mut state);
        assert_eq!(*seen.lock().unwrap(), *calls);
        assert_eq!(
            diagnostics.iter().map(payload).collect::<Vec<_>>(),
            *row["payload"].as_array().unwrap()
        );

        if let Some(rebase) = row.get("rebase") {
            // The oracle uses a start insertion in both coordinate systems.
            assert_eq!(rebase["from"], 0);
            state
                .update(change(0..0, rebase["insert"].as_str().unwrap()))
                .unwrap();
        }
        let active = LintState::get(&state).unwrap().panel_diagnostics();
        for fix in row["fixes"].as_array().unwrap() {
            let diagnostic = active.iter().find(|d| fix["message"] == d.message).unwrap();
            let action = lint::run_action(&state, diagnostic, 0).unwrap().unwrap();
            let range = byte_range(&fix["range"]);
            assert_eq!(
                action.changes().changes().collect::<Vec<_>>(),
                [(&range, fix["insert"].as_str().unwrap())]
            );
            assert_eq!(action.scroll_into_view(), fix["scroll"].as_bool().unwrap());
            assert!(action.effects().is_empty() && action.selection().is_none());
            let mut view = EditorView::new(state.clone());
            view.update(action, (), |_| (None, ())).unwrap();
            assert_eq!(view.state().text().as_str(), fix["after"].as_str().unwrap());
        }
    }
}

fn point(line: usize, column: usize, message: &str) -> EslintFinding {
    EslintFinding {
        line,
        column,
        end_line: None,
        end_column: None,
        severity: 2,
        message: message.into(),
        rule_id: None,
        fix: None,
    }
}

fn with_fix(finding: EslintFinding, range: Range<usize>, text: &str) -> EslintFinding {
    EslintFinding {
        fix: Some(EslintFix {
            range,
            text: text.into(),
        }),
        ..finding
    }
}

#[test]
fn rejects_invalid_scopes_findings_and_fixes_without_clipping() {
    let js = javascript(false, false);
    let provider = es_lint(
        move |_| {
            vec![
                (NativeLanguage::Css, 0..1),
                (js, 0..2), // inside the emoji
                (js, 0..7), // between CR and LF
                (js, 7..10),
                (js, Range { start: 9, end: 8 }),
                (js, 0..usize::MAX),
                (js, 1..9),
            ]
        },
        |input, _| {
            assert_eq!(input, "😀x\nz");
            vec![
                point(0, 1, "zero line"),
                point(1, 0, "zero column"),
                point(usize::MAX, 1, "bad line"),
                point(2, usize::MAX, "bad column"),
                point(1, 2, "surrogate midpoint"),
                point(1, 5, "column past line"),
                point(2, 3, "outside region"),
                EslintFinding {
                    end_line: Some(1),
                    end_column: Some(3),
                    ..point(2, 1, "reversed end")
                },
                EslintFinding {
                    end_line: Some(2),
                    ..point(1, 1, "missing end column")
                },
                EslintFinding {
                    end_line: Some(2),
                    end_column: Some(3),
                    ..point(1, 1, "end outside")
                },
                EslintFinding {
                    end_line: Some(1),
                    end_column: Some(2),
                    ..point(1, 1, "end on surrogate")
                },
                EslintFinding {
                    end_line: Some(99),
                    end_column: Some(1),
                    ..point(2, 1, "collapsed end")
                },
                with_fix(point(1, 1, "split surrogate fix"), 1..2, "bad"),
                with_fix(point(1, 1, "out of scope fix"), 0..6, "bad"),
                with_fix(
                    point(1, 1, "reversed fix"),
                    Range { start: 2, end: 0 },
                    "bad",
                ),
                with_fix(point(1, 4, "insert at CRLF"), 3..3, "!"),
            ]
        },
        None,
        Vec::new,
    );
    let mut state = with_provider("[😀x\r\nz]", provider, Extension::default());
    let diagnostics = run(&mut state);
    assert_eq!(
        diagnostics
            .iter()
            .map(|d| (d.message.as_str(), d.range.clone(), d.actions.len()))
            .collect::<Vec<_>>(),
        [
            ("collapsed end", 8..8, 0),
            ("split surrogate fix", 1..1, 0),
            ("out of scope fix", 1..1, 0),
            ("reversed fix", 1..1, 0),
            ("insert at CRLF", 6..6, 1),
        ]
    );
    let insertion = LintState::get(&state)
        .unwrap()
        .panel_diagnostics()
        .into_iter()
        .find(|d| d.message == "insert at CRLF")
        .unwrap();
    state
        .update(lint::run_action(&state, &insertion, 0).unwrap().unwrap())
        .unwrap();
    assert_eq!(state.text().as_str(), "[😀x!\r\nz]");
}

fn html_provider() -> LintSource {
    es_lint(
        |state| regions_for("html", state),
        |_, _| vec![with_fix(point(1, 5, "good"), 0..3, "okay")],
        None,
        Vec::new,
    )
}

#[test]
fn fix_rechecks_its_text_and_language_and_respects_read_only() {
    let html = "<script>bad good</script>";
    let mut state = with_provider(html, html_provider(), Extension::default());
    run(&mut state);
    let diagnostic = LintState::get(&state)
        .unwrap()
        .panel_diagnostics()
        .pop()
        .unwrap();
    assert!(lint::run_action(&state, &diagnostic, 0).unwrap().is_some());

    let mut changed_target = state.clone();
    changed_target.update(change(8..11, "BAD")).unwrap();
    assert_eq!(
        lint::run_action(&changed_target, &diagnostic, 0).unwrap_err(),
        EditError::InvalidRange
    );

    state.update(change(0..8, "<style>")).unwrap();
    assert_eq!(
        lint::run_action(&state, &diagnostic, 0).unwrap_err(),
        EditError::InvalidRange
    );

    let mut read_only = with_provider(html, html_provider(), READ_ONLY.of(true));
    run(&mut read_only);
    let diagnostic = LintState::get(&read_only)
        .unwrap()
        .panel_diagnostics()
        .pop()
        .unwrap();
    assert_eq!(
        lint::run_action(&read_only, &diagnostic, 0).unwrap_err(),
        EditError::InvalidRange
    );
}

#[test]
fn fix_does_not_bypass_filters_or_apply_unaccepted_changes() {
    let html = "<script>bad good</script>";
    let mut state = with_provider(
        html,
        html_provider(),
        change_filter(|_, _| ChangePermission::Reject),
    );
    run(&mut state);
    let diagnostic = LintState::get(&state)
        .unwrap()
        .panel_diagnostics()
        .pop()
        .unwrap();
    let proposed = lint::run_action(&state, &diagnostic, 0).unwrap().unwrap();
    let mut view = EditorView::new(state);
    let rejected = view.update_checked(
        proposed.clone(),
        (),
        |accepted| {
            assert!(accepted.changes().is_empty());
            if accepted.changes() != proposed.changes() {
                return Err(EditError::InvalidRange);
            }
            Ok(())
        },
        |_| (None, ()),
    );
    assert_eq!(rejected, Err(EditError::InvalidRange));
    assert_eq!(view.state().text().as_str(), html);
    assert_eq!(view.undo_depth(), 0);
}
