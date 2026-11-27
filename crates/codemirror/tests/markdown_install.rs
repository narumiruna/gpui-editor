use std::{sync::Arc, time::Duration};

use gpui_editor_codemirror::{
    Assoc, Change, ChangeSet, Compartment, EditorSelection, EditorState, Extension, Precedence,
    READ_ONLY, Text, Transaction,
    language::{
        fold::{fold_code, fold_service, foldable, folded_ranges},
        get_indentation,
        markdown::{
            MarkdownCompletionSource, MarkdownConfig, MarkdownMarkupCommand, markdown,
            markdown_completions, markdown_keymap_for, markdown_snapshot, plan_markdown_markup,
            plan_markdown_paste,
        },
    },
    language_data::NativeLanguage,
    view::{
        ClipboardLinewiseMemory, EditorView,
        keymap::{KeyBinding, KeyEvent, KeyPlatform, KeySequence, Keymap},
        plan_clipboard_paste,
    },
};
use serde_json::{Value, json};

const MANIFEST: &str = include_str!("../../../docs/codemirror-upstream.json");
const TRACE: &str = include_str!("fixtures/markdown_install.jsonl");

fn offset(value: &Value) -> usize {
    value.as_u64().unwrap().try_into().unwrap()
}

const ALL_OFF: MarkdownConfig = MarkdownConfig {
    add_keymap: false,
    complete_html_tags: false,
    paste_url_as_link: false,
};

fn configuration(options: &Value) -> Extension {
    if options.is_null() {
        return Extension::default();
    }
    let options = options.as_object().unwrap();
    let default = MarkdownConfig::default();
    let get = |name, default| {
        options
            .get(name)
            .map_or(default, |value| value.as_bool().unwrap())
    };
    markdown(MarkdownConfig {
        add_keymap: get("addKeymap", default.add_keymap),
        complete_html_tags: get("completeHTMLTags", default.complete_html_tags),
        paste_url_as_link: get("pasteURLAsLink", default.paste_url_as_link),
    })
}

fn assert_transaction(state: EditorState, plan: Option<Transaction>, expected: &Value) {
    if let Some(plan) = plan {
        assert_eq!(json!(plan.user_event()), expected["event"]);
        assert_eq!(plan.scroll_into_view(), expected["scroll"]);
        let mut view = EditorView::new(state.clone());
        view.update(plan, (), |_| (None, ())).unwrap();
        let selection = view.state().selection().main();
        assert_eq!(
            json!([selection.anchor(), selection.head()]),
            expected["selection"]
        );
        assert_eq!(view.state().text().as_str(), expected["output"]);
        assert!(view.undo().is_some());
        assert_eq!(view.state().text(), state.text());
        assert_eq!(view.state().selection(), state.selection());
    } else {
        assert!(expected["output"].is_null());
    }
}

#[derive(Clone, Copy, Eq, PartialEq)]
enum Command {
    Markup(MarkdownMarkupCommand),
    Fallback,
}

fn keymap(state: &EditorState) -> Keymap<Command> {
    let package = markdown_keymap_for(state)
        .into_iter()
        .flatten()
        .map(|binding| {
            KeyBinding::new(binding.key.unwrap(), Command::Markup(binding.run.unwrap()))
        });
    let fallback = ["Enter", "Backspace"].map(|key| KeyBinding::new(key, Command::Fallback));
    Keymap::new(KeyPlatform::Other, package.chain(fallback)).unwrap()
}

#[test]
fn markdown_install_matches_the_pinned_configuration_trace() {
    let manifest: Value = serde_json::from_str(MANIFEST).unwrap();
    let mut rows = TRACE
        .lines()
        .map(|line| serde_json::from_str::<Value>(line).unwrap());
    let provenance = rows.next().unwrap();
    assert_eq!(provenance["name"], "@codemirror/lang-markdown");
    assert_eq!(provenance["version"], "6.5.2");
    assert_eq!(
        provenance["capture"],
        "markdown(config): installed keymap, fold service, language data, and paste handler"
    );
    let pin = manifest["packages"]
        .as_array()
        .unwrap()
        .iter()
        .find(|pin| pin["name"] == provenance["name"])
        .unwrap();
    for key in ["version", "tarball", "integrity", "commit"] {
        assert_eq!(provenance[key], pin[key], "{key}");
    }

    let rows: Vec<_> = rows.collect();
    let modes = [
        "default",
        "no-keymap",
        "no-tag-source",
        "no-paste",
        "all-off",
        "removed",
        "restored",
    ];
    assert_eq!(rows.len(), modes.len());
    let compartment = Compartment::new();
    let source = rows[0]["source"].as_str().unwrap();
    let mut state = EditorState::create_from_text(
        Text::new(source),
        None,
        compartment.of(configuration(&rows[0]["config"])),
    )
    .unwrap();
    for (row, mode) in rows.iter().zip(modes) {
        assert_eq!(row["mode"], mode);
        assert_eq!(row["source"], source);
        if mode != "default" {
            state
                .update(
                    Transaction::default()
                        .with_effects([compartment.reconfigure(configuration(&row["config"]))]),
                )
                .unwrap();
        }
        assert_eq!(state.text().as_str(), source);
        let snapshot = markdown_snapshot(&state);
        assert_eq!(snapshot.is_some(), mode != "removed");
        for key in row["keys"].as_array().unwrap() {
            let at = offset(&key["at"]);
            let mut selected = state.clone();
            selected
                .update(Transaction::default().with_selection(EditorSelection::single(at, at)))
                .unwrap();
            let mut plan = None;
            let mut fallback = false;
            let result = keymap(&selected).dispatch(
                "editor",
                KeyEvent::new(key["key"].as_str().unwrap(), Default::default()),
                Duration::ZERO,
                &mut KeySequence::default(),
                |command| match command {
                    Command::Markup(command) => {
                        plan = plan_markdown_markup(&selected, *command).unwrap();
                        plan.is_some()
                    }
                    Command::Fallback => {
                        fallback = true;
                        true
                    }
                },
            );
            assert_eq!(result.handled, key["handled"], "{mode}: {key}");
            assert_eq!(fallback, key["fallback"], "{mode}: {key}");
            assert_transaction(selected, plan, key);
        }
        for fold in row["folds"].as_array().unwrap() {
            let [from, to] = [offset(&fold["from"]), offset(&fold["to"])];
            assert_eq!(
                json!(
                    foldable(&state, from, to, |_, _, _| None)
                        .map(|range| [range.start, range.end])
                ),
                fold["range"],
                "{mode}: {fold}"
            );
            let syntax = snapshot.as_ref().map(|snapshot| snapshot.syntax());
            let indentation = get_indentation(&state, from, Default::default(), syntax).unwrap();
            assert_eq!(json!(indentation), fold["indent"], "{mode}: {fold}");
        }
        let completions = row["completions"].as_array().unwrap();
        assert_eq!(completions.len(), 2);
        let prose = &completions[0];
        let sources = state.language_data_at::<MarkdownCompletionSource>(
            "autocomplete",
            offset(&prose["at"]),
            Assoc::Before,
        );
        assert_eq!(sources.len(), prose["count"], "{mode}");
        let mounted_at = offset(&completions[1]["at"]);
        if let Some(snapshot) = &snapshot {
            assert_eq!(
                snapshot.language_at(mounted_at, Assoc::Before),
                Some(NativeLanguage::Html)
            );
        }
        for completion in completions {
            let at = offset(&completion["at"]);
            let results: Vec<_> = markdown_completions(&state, at, false)
                .into_iter()
                .map(|result| {
                    let labels: Vec<_> = result
                        .options
                        .iter()
                        .filter_map(|item| {
                            let label = item.label.as_str();
                            ["<span", "<div", "class"].contains(&label).then_some(label)
                        })
                        .collect();
                    json!({"range": [result.from, result.to.unwrap_or(at)], "labels": labels})
                })
                .collect();
            assert_eq!(
                json!(results),
                completion["results"],
                "{mode}: {completion}"
            );
        }
        let paste = &row["paste"];
        let mut selected = state.clone();
        let [anchor, head] = [
            offset(&paste["selection"][0]),
            offset(&paste["selection"][1]),
        ];
        selected
            .update(Transaction::default().with_selection(EditorSelection::single(anchor, head)))
            .unwrap();
        let clipboard = row["clipboard"].as_str().unwrap();
        let plan = plan_markdown_paste(&selected, clipboard).unwrap();
        assert_eq!(plan.is_some(), paste["expected"]["handled"], "{mode}");
        if plan.is_none() {
            let mut fallback = selected.clone();
            let normal =
                plan_clipboard_paste(&fallback, clipboard, &ClipboardLinewiseMemory::default())
                    .unwrap()
                    .unwrap();
            fallback.update(normal).unwrap();
            assert_eq!(
                fallback.text().as_str(),
                format!("{}{clipboard}{}", &source[..head], &source[anchor..])
            );
        }
        assert_transaction(selected, plan, &paste["expected"]);
    }
}

#[test]
fn snapshot_tracks_edits_and_heading_fold_defers_on_other_blocks() {
    let compartment = Compartment::new();
    let source = "# Top 😀\r\n\r\n> line\r\n> tail\r\n\r\n# Next\r\n";
    let mut state = EditorState::create_from_text(
        Text::new(source),
        None,
        compartment.of(markdown(MarkdownConfig::default())),
    )
    .unwrap();
    let before = markdown_snapshot(&state).unwrap();
    let line = state.text().line(3).unwrap().range;
    let fallback = |_: &EditorState, from, to| before.syntax().foldable(from, to);
    assert_eq!(foldable(&state, line.start, line.end, |_, _, _| None), None);
    assert_eq!(
        foldable(&state, line.start, line.end, fallback),
        Some(line.end..source.find("\r\n\r\n# Next").unwrap())
    );

    state
        .update(Transaction::default().with_effects([compartment.reconfigure(markdown(ALL_OFF))]))
        .unwrap();
    assert!(Arc::ptr_eq(&before, &markdown_snapshot(&state).unwrap()));
    let at = source.find("tail").unwrap();
    let changes = ChangeSet::single(Change::new(at..at + "tail".len(), "longer 猫")).unwrap();
    state.update(Transaction::new(changes)).unwrap();
    let updated = markdown_snapshot(&state).unwrap();
    let output = source.replace("tail", "longer 猫");
    assert_eq!(updated.text().as_str(), output);
    assert_eq!(before.text().as_str(), source);
    let from = output.find("\r\n").unwrap();
    let to = output.find("\r\n\r\n# Next").unwrap();
    state
        .update(fold_code(&state, |_, _, _| None).unwrap())
        .unwrap();
    assert!(Arc::ptr_eq(&updated, &markdown_snapshot(&state).unwrap()));
    let folds: Vec<_> = folded_ranges(&state)
        .iter(0)
        .map(|fold| fold.from..fold.to)
        .collect();
    assert_eq!(folds, vec![from..to]);

    let higher_service =
        fold_service(move |_, _, _| Some(from..from + 2)).with_precedence(Precedence::High);
    let reconfigure =
        compartment.reconfigure(Extension::group([markdown(ALL_OFF), higher_service]));
    state
        .update(Transaction::default().with_effects([reconfigure]))
        .unwrap();
    assert_eq!(
        foldable(&state, 0, from, |_, _, _| None),
        Some(from..from + 2)
    );
}

#[test]
fn read_only_and_invalid_positions_do_not_produce_edits_or_completions() {
    let mut state = EditorState::with_extensions(
        "- 猫",
        Extension::group([markdown(MarkdownConfig::default()), READ_ONLY.of(true)]),
    )
    .unwrap();
    state
        .update(Transaction::default().with_selection(EditorSelection::single(5, 5)))
        .unwrap();
    assert!(
        plan_markdown_markup(&state, MarkdownMarkupCommand::Continue)
            .unwrap()
            .is_none()
    );
    state
        .update(Transaction::default().with_selection(EditorSelection::single(2, 5)))
        .unwrap();
    assert!(
        plan_markdown_paste(&state, "https://example.test")
            .unwrap()
            .is_none()
    );
    for invalid in [3, 4, 6, usize::MAX] {
        assert!(markdown_completions(&state, invalid, true).is_empty());
    }
}
