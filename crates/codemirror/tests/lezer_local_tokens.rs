//! Generated tables and trees from the pinned Lezer and CodeMirror JS packages.
//! The oracle compared `LRParser.parse` with `LRLanguage` on `EditorState`
//! documents, including edited states. The Rust tests do not run JavaScript.

use gpui_editor_codemirror::language::lezer::{
    GrammarError, GrammarSpec, LocalTokenGroup, ParseError, Parser, SerializedTables,
    Specialization, Specializer, SyntaxTree, TableError, Tokenizer, TreeFragment,
};
use serde_json::{Value, json};

fn fixture() -> Value {
    serde_json::from_str(include_str!("fixtures/lezer_local_tokens.json")).unwrap()
}

fn text(value: &Value) -> &str {
    value.as_str().unwrap()
}

fn word(value: &Value) -> u16 {
    u16::try_from(value.as_u64().unwrap()).unwrap()
}

fn spec<'a>(table: &'a Value, tokenizers: &'a [Tokenizer]) -> GrammarSpec<'a, Tokenizer> {
    let top = table["topRules"]
        .as_object()
        .unwrap()
        .values()
        .next()
        .unwrap();
    GrammarSpec {
        tables: SerializedTables {
            version: u32::from(word(&table["version"])),
            states: text(&table["states"]),
            state_data: text(&table["stateData"]),
            goto: text(&table["goto"]),
            token_data: text(&table["tokenData"]),
        },
        node_names: text(&table["nodeNames"]),
        max_term: word(&table["maxTerm"]),
        repeat_node_count: word(&table["repeatNodeCount"]),
        tokenizers,
        top_rule: [word(&top[0]), word(&top[1])],
        token_prec: usize::from(word(&table["tokenPrec"])),
    }
}

fn table<'a>(fixture: &'a Value, id: &str) -> &'a Value {
    &fixture["grammars"]
        .as_array()
        .unwrap()
        .iter()
        .find(|grammar| grammar["id"] == id)
        .unwrap()["table"]
}

fn parser(table: &Value) -> Parser {
    let slots: Vec<_> = table["tokenizers"]
        .as_array()
        .unwrap()
        .iter()
        .map(|slot| {
            if slot.is_number() {
                Tokenizer::Internal(u8::try_from(word(slot)).unwrap())
            } else {
                Tokenizer::Local(
                    LocalTokenGroup::new(
                        text(&slot["data"]),
                        usize::from(word(&slot["precTable"])),
                        slot.get("elseToken").map(word),
                    )
                    .unwrap(),
                )
            }
        })
        .collect();
    let specializers: Vec<_> = table["specialized"]
        .as_array()
        .into_iter()
        .flatten()
        .map(|entry| {
            let result = word(&entry["result"]);
            let specialization = if result & 1 == 0 {
                Specialization::Replace(result >> 1)
            } else {
                Specialization::Extend(result >> 1)
            };
            Specializer::context_free(
                word(&entry["term"]),
                &[(text(&entry["text"]), specialization)],
            )
        })
        .collect();
    Parser::deserialize_with_hooks(spec(table, &slots), &specializers).unwrap()
}

fn check_tree(tree: &SyntaxTree, case: &Value) {
    let source = text(&case["source"]);
    let mut records = Vec::new();
    let mut pending = vec![tree.root()];
    while let Some(node) = pending.pop() {
        let range = node.range();
        // JS reports UTF-16 ranges. Slicing here also checks byte alignment.
        records.push(json!([
            node.name(),
            source[..range.start].encode_utf16().count(),
            source[..range.end].encode_utf16().count()
        ]));
        pending.extend(node.children().rev());
    }
    assert_eq!(Value::Array(records), case["nodes"], "source: {source:?}");
}

#[test]
fn parses_generated_local_groups_like_the_js_oracle() {
    for grammar in fixture()["grammars"].as_array().unwrap() {
        let parser = parser(&grammar["table"]);
        for case in grammar["valid"].as_array().unwrap() {
            let source = text(&case["source"]);
            check_tree(&parser.parse(source).unwrap(), case);
            check_tree(&parser.parse_glr(source).unwrap(), case);
            check_tree(&parser.parse_recovering(source).unwrap(), case);
        }
        for case in grammar["recovery"].as_array().unwrap() {
            let source = text(&case["source"]);
            assert!(matches!(
                parser.parse(source),
                Err(ParseError::UnexpectedToken { .. })
            ));
            check_tree(&parser.parse_recovering(source).unwrap(), case);
        }
    }
}

#[test]
fn checkpoints_depend_on_reads_past_the_else_token() {
    let fixture = fixture();
    let parser = parser(table(&fixture, "mixed"));
    let edits = &fixture["edits"];
    let before = &edits["before"];
    let previous = parser
        .parse_incremental(text(&before["source"]), None)
        .unwrap();
    check_tree(&previous, before);

    for name in ["far", "late"] {
        let case = &edits[name];
        let end = usize::from(word(&case["at"]));
        let tree = parser
            .parse_incremental(
                text(&case["source"]),
                Some(TreeFragment {
                    tree: &previous,
                    old_range: 0..end,
                    new_range: 0..end,
                }),
            )
            .unwrap();
        check_tree(&tree, case);
        let reuse = tree.reuse_stats();
        if name == "far" {
            // A failed LongClose probe read to the edit, beyond the Text end
            // and beyond the read extent of the final successful Close probe.
            assert!(reuse.resumed_at < 81, "{reuse:?}");
        } else {
            assert!(
                reuse.resumed_at >= 82 && reuse.nodes_reused > 0,
                "{reuse:?}"
            );
        }
    }
}

#[test]
fn checkpoints_depend_on_eof_after_an_else_only_group() {
    let fixture = fixture();
    let parser = parser(table(&fixture, "else_only"));
    let [before, after] = ["before", "after"].map(|name| &fixture["append"][name]);
    let source = text(&before["source"]);
    let previous = parser.parse_incremental(source, None).unwrap();
    check_tree(&previous, before);
    let tree = parser
        .parse_incremental(
            text(&after["source"]),
            Some(TreeFragment {
                tree: &previous,
                old_range: 0..source.len(),
                new_range: 0..source.len(),
            }),
        )
        .unwrap();
    check_tree(&tree, after);
    assert_eq!(tree.reuse_stats().resumed_at, 0);
}

// Small handcrafted tables for cases the generator cannot emit.
fn encode(words: &[u16]) -> String {
    std::iter::once(u16::try_from(words.len()).unwrap())
        .chain(words.iter().copied())
        .map(|word| {
            if word == u16::MAX {
                return '~';
            }
            assert!(word < 46);
            let byte = b'O' + word as u8;
            char::from(byte + u8::from(byte >= b'\\'))
        })
        .collect()
}

#[test]
fn rejects_invalid_local_data_and_reserved_tokens() {
    const END: u16 = u16::MAX;
    assert!(matches!(
        LocalTokenGroup::new("not encoded", 3, None),
        Err(GrammarError::Tables(TableError::Encoding {
            table: "local token data",
            ..
        }))
    ));
    for (words, precedence) in [
        (&[0, 3, 0, END][..], 5),
        (&[0, 3, 0][..], 3),
        (&[0, 3, 0, END, END][..], 3),
    ] {
        assert!(matches!(
            LocalTokenGroup::new(&encode(words), precedence, None),
            Err(GrammarError::InvalidLocalTokenPrecedence { .. })
        ));
    }

    let fixture = fixture();
    let table = table(&fixture, "mixed");
    let load = |words: &[u16], precedence, fallback| {
        let group = LocalTokenGroup::new(&encode(words), precedence, fallback).unwrap();
        Parser::deserialize_with_hooks(
            spec(
                table,
                &[
                    Tokenizer::Internal(2),
                    Tokenizer::Local(group.clone()),
                    Tokenizer::Local(group),
                ],
            ),
            &[],
        )
        .unwrap_err()
    };
    // The DFA cannot accept from its precedence trailer or jump to it.
    assert_eq!(
        load(&[1, 5, 0, 2, END], 3, None),
        GrammarError::InvalidLocalTokenData { offset: 0 }
    );
    assert_eq!(
        load(&[1, 3, 1, 1, 2, 6, END], 6, None),
        GrammarError::InvalidLocalTokenData { offset: 0 }
    );
    // Error, repeat, EOF, and out-of-range terms cannot be tokens.
    for term in [0, 14, 17, 22] {
        assert_eq!(
            load(&[1, 5, 0, term, 1, END], 5, None),
            GrammarError::InvalidLocalTokenData { offset: 0 }
        );
        assert_eq!(
            load(&[0, 3, 0, END], 3, Some(term)),
            GrammarError::InvalidMetadata("local elseToken")
        );
        assert_eq!(
            load(&[0, 3, 0, term, END], 3, None),
            GrammarError::InvalidLocalTokenPrecedence { offset: 3 }
        );
    }
}

#[test]
fn bounds_repeated_probes_and_eof_cycles() {
    let fixture = fixture();
    let overlap = parser(table(&fixture, "overlapping"));
    assert_eq!(
        overlap.parse(&"a".repeat(2048)).unwrap_err(),
        ParseError::LimitExceeded { offset: 0 }
    );

    let end = u16::MAX;
    let cycle = LocalTokenGroup::new(&encode(&[1, 3, 1, end, end, 0, end]), 6, None).unwrap();
    let cyclic = Parser::deserialize_with_hooks(
        spec(table(&fixture, "no_else"), &[Tokenizer::Local(cycle)]),
        &[],
    )
    .unwrap();
    assert_eq!(
        cyclic.parse("").unwrap_err(),
        ParseError::LimitExceeded { offset: 0 }
    );
}
