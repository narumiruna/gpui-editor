use std::sync::{Arc, Mutex};

use gpui_editor_codemirror::{
    Assoc, Change, ChangeSet, Compartment, ConfigurationError, EditError, EditorSelection,
    EditorState, Extension, Text, Transaction,
    language::{
        HighlightTag, INDENT_UNIT, IndentOptions, IndentStyle, LanguageData,
        fold::{fold_service, foldable_in_state},
        get_indentation,
        lezer::{NodeProp, ParseError},
        wast::{
            FOLD, INDENT, LANGUAGE_DATA, SUBLANGUAGES, WastDataFacet, WastFoldCallback,
            WastIndentStrategy, WastNode, WastSublanguage, WastSublanguageMode, style_tags,
        },
        yaml::frontmatter_language::{YamlFrontmatterDocument, YamlFrontmatterLanguage},
    },
    language_data::{NativeSyntax, SyntaxUpdateError, wast_language},
};
use serde_json::{Value, json};

fn number(value: &Value) -> usize {
    usize::try_from(value.as_u64().unwrap()).unwrap()
}

fn property(value: &str) -> LanguageData {
    LanguageData::default().with("probe", value.to_owned())
}

fn nodes(doc: &YamlFrontmatterDocument, key: &NodeProp<&'static str>) -> Vec<Value> {
    let origin = doc.body_range().start;
    let NativeSyntax::Wast(wast) = doc.body() else {
        panic!("expected the configured WAST parser");
    };
    wast.nodes()
        .map(|node| {
            let range = node.range();
            json!([
                node.kind().name(),
                range.start + origin,
                range.end + origin,
                node.prop(key),
            ])
        })
        .collect()
}

#[test]
fn configured_wast_body_matches_pinned_syntax_metadata_and_indentation() {
    let key = NodeProp::new();
    let base = WastDataFacet::new(Some(property("root")));
    let extended = WastDataFacet::new(Some(property("extra")));
    let replaced = WastDataFacet::new(Some(property("replace")));
    let data_events = Arc::new(Mutex::new(Vec::new()));
    let indent_events = Arc::new(Mutex::new(Vec::new()));
    let fold_events = Arc::new(Mutex::new(Vec::new()));
    let languages = [("strict", "atom"), ("recover", "bool")].map(|(name, tag)| {
        let events = data_events.clone();
        let data = base.clone();
        let indent = indent_events.clone();
        let rule = move |node: WastNode<'_>, state: &EditorState| {
            let at = node.range();
            let name = node.kind().name();
            let head = state.selection().main().head();
            events
                .lock()
                .unwrap()
                .push(json!([name, at.start, at.end, head]));
            name == "Keyword" && head == 0
        };
        let rules = vec![
            WastSublanguage::new(WastSublanguageMode::Extend, extended.clone(), rule),
            WastSublanguage::new(WastSublanguageMode::Replace, replaced.clone(), |node, _| {
                node.kind().name() == "Keyword"
            }),
        ];
        let inspect: WastIndentStrategy = Arc::new(move |cx| {
            let next = cx.continue_indent();
            let lines = cx.line_context();
            let at = lines.position();
            let node = cx.node();
            let range = node.range();
            let base = cx.base_indent()?;
            let head = cx.state().expect("parent state").selection().main().head();
            indent.lock().unwrap().push(json!([
                node.kind().name(),
                range.start,
                range.end,
                at,
                head,
                lines.unit(),
                base,
                next,
                lines.text_after_pos(at, Assoc::After),
            ]));
            Some(base + lines.unit() + next.unwrap_or(0))
        });
        let fold = fold_events.clone();
        let fold_rule: WastFoldCallback = Arc::new(move |node, state| {
            let range = node.range();
            let head = state.selection().main().head();
            fold.lock()
                .unwrap()
                .push(json!([node.kind().name(), range.start, range.end, head]));
            if !state.text().slice(range.clone())?.starts_with("(module") {
                return None;
            }
            let first = state.text().line_at(range.start)?.range.end;
            Some(first - usize::from(head != 0)..range.end - 1)
        });
        let parser = wast_language()
            .configure(Some(name == "strict"), Some(name))
            .with_node_props(&[
                LANGUAGE_DATA.add(move |ty| ty.is_top().then(|| Some(data.clone()))),
                SUBLANGUAGES.add(move |ty| ty.is_top().then(|| rules.clone())),
                key.add(move |ty| ty.is("App").then_some(name)),
                style_tags([("Number", vec![HighlightTag::new(tag)])]).unwrap(),
                INDENT.add(move |ty| ty.is("App").then(|| inspect.clone())),
                FOLD.add(move |ty| ty.is("App").then(|| Some(fold_rule.clone()))),
            ]);
        YamlFrontmatterLanguage::with_wast(parser)
    });
    let fields = languages
        .each_ref()
        .map(|language| language.state_field(Extension::default()));
    let mut rows = include_str!("fixtures/yaml_frontmatter_wast.jsonl")
        .lines()
        .map(|line| serde_json::from_str::<Value>(line).unwrap());
    let initial = rows.next().unwrap();
    let slot = Compartment::new();
    let services = Compartment::new();
    let mut state = EditorState::with_extensions(
        initial["source"].as_str().unwrap(),
        Extension::group([
            slot.of(fields[0].extension()),
            services.of(Extension::default()),
            base.of(property("user")),
            INDENT_UNIT.of(IndentStyle::parse("   ").unwrap()),
        ]),
    )
    .unwrap();
    let field =
        |state: &EditorState, mode: usize| state.context().field(&fields[mode]).unwrap().unwrap();
    let mut mode = 0;
    for row in std::iter::once(initial).chain(rows) {
        let label = row["label"].as_str().unwrap();
        let next_mode = row["mode"]
            .as_str()
            .map_or(mode, |mode| usize::from(mode == "recover"));
        let previous = field(&state, mode);
        let changes = row["edit"]
            .as_array()
            .map_or_else(ChangeSet::default, |edit| {
                ChangeSet::single(Change::new(
                    number(&edit[0])..number(&edit[1]),
                    edit[2].as_str().unwrap(),
                ))
                .unwrap()
            });
        let mut transaction = Transaction::new(changes);
        if next_mode != mode {
            transaction =
                transaction.with_effects([slot.reconfigure(fields[next_mode].extension())]);
        }
        if row["selection"].is_number() {
            let at = number(&row["selection"]);
            transaction = transaction.with_selection(EditorSelection::single(at, at));
        }
        if row["rejected"] == true {
            let attempted = Text::new(row["attempted"].as_str().unwrap());
            let Some(SyntaxUpdateError::StrictSyntax(
                error @ ParseError::UnexpectedToken { offset, .. },
            )) = languages[next_mode].parse(attempted).err()
            else {
                panic!("expected strict syntax error: {label}");
            };
            assert_eq!(offset, number(&row["stop"]), "{label}");
            assert_eq!(
                state.update(transaction),
                Err(EditError::Configuration(ConfigurationError::StrictSyntax(
                    error
                ))),
                "{label}"
            );
            assert!(Arc::ptr_eq(&previous, &field(&state, mode)));
            continue;
        }
        if label != "strict" {
            state
                .update(transaction)
                .unwrap_or_else(|error| panic!("{label}: {error:?}"));
        }
        let current = field(&state, next_mode);
        let doc = current.as_ref().as_ref().unwrap();
        assert_eq!(doc.text().as_str(), row["source"].as_str().unwrap());
        let range = doc.body_range();
        assert_eq!(json!([range.start, range.end]), row["body"], "{label}");
        if label == "move selection" {
            assert!(Arc::ptr_eq(&previous, &current));
        }
        if label == "front matter grows" {
            assert!(std::ptr::eq(
                previous.as_ref().as_ref().unwrap().body(),
                doc.body()
            ));
        }
        assert_eq!(json!(nodes(doc, &key)), row["nodes"], "{label}");
        let highlights = doc
            .highlights(range.clone())
            .unwrap()
            .filter_map(|span| {
                let tag = span
                    .tags
                    .iter()
                    .map(HighlightTag::name)
                    .find(|tag| matches!(*tag, "atom" | "bool"))?;
                Some(json!([span.range.start, span.range.end, tag]))
            })
            .collect::<Vec<_>>();
        assert_eq!(json!(highlights), row["highlights"], "{label}");
        for expected in row["metadata"].as_array().unwrap() {
            let at = number(&expected[0]);
            for (side, expected) in [(Assoc::Before, &expected[1]), (Assoc::After, &expected[2])] {
                let actual = state.language_data_at::<String>("probe", at, side);
                assert_eq!(json!(actual), *expected, "{label}: {at} {side:?}");
            }
        }
        // CodeMirror can resolve the translated position outside the WAST mount.
        // Native WAST predicates do not receive those YAML wrapper or YAML nodes.
        let expected_data = row["dataCalls"]
            .as_array()
            .unwrap()
            .iter()
            .filter(|event| number(&event[1]) >= range.start)
            .cloned()
            .collect::<Vec<_>>();
        assert_eq!(
            std::mem::take(&mut *data_events.lock().unwrap()),
            expected_data,
            "{label}"
        );
        for query in row["indents"].as_array().unwrap() {
            let at = number(&query[0]);
            let simulate = query[1] == true;
            let result = get_indentation(
                &state,
                at,
                IndentOptions {
                    simulate_break: simulate.then_some(at),
                    simulate_double_break: simulate,
                    ..IndentOptions::default()
                },
                None,
            )
            .unwrap();
            assert_eq!(json!(result), query[2], "{label}: {at}");
            assert_eq!(
                json!(std::mem::take(&mut *indent_events.lock().unwrap())),
                query[3],
                "{label}: {at}"
            );
        }
        for query in row["folds"].as_array().unwrap() {
            let start = number(&query[0]);
            let end = number(&query[1]);
            let result =
                foldable_in_state(&state, start, end).map(|range| [range.start, range.end]);
            assert_eq!(json!(result), query[2], "{label}: fold at {start}..{end}");
            assert_eq!(
                json!(std::mem::take(&mut *fold_events.lock().unwrap())),
                query[3],
                "{label}: fold at {start}..{end}"
            );
        }
        let mut overridden = state.clone();
        let [start, end] = [range.start, range.end];
        overridden
            .update(
                Transaction::default().with_effects([services.reconfigure(fold_service(
                    move |_, from, to| (from == start).then_some(to..end),
                ))]),
            )
            .unwrap();
        let first = doc.body().text().line(1).unwrap().range;
        let result = foldable_in_state(&overridden, start, start + first.end)
            .map(|range| [range.start, range.end]);
        assert_eq!(
            json!(result),
            row["priorityFold"],
            "{label}: priority service"
        );
        assert!(fold_events.lock().unwrap().is_empty());
        mode = next_mode;
    }
}
