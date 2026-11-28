use std::{num::NonZeroUsize, sync::Mutex};

use gpui_editor_codemirror::{
    TAB_SIZE,
    language::{
        INDENT_UNIT, IndentDecision, IndentOptions, IndentStyle, get_indentation, indent_service,
        lezer::{INDENT, LrIndentStrategy, LrSyntaxNode},
        yaml::frontmatter_language::YamlFrontmatterLanguage,
    },
};

use super::*;

fn num(value: &Value) -> usize {
    usize::try_from(value.as_u64().unwrap()).unwrap()
}

fn span(node: LrSyntaxNode<'_>) -> Value {
    let range = node.range();
    json!([node.name(), range.start, range.end])
}

#[test]
fn lr_indent_properties_match_pinned_queries_and_continuations() {
    for row in include_str!("../fixtures/lr_indent_props.jsonl").lines() {
        let row: Value = serde_json::from_str(row).unwrap();
        let [stock, _] = handles(&row);
        let name = row["name"].as_str().unwrap();
        let scope = row["scope"].as_str().unwrap();
        let source = row["input"].as_str().unwrap();
        let events = Arc::new(Mutex::new(Vec::new()));
        let configure = |offset: usize, enabled: bool| {
            let events = events.clone();
            let rule: LrIndentStrategy = Arc::new(move |cx| {
                let state = cx.state().expect("current query state");
                let lines = cx.line_context();
                assert_eq!(state.text().len(), lines.text().len());
                assert_eq!(state.tab_size(), lines.settings().tab_size);
                let next = cx.continue_indent();
                let base = cx.base_indent()?;
                let pos = lines.position();
                events.lock().unwrap().push(json!([
                    span(cx.node()),
                    cx.node().parent().map(span),
                    pos,
                    state.selection().main().head(),
                    base,
                    lines.unit(),
                    next,
                    lines.text_after_pos(pos, Assoc::After),
                ]));
                Some(offset + base + lines.unit() + next.unwrap_or(0))
            });
            let stop: LrIndentStrategy = Arc::new(|_| None);
            let target = row["target"].as_str().unwrap().to_owned();
            let leaf = row["leaf"].as_str().unwrap().to_owned();
            let sources = [INDENT.add(move |ty| {
                if ty.is(&target) {
                    Some(enabled.then(|| rule.clone()))
                } else if enabled && ty.is(&leaf) {
                    Some(Some(stop.clone()))
                } else {
                    None
                }
            })];
            match &stock {
                MarkdownCodeParser::Json(lang) => {
                    MarkdownCodeParser::Json(lang.with_node_props(&sources))
                }
                MarkdownCodeParser::Lezer(lang) => {
                    MarkdownCodeParser::Lezer(lang.with_node_props(&sources))
                }
                _ => unreachable!(),
            }
        };
        let install = |parser| {
            if scope == "standalone" {
                with_lr!(parser, lang => lang.extension())
            } else if scope == "yaml" {
                match parser {
                    MarkdownCodeParser::Json(lang) => YamlFrontmatterLanguage::with_json(lang),
                    MarkdownCodeParser::Lezer(lang) => YamlFrontmatterLanguage::with_lezer(lang),
                    _ => unreachable!(),
                }
                .state_field(Extension::default())
                .extension()
            } else {
                let name = name.to_owned();
                markdown_with_parser(
                    MarkdownConfig::default(),
                    MarkdownParserConfig {
                        code_languages: Some(MarkdownCodeLanguages::Configured {
                            resolve: Arc::new(move |info| (info == name).then(|| parser.clone())),
                            fallback: None,
                        }),
                        ..Default::default()
                    },
                )
            }
        };
        let slot = Compartment::new();
        let create = |extension| {
            EditorState::with_extensions(
                source,
                Extension::group([
                    TAB_SIZE.of(NonZeroUsize::new(8).unwrap()),
                    INDENT_UNIT.of(IndentStyle::parse("   ").unwrap()),
                    extension,
                ]),
            )
            .unwrap()
        };
        let original = create(slot.of(install(configure(7, true))));
        let snapshot = |state: &EditorState| {
            attached_language_snapshot(state)
                .or_else(|| markdown_snapshot(state))
                .unwrap()
        };
        let update = |state: &EditorState, tx| {
            let mut state = state.clone();
            state.update(tx).unwrap();
            state
        };
        let head = num(&row["head"]);
        let selected = update(
            &original,
            Transaction::default()
                .with_selection(gpui_editor_codemirror::EditorSelection::single(head, head)),
        );
        if scope != "yaml" {
            assert!(Arc::ptr_eq(&snapshot(&original), &snapshot(&selected)));
        }
        let reconfigure = |parser| {
            update(
                &selected,
                Transaction::default().with_effects([slot.reconfigure(install(parser))]),
            )
        };
        let replacement = reconfigure(configure(11, true));
        let disabled = reconfigure(configure(0, false));
        if scope != "yaml" {
            assert!(!Arc::ptr_eq(&snapshot(&selected), &snapshot(&replacement)));
        }
        let baseline = create(install(stock.clone()));
        let service = create(Extension::group([
            install(configure(7, true)),
            indent_service(|_, _| IndentDecision::Column(17)),
        ]));
        let edit_at = num(&row["editAt"]);
        let edit = ChangeSet::single(Change::new(
            edit_at..edit_at,
            row["insert"].as_str().unwrap(),
        ))
        .unwrap();
        let edited = update(&selected, Transaction::new(edit));
        for check in row["checks"].as_array().unwrap() {
            let step = check["step"].as_str().unwrap();
            let state = match step {
                "stock" => &baseline,
                "configured" => &original,
                "replacement" => &replacement,
                "disabled" => &disabled,
                "service" => &service,
                "edited" => &edited,
                _ => &selected,
            };
            let pos = num(&check["at"]);
            let override_indent = |at| (at == num(&row["bodyLine"])).then_some(5);
            let options = if step == "simulated" {
                IndentOptions {
                    override_indentation: Some(&override_indent),
                    simulate_break: Some(pos),
                    simulate_double_break: true,
                }
            } else {
                IndentOptions::default()
            };
            let column = get_indentation(state, pos, options, None).unwrap();
            assert_eq!(json!(column), check["column"], "{name} {scope} {step}");
            let calls = std::mem::take(&mut *events.lock().unwrap());
            assert_eq!(json!(calls), check["calls"], "{name} {scope} {step}");
        }
        assert_eq!(original.text().as_str(), source);
    }
}
