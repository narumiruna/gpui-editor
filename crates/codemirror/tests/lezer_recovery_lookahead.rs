//! Trees from the SHA-512-verified Lezer LR 1.4.13 and Common 1.5.7 packages.

use gpui_editor_codemirror::language::lezer::{
    ExternalOptions, ExternalToken, ExternalTokenizer, GrammarSpec, ParseError, Parser,
    SerializedTables, SyntaxTree, Tokenizer,
};
use serde_json::Value;

fn fixture() -> Value {
    serde_json::from_str(include_str!("fixtures/lezer_recovery_lookahead.json")).unwrap()
}

fn word(value: &Value) -> u16 {
    u16::try_from(value.as_u64().unwrap()).unwrap()
}

fn parser(fixture: &Value) -> Parser {
    let probe = word(&fixture["terms"]["Probe"]);
    let mark = word(&fixture["terms"]["mark"]);
    let tokenizer =
        ExternalTokenizer::new_deterministic(ExternalOptions::default(), move |input, _| {
            let start = input.position();
            match input.peek(0)? {
                Some(96) => {
                    while let Some(unit) = input.advance(1)? {
                        if unit == 96 {
                            return Ok(Some(ExternalToken {
                                term: probe,
                                end: input.position() + 1,
                            }));
                        }
                    }
                    Ok(None)
                }
                Some(126) => {
                    let mut offset = 1;
                    while let Some(unit) = input.peek(offset)? {
                        if unit == 96 {
                            return Ok(Some(ExternalToken {
                                term: probe,
                                end: start + offset as usize + 1,
                            }));
                        }
                        offset += 1;
                    }
                    Ok(Some(ExternalToken {
                        term: mark,
                        end: start + 1,
                    }))
                }
                _ => Ok(None),
            }
        });
    let slots = [Tokenizer::External(tokenizer), Tokenizer::Internal(0)];
    let table = &fixture["table"];
    assert_eq!(table["tokenizers"], serde_json::json!(["probe", 0]));
    let text = |key: &str| table[key].as_str().unwrap();
    let top = &table["topRules"]["Main"];
    let precedence: Vec<_> = table["dynamicPrecedences"]
        .as_object()
        .unwrap()
        .iter()
        .map(|(term, score)| {
            (
                term.parse().unwrap(),
                i8::try_from(score.as_i64().unwrap()).unwrap(),
            )
        })
        .collect();
    Parser::deserialize_with_hooks(
        GrammarSpec {
            tables: SerializedTables {
                version: u32::from(word(&table["version"])),
                states: text("states"),
                state_data: text("stateData"),
                goto: text("goto"),
                token_data: text("tokenData"),
            },
            node_names: text("nodeNames"),
            max_term: word(&table["maxTerm"]),
            repeat_node_count: word(&table["repeatNodeCount"]),
            tokenizers: &slots,
            top_rule: [word(&top[0]), word(&top[1])],
            token_prec: usize::from(word(&table["tokenPrec"])),
        },
        &[],
    )
    .unwrap()
    .with_dynamic_precedence(&precedence)
    .unwrap()
}

fn records(tree: &SyntaxTree) -> Vec<(usize, &str, usize, usize)> {
    let mut records = Vec::new();
    let mut pending = vec![(0, tree.root())];
    while let Some((depth, node)) = pending.pop() {
        let range = node.range();
        records.push((depth, node.name(), range.start, range.end));
        pending.extend(node.children().rev().map(|child| (depth + 1, child)));
    }
    records
}

#[test]
fn groups_recovery_errors_like_pinned_lezer() {
    let fixture = fixture();
    let parser = parser(&fixture);
    for case in fixture["cases"].as_array().unwrap() {
        let id = case["id"].as_str().unwrap();
        let source = case["source"].as_str().unwrap();
        let actual = parser.parse_recovering(source).unwrap();
        assert_eq!(
            serde_json::to_value(records(&actual)).unwrap(),
            case["nodes"],
            "{id}"
        );
        if id == "valid" {
            assert_eq!(records(&parser.parse(source).unwrap()), records(&actual));
            assert_eq!(
                records(&parser.parse_glr(source).unwrap()),
                records(&actual)
            );
            assert_eq!(
                records(&parser.parse_incremental(source, None).unwrap()),
                records(&actual)
            );
        } else if id != "cached-branch" {
            let error = parser.parse(source).unwrap_err();
            assert!(matches!(error, ParseError::UnexpectedToken { .. }), "{id}");
            assert_eq!(parser.parse_glr(source).unwrap_err(), error, "{id}");
            assert_eq!(
                parser.parse_incremental(source, None).unwrap_err(),
                error,
                "{id}"
            );
        }
    }
}

#[test]
fn bounds_repeated_long_lookahead() {
    let source = "~".repeat(4096);
    assert!(matches!(
        parser(&fixture()).parse_recovering(&source),
        Err(ParseError::LimitExceeded { .. })
    ));
}
