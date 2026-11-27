//! Reference vectors from `@lezer/lr` 1.4.13, `@lezer/common` 1.5.7, and
//! `@lezer/generator` 1.8.1.
//!
//! The JSON arrays are from the published `@lezer/json` 1.0.3 `dist/index.js`.
//! See `../../docs/licenses/codemirror-lang-json.txt` from the crate root.
//! Expected values were recorded from the pinned JS runtime, not the Rust decoder.

use std::sync::{
    Arc, Mutex,
    atomic::{AtomicUsize, Ordering},
};

use gpui_editor_codemirror::language::lezer::{
    Action, ActionKind, ContextTracker, DecodeError, ExternalOptions, ExternalToken,
    ExternalTokenizer, GrammarError, GrammarSpec, ParseError, Parser, ReuseStats,
    SERIALIZED_VERSION, SerializedTables, Specialization, Specializer, StackView, SyntaxTree,
    TableError, Tables, TokenInput, Tokenizer, TreeFragment,
};

const JSON: SerializedTables<'_> = SerializedTables {
    version: 14,
    states: "$bOVQPOOOOQO'#Cb'#CbOnQPO'#CeOvQPO'#ClOOQO'#Cr'#CrQOQPOOOOQO'#Cg'#CgO}QPO'#CfO!SQPO'#CtOOQO,59P,59PO![QPO,59PO!aQPO'#CuOOQO,59W,59WO!iQPO,59WOVQPO,59QOqQPO'#CmO!nQPO,59`OOQO1G.k1G.kOVQPO'#CnO!vQPO,59aOOQO1G.r1G.rOOQO1G.l1G.lOOQO,59X,59XOOQO-E6k-E6kOOQO,59Y,59YOOQO-E6l-E6l",
    state_data: "#O~OeOS~OQSORSOSSOTSOWQO_ROgPO~OVXOgUO~O^[O~PVO[^O~O]_OVhX~OVaO~O]bO^iX~O^dO~O]_OVha~O]bO^ia~O",
    goto: "!kjPPPPPPkPPkqwPPPPk{!RPPP!XP!e!hXSOR^bQWQRf_TVQ_Q`WRg`QcZRicQTOQZRQe^RhbRYQR]R",
    token_data: "(|~RaXY!WYZ!W]^!Wpq!Wrs!]|}$u}!O$z!Q!R%T!R![&c![!]&t!}#O&y#P#Q'O#Y#Z'T#b#c'r#h#i(Z#o#p(r#q#r(w~!]Oe~~!`Wpq!]qr!]rs!xs#O!]#O#P!}#P;'S!];'S;=`$o<%lO!]~!}Og~~#QXrs!]!P!Q!]#O#P!]#U#V!]#Y#Z!]#b#c!]#f#g!]#h#i!]#i#j#m~#pR!Q![#y!c!i#y#T#Z#y~#|R!Q![$V!c!i$V#T#Z$V~$YR!Q![$c!c!i$c#T#Z$c~$fR!Q![!]!c!i!]#T#Z!]~$rP;=`<%l!]~$zO]~~$}Q!Q!R%T!R![&c~%YRT~!O!P%c!g!h%w#X#Y%w~%fP!Q![%i~%nRT~!Q![%i!g!h%w#X#Y%w~%zR{|&T}!O&T!Q![&Z~&WP!Q![&Z~&`PT~!Q![&Z~&hST~!O!P%c!Q![&c!g!h%w#X#Y%w~&yO[~~'OO_~~'TO^~~'WP#T#U'Z~'^P#`#a'a~'dP#g#h'g~'jP#X#Y'm~'rOR~~'uP#i#j'x~'{P#`#a(O~(RP#`#a(U~(ZOS~~(^P#f#g(a~(dP#i#j(g~(jP#X#Y(m~(rOQ~~(wOW~~(|OV~",
};

#[test]
fn decodes_all_words_of_pinned_json_tables() {
    assert_eq!(SERIALIZED_VERSION, JSON.version);
    let tables = Tables::deserialize(JSON).unwrap();
    assert_eq!(tables.state_count(), 26);
    assert_eq!(
        tables.states(),
        &[
            0, 7, 2, 1, 0, 0, 0, 0, 2, 0, 589830, 589830, 0, 30, 2, 1, 0, 589833, 0, 38, 2, 1, 0,
            589840, 0, 0, 2, 0, 589846, 589846, 2, 0, 2, 1, 0, 0, 0, 0, 2, 0, 589835, 589835, 0,
            45, 2, 1, 0, 589834, 0, 50, 2, 1, 0, 589848, 0, 0, 2, 0, 1114121, 1114121, 0, 58, 2, 1,
            0, 1114121, 0, 63, 2, 1, 0, 589849, 0, 0, 2, 0, 1114128, 1114128, 0, 71, 2, 1, 0,
            1114128, 0, 7, 2, 1, 0, 1114122, 0, 33, 2, 1, 0, 589841, 0, 76, 2, 1, 0, 1114136, 0, 0,
            2, 0, 1638409, 1638409, 0, 7, 2, 1, 0, 589842, 0, 84, 2, 1, 0, 1114137, 0, 0, 2, 0,
            1638416, 1638416, 0, 0, 2, 0, 1638410, 1638410, 0, 0, 2, 0, 1114129, 1114129, 0, 0, 2,
            0, 1245201, 1245201, 0, 0, 2, 0, 1114130, 1114130, 0, 0, 2, 0, 1245202, 1245202,
        ]
    );
    assert_eq!(
        tables.state_data(),
        &[
            65535, 0, 21, 0, 4, 65535, 0, 2, 4, 0, 3, 4, 0, 4, 4, 0, 5, 4, 0, 8, 2, 0, 15, 3, 0,
            23, 1, 0, 65535, 0, 7, 9, 0, 23, 6, 0, 65535, 0, 14, 12, 0, 65535, 1, 7, 0, 12, 14, 0,
            65535, 0, 13, 15, 0, 7, 24, 9, 65535, 0, 7, 17, 0, 65535, 0, 13, 18, 0, 14, 25, 9,
            65535, 0, 14, 20, 0, 65535, 0, 13, 15, 0, 7, 24, 17, 65535, 0, 13, 18, 0, 14, 25, 17,
            65535, 0,
        ]
    );
    assert_eq!(
        tables.goto_data(),
        &[
            26, 1, 1, 1, 1, 1, 1, 27, 1, 1, 27, 33, 39, 1, 1, 1, 1, 27, 43, 49, 1, 1, 1, 55, 1, 67,
            70, 9, 4, 0, 3, 14, 18, 2, 8, 2, 3, 22, 15, 5, 7, 2, 15, 2, 16, 8, 3, 23, 16, 2, 19,
            11, 3, 25, 19, 2, 5, 0, 2, 11, 3, 2, 21, 14, 3, 24, 18, 3, 10, 2, 3, 13, 3,
        ]
    );
    assert_eq!(
        tables.token_data(),
        &[
            65535, 3, 17, 9, 10, 54, 10, 11, 54, 13, 14, 54, 32, 33, 54, 34, 35, 59, 44, 45, 175,
            45, 46, 180, 48, 49, 189, 49, 58, 249, 58, 59, 266, 91, 92, 271, 93, 94, 276, 102, 103,
            281, 110, 111, 310, 116, 117, 333, 123, 124, 356, 125, 126, 361, 65535, 59, 0, 21,
            65535, 65535, 62, 8, 32, 33, 59, 33, 34, 59, 34, 35, 86, 35, 92, 59, 92, 93, 91, 93,
            55296, 59, 55296, 56320, 169, 57344, 0, 59, 65535, 91, 0, 23, 65535, 65535, 94, 9, 34,
            35, 59, 47, 48, 59, 92, 93, 59, 98, 99, 59, 102, 103, 59, 110, 111, 59, 114, 115, 59,
            116, 117, 59, 117, 118, 121, 65535, 124, 3, 48, 58, 133, 65, 71, 133, 97, 103, 133,
            65535, 136, 3, 48, 58, 145, 65, 71, 145, 97, 103, 145, 65535, 148, 3, 48, 58, 157, 65,
            71, 157, 97, 103, 157, 65535, 160, 3, 48, 58, 59, 65, 71, 59, 97, 103, 59, 65535, 172,
            1, 56320, 57344, 59, 65535, 180, 0, 13, 65535, 65535, 183, 2, 48, 49, 189, 49, 58, 249,
            65535, 194, 3, 5, 65535, 46, 47, 203, 69, 70, 223, 101, 102, 223, 65535, 206, 1, 48,
            58, 209, 65535, 214, 3, 5, 65535, 48, 58, 209, 69, 70, 223, 101, 102, 223, 65535, 226,
            3, 43, 44, 235, 45, 46, 235, 48, 58, 241, 65535, 238, 1, 48, 58, 241, 65535, 246, 1, 5,
            65535, 48, 58, 241, 65535, 254, 4, 5, 65535, 46, 47, 203, 48, 58, 249, 69, 70, 223,
            101, 102, 223, 65535, 271, 0, 12, 65535, 65535, 276, 0, 15, 65535, 65535, 281, 0, 14,
            65535, 65535, 284, 1, 97, 98, 287, 65535, 290, 1, 108, 109, 293, 65535, 296, 1, 115,
            116, 299, 65535, 302, 1, 101, 102, 305, 65535, 310, 0, 3, 65535, 65535, 313, 1, 117,
            118, 316, 65535, 319, 1, 108, 109, 322, 65535, 325, 1, 108, 109, 328, 65535, 333, 0, 4,
            65535, 65535, 336, 1, 114, 115, 339, 65535, 342, 1, 117, 118, 345, 65535, 348, 1, 101,
            102, 351, 65535, 356, 0, 2, 65535, 65535, 361, 0, 8, 65535, 65535, 366, 0, 7, 65535,
        ]
    );
}

fn raw_actions(tables: &Tables, state: u16, term: u16) -> Vec<u32> {
    tables
        .actions_for(state, term)
        .into_iter()
        .map(Action::raw)
        .collect()
}

#[test]
fn looks_up_json_actions_and_grouped_gotos() {
    let tables = Tables::deserialize(JSON).unwrap();
    // State 3 has a local shift for `]`, then a link to state 0's list.
    for (state, term, expected) in [
        (3, 14, Some(12)),
        (3, 8, Some(2)),
        (3, 23, Some(1)),
        (0, 21, Some(262144)),
        (8, 7, Some(589848)),
        (16, 7, Some(1114136)),
        (1, 19, None),
        (5, 19, None),
    ] {
        assert_eq!(tables.has_action(state, term).map(Action::raw), expected);
        assert_eq!(
            raw_actions(&tables, state, term),
            expected.into_iter().collect::<Vec<_>>()
        );
    }
    let state = tables.state(3).unwrap();
    assert_eq!(
        (
            state.actions_offset,
            state.skip_offset,
            state.tokenizer_mask
        ),
        (38, 2, 1)
    );
    assert_eq!(state.default_reduce, None);
    assert_eq!(state.forced_reduce.unwrap().raw(), 589840);
    assert_eq!(
        tables.state(1).unwrap().default_reduce.unwrap().raw(),
        589830
    );
    assert_eq!(
        tables.state(23).unwrap().default_reduce.unwrap().kind(),
        ActionKind::Reduce {
            term: 17,
            depth: 2,
            repeat: true,
            stay: false,
        }
    );

    for (state, term, strict, loose) in [
        (2, 10, Some(8), Some(8)),
        (15, 10, Some(22), Some(22)),
        (0, 10, None, Some(22)),
        (1, 22, None, Some(24)),
        (2, 24, Some(10), Some(10)),
        (3, 24, None, Some(10)),
        (0, 0, None, Some(1)),
        (0, 26, None, None),
    ] {
        assert_eq!(tables.goto(state, term, false), strict);
        assert_eq!(tables.goto(state, term, true), loose);
    }
    assert!(tables.state(26).is_none());
    assert!(tables.actions_for(26, 8).is_empty());
    assert_eq!(tables.has_action(26, 8), None);
    assert_eq!(tables.goto(26, 10, true), None);
}

#[test]
fn replays_upstream_json_shift_reduce_trace() {
    let tables = Tables::deserialize(JSON).unwrap();
    // Captured by observing Stack.apply and default Stack.reduce calls during
    // parser.parse(r#"{"a":[true, false]}"#). None denotes a default reduction.
    let trace = [
        (0, Some(8), 2, 2),
        (2, Some(23), 6, 6),
        (6, None, 589835, 7),
        (7, Some(12), 14, 14),
        (14, Some(15), 3, 3),
        (3, Some(2), 4, 4),
        (4, None, 589846, 11),
        (11, Some(13), 18, 18),
        (18, Some(21), 262144, 18),
        (18, Some(3), 4, 4),
        (4, None, 589846, 24),
        (24, None, 1114130, 19),
        (19, Some(14), 1114137, 13),
        (13, Some(14), 20, 20),
        (20, None, 1638416, 4),
        (4, None, 589846, 21),
        (21, None, 1638410, 8),
        (8, Some(7), 589848, 10),
        (10, Some(7), 17, 17),
        (17, None, 1638409, 4),
        (4, None, 589846, 5),
    ];
    let mut stack = vec![0];
    for (state, term, raw, next) in trace {
        assert_eq!(stack.last(), Some(&state));
        let entry = tables.state(state).unwrap();
        let action = match term {
            Some(term) => {
                assert_eq!(entry.default_reduce, None);
                let actions = tables.actions_for(state, term);
                assert_eq!(actions.len(), 1);
                actions[0]
            }
            None => entry.default_reduce.unwrap(),
        };
        assert_eq!(action.raw(), raw);
        match action.kind() {
            ActionKind::Shift(state) => stack.push(state),
            ActionKind::Skip => {}
            ActionKind::Reduce {
                term,
                depth,
                stay: false,
                ..
            } => {
                stack.truncate(stack.len() - usize::from(depth));
                let base = *stack.last().unwrap();
                let target = tables.goto(base, term, true).unwrap();
                assert_eq!(tables.goto(base, term, false), Some(target));
                stack.push(target);
            }
            action => panic!("unexpected JSON action: {action:?}"),
        }
        assert_eq!(stack.last(), Some(&next));
    }
    assert!(tables.state(5).unwrap().is_accepting());
}

// buildParserFile output from @lezer/generator 1.8.1 for this grammar:
// @top Program { Part* }
// Part { Name "/" }
// @skip { Comment | space }
// @skip {} { Comment { "#" (Name | "/" | space)* "!"? } }
// @tokens { Name { $[a-z]+ } space { $[ \t\n]+ } }
const SKIP: SerializedTables<'_> = SerializedTables {
    version: 14,
    states: "!vQYQPOOP_OPOOOdQPO'#C`OOQO'#Cb'#CbQYQPOOPiOPO'#C]POOO)C>Y)C>YOOQO,58z,58zOOQO-E6`-E6`POOO'#Ca'#CaPyOPO,58wPOOO,58w,58wPOOO-E6_-E6_POOO1G.c1G.c",
    state_data: "!Z~OZOSXPQ~OQQO~OXTO~OYVO~OQXOYXOZXO[ZO~QPXQXOYXOZXO[]O~QPa",
    goto: "kVPWPPZ_eRUPTROSQYTR[YQSORWS",
    token_data: "!_~RVXYhYZhpqhqrvst{!P!Q!Q#T#o!V~mRZ~XYhYZhpqh~{O[~~!QOX~~!VOY~~![PQ~#T#o!V",
};

#[test]
fn looks_up_skip_rules_other_actions_and_stay_reduction() {
    let tables = Tables::deserialize(SKIP).unwrap();
    assert!(tables.state(0).unwrap().is_accepting());
    assert!(!tables.state(0).unwrap().is_skipped());
    assert!(tables.state(5).unwrap().is_skipped());
    for (state, term, raw, kind) in [
        (0, 9, 131073, ActionKind::Goto(1)),
        (0, 11, 262144, ActionKind::Skip),
        (5, 2, 9, ActionKind::Shift(9)),
        (
            5,
            9,
            589825,
            ActionKind::Reduce {
                term: 1,
                depth: 1,
                repeat: false,
                stay: false,
            },
        ),
    ] {
        let action = tables.has_action(state, term).unwrap();
        assert_eq!((action.raw(), action.kind()), (raw, kind));
        assert_eq!(tables.actions_for(state, term), [action]);
    }
    assert_eq!(
        tables.state(6).unwrap().default_reduce.unwrap().kind(),
        ActionKind::Reduce {
            term: 8,
            depth: 1,
            repeat: false,
            stay: true,
        }
    );
    assert_eq!(tables.goto(1, 1, false), Some(6));
    assert_eq!(tables.goto(0, 1, false), None);
}

// @lezer/generator 1.8.1 output for `@top Main { }`.
const EMPTY: SerializedTables<'_> = SerializedTables {
    version: 14,
    states: "UQOOOOO",
    state_data: "Q~O",
    goto: "QPP",
    token_data: "RORO",
};

const JSON_GRAMMAR: GrammarSpec<'_> = GrammarSpec {
    tables: JSON,
    node_names: "⚠ JsonText True False Null Number String } { Object Property PropertyName : , ] [ Array",
    max_term: 25,
    repeat_node_count: 2,
    tokenizers: &[0],
    top_rule: [0, 1],
    token_prec: 0,
};

const EMPTY_GRAMMAR: GrammarSpec<'_> = GrammarSpec {
    tables: EMPTY,
    node_names: "⚠ Main",
    max_term: 2,
    repeat_node_count: 0,
    tokenizers: &[],
    top_rule: [0, 1],
    token_prec: 0,
};

#[test]
fn supports_a_generated_grammar_without_goto_groups() {
    let tables = Tables::deserialize(EMPTY).unwrap();
    assert_eq!(tables.goto_data(), [1, 1]);
    assert_eq!(tables.goto(0, 0, false), None);
    assert_eq!(tables.goto(0, 0, true), None);
    assert!(tables.state(0).unwrap().is_accepting());
    let parser = Parser::deserialize(EMPTY_GRAMMAR).unwrap();
    assert_eq!(
        tree_records(&parser.parse("").unwrap()),
        [(0, "Main", 0, 0)]
    );
    assert_eq!(
        parser.parse("x").unwrap_err(),
        ParseError::UnexpectedToken {
            offset: 0,
            state: 0,
            term: None
        }
    );
}

#[test]
fn distinguishes_probes_from_token_candidates_and_preserves_action_order() {
    // Hand-built v14 rows, checked via LRParser.hasAction and TokenCache.addActions.
    // The main list contains an error entry and repeated shifts on term 5.
    // The skip list repeats one shift, adds a skip, then ends in Other.
    let tables = Tables::deserialize(SerializedTables {
        states: "hOO^5H,(AZOOO[[OOOO[[OOOO[[OOO",
        state_data: "hOROTPOTPOTQO~OTQOTOS~QXP",
        goto: "PO",
        token_data: "O",
        ..EMPTY
    })
    .unwrap();
    assert_eq!(tables.state(0).unwrap().tokenizer_mask, u32::MAX);
    assert!(tables.token_data().is_empty());
    for (term, actions) in [(0, &[3][..]), (5, &[1, 2, 262144]), (6, &[65545])] {
        assert_eq!(tables.has_action(0, term).unwrap().raw(), 3);
        assert_eq!(raw_actions(&tables, 0, term), actions);
    }
}

#[test]
fn rejects_bad_version_and_encoding() {
    assert_eq!(
        Tables::deserialize(SerializedTables {
            version: 13,
            ..EMPTY
        })
        .unwrap_err(),
        TableError::UnsupportedVersion(13)
    );
    for (token_data, error) in [
        ("", DecodeError::TruncatedInteger { offset: 0 }),
        ("P!", DecodeError::TruncatedInteger { offset: 1 }),
        ("P\\", DecodeError::InvalidByte { offset: 1 }),
        ("P\"", DecodeError::InvalidByte { offset: 1 }),
        ("Pé", DecodeError::InvalidByte { offset: 1 }),
        ("P!~", DecodeError::InvalidByte { offset: 2 }),
        ("P?Mp", DecodeError::IntegerOverflow { offset: 1 }), // 65536 > u16::MAX
        ("P5H,(A[", DecodeError::IntegerOverflow { offset: 1 }), // u32::MAX + 1
        ("QO", DecodeError::LengthMismatch { declared: 2 }),
        ("O~", DecodeError::LengthMismatch { declared: 0 }),
        ("5H,(AZ", DecodeError::LengthMismatch { declared: u32::MAX }),
    ] {
        assert_eq!(
            Tables::deserialize(SerializedTables {
                token_data,
                ..EMPTY
            })
            .unwrap_err(),
            TableError::Encoding {
                table: "tokenData",
                error
            },
            "input {token_data:?}",
        );
    }
}

#[test]
fn rejects_bad_lr_record_structure() {
    for (spec, error) in [
        (
            SerializedTables {
                states: "PO",
                ..EMPTY
            },
            TableError::InvalidStateTable,
        ),
        (
            SerializedTables {
                states: "UOOOOPO",
                ..EMPTY
            },
            TableError::InvalidReduction { state: 0 },
        ),
        (
            SerializedTables {
                states: "UO5H,(AZOOOO",
                ..EMPTY
            },
            TableError::InvalidActionSequence {
                offset: u32::MAX as usize,
            },
        ),
        (
            SerializedTables {
                state_data: "R~QP",
                ..EMPTY
            },
            TableError::InvalidActionSequence { offset: 0 },
        ),
        (
            SerializedTables {
                state_data: "S~PPP",
                ..EMPTY
            },
            TableError::InvalidActionSequence { offset: 65537 },
        ),
        (
            SerializedTables {
                state_data: "VPOO~POO",
                ..EMPTY
            },
            TableError::InvalidActionSequence { offset: 0 },
        ),
        (
            SerializedTables {
                goto: "QQP",
                ..EMPTY
            },
            TableError::InvalidGotoTable { offset: 0 },
        ),
        (
            SerializedTables {
                goto: "QPO",
                ..EMPTY
            },
            TableError::InvalidGotoTable { offset: 0 },
        ),
        (
            SerializedTables {
                goto: "SPQRO",
                ..EMPTY
            },
            TableError::InvalidGotoTable { offset: 2 },
        ),
    ] {
        assert_eq!(Tables::deserialize(spec).unwrap_err(), error);
    }
}

type TreeRecord<'a> = (usize, &'a str, usize, usize);

fn tree_records(tree: &SyntaxTree) -> Vec<TreeRecord<'_>> {
    let mut records = Vec::new();
    let mut todo = vec![(0, tree.root())];
    while let Some((depth, node)) = todo.pop() {
        records.push((depth, node.name(), node.range().start, node.range().end));
        todo.extend(node.children().rev().map(|child| (depth + 1, child)));
    }
    records
}

#[test]
fn parses_pinned_json_with_byte_ranges_and_flattens_repeats() {
    let tree = {
        let parser = Parser::deserialize(JSON_GRAMMAR).unwrap();
        let source =
            String::from(" \n{\"é😀\":[true, false, null, -1.2E+03, \"a\\n🦀\"],\"z\":{}}\t");
        let tree = parser.parse(&source).unwrap();
        for node in tree_records(&tree) {
            assert!(source.is_char_boundary(node.2));
            assert!(source.is_char_boundary(node.3));
        }
        tree
    };
    // Strict upstream cursor traversal, with UTF-16 offsets converted to bytes.
    assert_eq!(
        tree_records(&tree),
        [
            (0, "JsonText", 0, 61),
            (1, "Object", 2, 60),
            (2, "{", 2, 3),
            (2, "Property", 3, 52),
            (3, "PropertyName", 3, 11),
            (3, ":", 11, 12),
            (3, "Array", 12, 52),
            (4, "[", 12, 13),
            (4, "True", 13, 17),
            (4, ",", 17, 18),
            (4, "False", 19, 24),
            (4, ",", 24, 25),
            (4, "Null", 26, 30),
            (4, ",", 30, 31),
            (4, "Number", 32, 40),
            (4, ",", 40, 41),
            (4, "String", 42, 51),
            (4, "]", 51, 52),
            (2, ",", 52, 53),
            (2, "Property", 53, 59),
            (3, "PropertyName", 53, 56),
            (3, ":", 56, 57),
            (3, "Object", 57, 59),
            (4, "{", 57, 58),
            (4, "}", 58, 59),
            (2, "}", 59, 60),
        ]
    );
    assert_eq!(tree.root().term(), 1);
}

#[test]
fn requires_a_complete_json_parse_without_recovery() {
    let parser = Parser::deserialize(JSON_GRAMMAR).unwrap();
    for (text, byte, terminal) in [
        ("", 0, Some(19)),
        ("  ", 2, Some(19)),
        ("[", 1, Some(19)),
        ("[true,]", 6, Some(14)),
        ("true false", 5, Some(3)),
        ("01", 1, Some(5)),
        ("1e", 1, None),
        (r#""\uD80""#, 0, None),
        ("[\"é\",💣]", 6, None),
    ] {
        assert!(
            matches!(parser.parse(text), Err(ParseError::UnexpectedToken { offset, term, .. }) if offset == byte && term == terminal),
            "input {text:?}: {:?}",
            parser.parse(text)
        );
    }
    assert!(parser.parse(r#""\uD800""#).is_ok(), "valid JSON escape");
    for text in [
        "true",
        "false",
        "null",
        "0",
        "-0",
        " 1.9e-42\r\n",
        "[]",
        "{}",
        "\"\u{ffff}\"",
    ] {
        assert_eq!(parser.parse(text).unwrap().root().range(), 0..text.len());
    }
}

#[test]
fn recovers_pinned_json_without_changing_strict_results() {
    let parser = Parser::deserialize(JSON_GRAMMAR).unwrap();
    // `@lezer/json` 1.0.3 with `@lezer/lr` 1.4.13, with cursor positions
    // converted to UTF-8 bytes. These cover insertion, deletion, forced
    // reduction, restart, and a partial tree at EOF.
    let cases: &[(&str, &[TreeRecord<'_>])] = &[
        ("", &[(0, "JsonText", 0, 0), (1, "⚠", 0, 0)]),
        (
            r#"{"é" 1}"#,
            &[
                (0, "JsonText", 0, 8),
                (1, "Object", 0, 8),
                (2, "{", 0, 1),
                (2, "Property", 1, 7),
                (3, "PropertyName", 1, 5),
                (3, "⚠", 6, 6),
                (3, "Number", 6, 7),
                (2, "}", 7, 8),
            ],
        ),
        (
            "[1 😃🙂 2]",
            &[
                (0, "JsonText", 0, 14),
                (1, "Array", 0, 14),
                (2, "[", 0, 1),
                (2, "Number", 1, 2),
                (2, "⚠", 3, 11),
                (2, "⚠", 12, 13),
                (3, "Number", 12, 13),
                (2, "]", 13, 14),
            ],
        ),
        (
            r#"{"a":"#,
            &[
                (0, "JsonText", 0, 5),
                (1, "Object", 0, 5),
                (2, "{", 0, 1),
                (2, "Property", 1, 5),
                (3, "PropertyName", 1, 4),
                (3, ":", 4, 5),
                (3, "⚠", 5, 5),
                (2, "⚠", 5, 5),
            ],
        ),
        (
            "true false",
            &[
                (0, "JsonText", 0, 10),
                (1, "True", 0, 4),
                (1, "⚠", 5, 5),
                (1, "False", 5, 10),
            ],
        ),
    ];
    for &(source, expected) in cases {
        let strict = parser.parse(source).unwrap_err();
        assert!(matches!(strict, ParseError::UnexpectedToken { .. }));
        assert_eq!(parser.parse_glr(source).unwrap_err(), strict);
        assert_eq!(parser.parse_incremental(source, None).unwrap_err(), strict);
        assert_recovered(&parser, source, expected);
    }
}

#[test]
fn completes_nested_eof_recovery_like_pinned_lezer() {
    let fixture: serde_json::Value =
        serde_json::from_str(include_str!("fixtures/lezer_recovery_eof.json")).unwrap();
    let parser = Parser::deserialize(JSON_GRAMMAR).unwrap();
    assert_recovery_cases(&parser, &fixture["cases"]);
}

#[test]
fn keeps_repeat_groups_until_recovery_places_error_markers() {
    let fixture: serde_json::Value =
        serde_json::from_str(include_str!("fixtures/lezer_recovery_repeats.json")).unwrap();
    let json = Parser::deserialize(JSON_GRAMMAR).unwrap();
    let blocks = Parser::deserialize(RECOVERY_REPEAT_GRAMMAR).unwrap();
    assert_recovery_cases(&json, &fixture["json"]);
    assert_recovery_cases(&blocks, &fixture["blocks"]);
    let valid = "(one:two:three) (four)";
    assert_eq!(
        tree_records(&blocks.parse_recovering(valid).unwrap()),
        tree_records(&blocks.parse(valid).unwrap())
    );
}

fn assert_recovery_cases(parser: &Parser, cases: &serde_json::Value) {
    for case in cases.as_array().unwrap() {
        let source = case["source"].as_str().unwrap();
        let nodes: Vec<(usize, String, usize, usize)> =
            serde_json::from_value(case["nodes"].clone()).unwrap();
        let expected: Vec<_> = nodes
            .iter()
            .map(|(depth, name, from, to)| (*depth, name.as_str(), *from, *to))
            .collect();
        assert!(matches!(
            parser.parse(source),
            Err(ParseError::UnexpectedToken { .. })
        ));
        assert_recovered(parser, source, &expected);
    }
}

// `@lezer/generator` 1.8.1 output for the Block grammar in
// fixtures/lezer_recovery_repeats.mjs.
const RECOVERY_REPEAT_GRAMMAR: GrammarSpec<'_> = GrammarSpec {
    tables: SerializedTables {
        states: "!jQYQPOOO_QPO'#C_OOQO'#Ca'#CaQYQPOOOdQPO,58yOOQO-E6_-E6_OlQPO'#CbOqQPO1G.eOOQO1G.e1G.eOOQO,58|,58|OOQO-E6`-E6`OOQO7+$P7+$P",
        state_data: "y~OXOSPOS~OYPO~OSSO~OZUO[WO~OSXO~OZUO[ZO~O",
        goto: "hVPPPWP[bTQORQRORTRQVSRYV",
        token_data: "!p~RXXYnYZn]^npqnst!Pxy!Xyz!^![!]!c#T#o!h~sSX~XYnYZn]^npqn~!UPP~#T#o!P~!^OY~~!cO[~~!hOZ~~!mPS~#T#o!h",
        ..EMPTY
    },
    node_names: "⚠ Comment Main Block Word",
    max_term: 12,
    repeat_node_count: 2,
    tokenizers: &[0],
    top_rule: [0, 2],
    token_prec: 0,
};

// `@lezer/generator` 1.8.1 output for:
// @top Main { Part* }
// Part { "(" Word ":" Word ")" }
// @skip { space | Comment }
// @tokens { Word { $[a-z]+ } space { $[ \t]+ } Comment { "#" $[a-z]* } }
const RECOVERY_GRAMMAR: GrammarSpec<'_> = GrammarSpec {
    tables: SerializedTables {
        states: "!WQYQPOOO_QPO'#C_OOQO'#Ca'#CaQYQPOOOdQPO,58yOOQO-E6_-E6_OiQPO1G.eOnQPO7+$POOQO<<Gk<<Gk",
        state_data: "s~OWOSPOS~OXPO~OSSO~OYUO~OSVO~OZWO~O",
        goto: "aUPPPVPZTQORQRORTR",
        token_data: "!d~RVXYhpqhstsxy{yz!Q![!]!V#T#o![~mQW~XYhpqh~xPP~#T#os~!QOX~~!VOZ~~![OY~~!aPS~#T#o![",
        ..EMPTY
    },
    node_names: "⚠ Comment Main Part Word",
    max_term: 11,
    repeat_node_count: 1,
    tokenizers: &[0],
    top_rule: [0, 2],
    token_prec: 0,
};

#[test]
fn recovers_generated_grammar_and_preserves_named_skips() {
    let parser = Parser::deserialize(RECOVERY_GRAMMAR).unwrap();
    let valid = "(one:#note two)";
    assert_eq!(
        tree_records(&parser.parse_recovering(valid).unwrap()),
        tree_records(&parser.parse_glr(valid).unwrap())
    );
    // Pinned JS cursor output. The forced reduction contains the trailing
    // skip node, but deletion after a complete Part stays outside that Part.
    assert_recovered(
        &parser,
        "(one:two #trailing",
        &[
            (0, "Main", 0, 18),
            (1, "Part", 0, 18),
            (2, "Word", 1, 4),
            (2, "Word", 5, 8),
            (2, "Comment", 9, 18),
            (2, "⚠", 18, 18),
        ],
    );
    assert_recovered(
        &parser,
        "(one:two) # gap",
        &[
            (0, "Main", 0, 15),
            (1, "Part", 0, 9),
            (2, "Word", 1, 4),
            (2, "Word", 5, 8),
            (1, "Comment", 10, 11),
            (1, "⚠", 12, 15),
            (2, "Word", 12, 15),
        ],
    );
}

#[test]
fn invokes_context_hooks_for_insertions_and_reductions_but_not_deletions() {
    let shifts = Arc::new(Mutex::new(Vec::new()));
    let reduces = Arc::new(Mutex::new(Vec::new()));
    let parser = Parser::deserialize(RECOVERY_GRAMMAR)
        .unwrap()
        .with_context_tracker(
            ContextTracker::new(())
                .with_shift({
                    let shifts = Arc::clone(&shifts);
                    move |_, term, stack, input| {
                        shifts
                            .lock()
                            .unwrap()
                            .push((term, input.position(), stack.byte_offset()));
                        Ok(None)
                    }
                })
                .with_reduce({
                    let reduces = Arc::clone(&reduces);
                    move |_, term, stack, input| {
                        reduces
                            .lock()
                            .unwrap()
                            .push((term, input.position(), stack.byte_offset()));
                        Ok(None)
                    }
                }),
        );
    assert_recovered(
        &parser,
        "(one two)(three:four)",
        &[
            (0, "Main", 0, 21),
            (1, "Part", 0, 9),
            (2, "Word", 1, 4),
            (2, "⚠", 5, 5),
            (2, "Word", 5, 8),
            (1, "Part", 9, 21),
            (2, "Word", 10, 15),
            (2, "Word", 16, 20),
        ],
    );
    // Term 10 is `:`. Its synthetic shift does not consume input.
    assert!(shifts.lock().unwrap().contains(&(10, 5, 5)));
    shifts.lock().unwrap().clear();
    reduces.lock().unwrap().clear();
    parser.parse_recovering("(one::two").unwrap();
    assert!(
        !shifts
            .lock()
            .unwrap()
            .iter()
            .any(|&(term, start, end)| term == 10 && (start, end) == (5, 6))
    );
    assert!(reduces.lock().unwrap().contains(&(3, 0, 9)));
}

fn assert_recovered(parser: &Parser, source: &str, expected: &[TreeRecord<'_>]) {
    let tree = parser.parse_recovering(source).unwrap();
    assert_eq!(tree_records(&tree), expected, "source {source:?}");
    assert_eq!(tree.root().range(), 0..source.len());
    let mut todo = vec![tree.root()];
    while let Some(node) = todo.pop() {
        let range = node.range();
        assert!(
            source.get(range.clone()).is_some(),
            "invalid range {range:?}"
        );
        assert_eq!(node.is_error(), node.name() == "⚠");
        let mut previous = range.start;
        for child in node.children() {
            let child_range = child.range();
            assert!(child_range.start >= previous && child_range.end <= range.end);
            previous = child_range.end;
            todo.push(child);
        }
    }
}

#[test]
fn parses_long_lists_and_deep_trees_iteratively() {
    let parser = Parser::deserialize(JSON_GRAMMAR).unwrap();
    let list = format!("[{}null]", "null,".repeat(4096));
    let tree = parser.parse(&list).unwrap();
    let array = tree.root().children().next().unwrap();
    assert_eq!(array.children().len(), 8195);
    let nested = format!("{}0{}", "[".repeat(4096), "]".repeat(4096));
    let tree = parser.parse(&nested).unwrap();
    let mut node = tree.root().children().next().unwrap();
    for _ in 0..4096 {
        assert_eq!(node.name(), "Array");
        node = node.children().nth(1).unwrap();
    }
    assert_eq!(node.name(), "Number");
}

fn skip_parser() -> Parser {
    Parser::deserialize(GrammarSpec {
        tables: SKIP,
        node_names: "⚠ Comment Name Program Part",
        max_term: 12,
        repeat_node_count: 2,
        top_rule: [0, 3],
        ..JSON_GRAMMAR
    })
    .unwrap()
}

#[test]
fn parses_separate_skip_rules_and_preserves_named_comments() {
    let tree = skip_parser()
        .parse("a/ # comment / spaces! b/ #tail")
        .unwrap();
    assert_eq!(
        tree_records(&tree),
        [
            (0, "Program", 0, 31),
            (1, "Part", 0, 2),
            (2, "Name", 0, 1),
            (1, "Comment", 3, 22),
            (2, "Name", 5, 12),
            (2, "Name", 15, 21),
            (1, "Part", 23, 25),
            (2, "Name", 23, 24),
            (1, "Comment", 26, 31),
            (2, "Name", 27, 31),
        ]
    );
}

#[test]
fn places_skipped_nodes_inside_or_outside_reductions_by_range() {
    // @top Main { Pair Pair }
    // Pair { "a" Word "b" }
    // @skip { Comment }
    // @skip {} { Comment { "#" CommentWord } }
    // @tokens { Word { "w" } CommentWord { "q"+ } }
    let parser = Parser::deserialize(GrammarSpec {
        tables: SerializedTables {
            version: 14,
            states: "!^OVQPOOP[OPOOOaQPO'#C`OVQPOOPfOPO'#C]POOO)C>X)C>XOkQPO,58zQOQPOOPOOO,58w,58wOOQO1G.f1G.f",
            state_data: "p~OWPQ~OXQO~OWSO~OTUO~OQWO~OYXO~O",
            goto: "_TPUPPXRTPQRORVR",
            token_data: "!O~RTstb#T#Ug#U#Vl#e#fq#k#ly~gOW~~lOX~~qOY~~vPQ~#e#fq~!OOT~",
        },
        node_names: "⚠ Comment CommentWord Main Pair Word",
        max_term: 10,
        top_rule: [0, 3],
        tokenizers: &[0],
        ..EMPTY_GRAMMAR
    }).unwrap();
    let tree = parser.parse("#qa#qwb#qawb#q").unwrap();
    assert_eq!(
        tree_records(&tree),
        [
            (0, "Main", 0, 14),
            (1, "Comment", 0, 2),
            (2, "CommentWord", 1, 2),
            (1, "Pair", 2, 7),
            (2, "Comment", 3, 5),
            (3, "CommentWord", 4, 5),
            (2, "Word", 5, 6),
            (1, "Comment", 7, 9),
            (2, "CommentWord", 8, 9),
            (1, "Pair", 9, 12),
            (2, "Word", 10, 11),
            (1, "Comment", 12, 14),
            (2, "CommentWord", 13, 14),
        ]
    );
}

#[test]
fn honors_precedence_over_longest_match_and_uses_state_groups() {
    // @top Main { Whole | Prefix Suffix }
    // @tokens { Prefix { "a" } Whole { "ab" } Suffix { "b" } @precedence { Prefix, Whole } }
    // This fixture adds a second Prefix to the generated precedence list.
    // Published tables such as @lezer/html 1.3.13 also repeat terms.
    let parser = Parser::deserialize(GrammarSpec {
        tables: SerializedTables {
            states: "bOQOPOOQOOOOOOYOPOO",
            state_data: "c~OQPORQO~OSPO~ORQR~",
            token_data: "k~RQ#T#UX#U#Vf~^PR~#U#Va~fOQ~~kOS~",
            ..EMPTY
        },
        node_names: "⚠ Main Whole Prefix Suffix",
        max_term: 5,
        tokenizers: &[0],
        token_prec: 15,
        ..EMPTY_GRAMMAR
    })
    .unwrap();
    assert_eq!(
        tree_records(&parser.parse("ab").unwrap()),
        [(0, "Main", 0, 2), (1, "Prefix", 0, 1), (1, "Suffix", 1, 2)]
    );

    // @top Main { Token | "#" Pair }
    // @tokens { Token { $[a-z]+ } Pair { $[a-z] $[a-z] } }
    let parser = Parser::deserialize(GrammarSpec {
        tables: SerializedTables {
            states: "bOQOPOOQOOOOOOYOQOO",
            state_data: "_~OQPOTQO~ORPO~O",
            token_data: "x~RQstX#T#o^~^OT~RcPQP#T#ofRmPRQQP#T#opPuPQP#T#op",
            ..EMPTY
        },
        node_names: "⚠ Main Token Pair",
        max_term: 5,
        tokenizers: &[0, 1],
        ..EMPTY_GRAMMAR
    })
    .unwrap();
    assert_eq!(
        tree_records(&parser.parse("ab").unwrap()),
        [(0, "Main", 0, 2), (1, "Token", 0, 2)]
    );
    assert_eq!(
        tree_records(&parser.parse("#ab").unwrap()),
        [(0, "Main", 0, 3), (1, "Pair", 1, 3)]
    );
    assert!(parser.parse("#abc").is_err());
}

#[test]
fn parses_zero_width_tokens_and_epsilon_rules() {
    // @top Main { Token End }
    // @tokens { Token { "x" } End { @eof } }
    let parser = Parser::deserialize(GrammarSpec {
        tables: SerializedTables {
            states: "bOQOPOOOVOPOOQOOOOO",
            state_data: "[~OQPO~ORQO~O",
            token_data: "c~RQ#l#mX~~^~^OQ~~cOR~",
            ..EMPTY
        },
        node_names: "⚠ Main Token End",
        max_term: 4,
        tokenizers: &[0],
        ..EMPTY_GRAMMAR
    })
    .unwrap();
    assert_eq!(
        tree_records(&parser.parse("x").unwrap()),
        [(0, "Main", 0, 1), (1, "Token", 0, 1), (1, "End", 1, 1)]
    );
    assert!(parser.parse("xx").is_err());

    // @top Main { A "x" }
    // A { }
    let parser = Parser::deserialize(GrammarSpec {
        tables: SerializedTables {
            states: "bOOOO?MrOOQOPOOQOOOOO",
            state_data: "V~OSQO~O",
            goto: "VRPPSRPO",
            token_data: "Z~RP#l#mU~ZOS~",
            ..EMPTY
        },
        node_names: "⚠ Main A",
        max_term: 4,
        tokenizers: &[0],
        ..EMPTY_GRAMMAR
    })
    .unwrap();
    assert_eq!(
        tree_records(&parser.parse("x").unwrap()),
        [(0, "Main", 0, 1), (1, "A", 0, 0)]
    );
}

#[test]
fn splits_an_eof_reduce_reduce_conflict_without_changing_parse() {
    // @top Main { A | B }
    // A { "x" ~same }
    // B { "x" ~same }
    let parser = Parser::deserialize(GrammarSpec {
        tables: SerializedTables {
            states: "bOQOPOOOVOPO'#C^QOOOOO",
            state_data: "_~OTPO~OSQXSRX~O",
            goto: "WSPPTTRQO",
            token_data: "Z~RP#l#mU~ZOT~",
            ..EMPTY
        },
        node_names: "⚠ Main A B",
        max_term: 5,
        tokenizers: &[0],
        ..EMPTY_GRAMMAR
    })
    .unwrap();
    assert_eq!(
        parser.parse("x").unwrap_err(),
        ParseError::Ambiguous {
            offset: 1,
            state: 1,
            term: 4
        }
    );
    assert_eq!(
        tree_records(&parser.parse_glr("x").unwrap()),
        [(0, "Main", 0, 1), (1, "B", 0, 1)]
    );
    // The same generated tables with B[@dynamicPrecedence=-2].
    let parser = parser.with_dynamic_precedence(&[(3, -2)]).unwrap();
    assert_eq!(
        tree_records(&parser.parse_glr("x").unwrap()),
        [(0, "Main", 0, 1), (1, "A", 0, 1)]
    );
}

#[test]
fn follows_both_shift_reduce_paths_until_one_accepts() {
    // @top Main { A "y" "a" | B "b" }
    // A { "x" ~shared }
    // B { "x" ~shared "y" }
    let parser = Parser::deserialize(GrammarSpec {
        tables: SerializedTables {
            states: "zOQOPOOOVOPO'#C^O_OPOOOdOPOOOOOO,58y,58yOiOPOOQOOOOO",
            state_data: "n~OTPO~OUSOUQX~OUTO~OWUO~OVUO~O",
            goto: "ZSPPTWRQORRO",
            token_data: "s~RS#T#U_#U#Vd#l#mi#m#nn~dOV~~iOW~~nOT~~sOU~",
            ..EMPTY
        },
        node_names: "⚠ Main A B",
        max_term: 8,
        tokenizers: &[0],
        ..EMPTY_GRAMMAR
    })
    .unwrap();
    for (text, name, end) in [("xya", "A", 1), ("xyb", "B", 2)] {
        assert!(matches!(
            parser.parse(text),
            Err(ParseError::Ambiguous { offset: 1, .. })
        ));
        assert_eq!(
            tree_records(&parser.parse_glr(text).unwrap()),
            [(0, "Main", 0, 3), (1, name, 0, end)]
        );
    }
    assert!(matches!(
        parser.parse_glr("xyc"),
        Err(ParseError::UnexpectedToken { offset: 2, .. })
    ));
}

#[test]
fn preserves_output_across_a_delayed_reduce_reduce_fork_and_skip_rules() {
    // @top Main { A "y" "a" | B "y" "b" }
    // A { Atom ~same }
    // B { Atom ~same }
    // @skip { Comment | space }
    // @skip {} { Comment { "#" "!" } }
    // @tokens { Atom { "é" } space { " " } }
    let parser = Parser::deserialize(GrammarSpec {
        tables: SerializedTables {
            states: "!dOYQPOOP_OPOOOdQPO'#C_OlQPOOOqQPOOPvOPO'#C]POOO)C>X)C>XO{QPOOO!QQPOOPOOO,58w,58wQOQPOO",
            state_data: "!V~OYOSWPQ~OSQO~OWTO~OZRXZTX~OZVO~OZWO~OXXO~O[YO~O]YO~O",
            goto: "`UPVPYP]RUPRRORSO",
            token_data: "!]~RVpqhqrmstr#T#Uw#U#V|#m#n!R&R&S!W~mOY~~rOX~~wOW~~|O[~~!RO]~~!WOZ~~!]OS~",
            ..EMPTY
        },
        node_names: "⚠ Comment Main A Atom B",
        max_term: 13,
        top_rule: [0, 2],
        tokenizers: &[0],
        ..EMPTY_GRAMMAR
    }).unwrap();
    for (text, name) in [("éy #! a", "A"), ("éy #! b", "B")] {
        assert_eq!(
            tree_records(&parser.parse_glr(text).unwrap()),
            [
                (0, "Main", 0, 8),
                (1, name, 0, 2),
                (2, "Atom", 0, 2),
                (1, "Comment", 4, 6)
            ]
        );
    }
    assert!(matches!(
        parser.parse_glr("éy #! c"),
        Err(ParseError::UnexpectedToken { offset: 7, .. })
    ));
}

#[test]
fn ranks_converging_branches_by_dynamic_precedence() {
    // @top Main { (A | B) "y" }
    // A { "x" ~same }
    // B[@dynamicPrecedence=2] { "x" ~same }
    let spec = GrammarSpec {
        tables: SerializedTables {
            states: "hOQOPOOOVOPO'#C^O_OPOOQOOOOO",
            state_data: "d~OTPO~OUQXURX~OURO~O",
            goto: "WSPPTTRQO",
            token_data: "c~RQ#l#mX#m#n^~^OT~~cOU~",
            ..EMPTY
        },
        node_names: "⚠ Main A B",
        max_term: 6,
        tokenizers: &[0],
        ..EMPTY_GRAMMAR
    };
    // Removing the property does not change the generated tables.
    let parser = Parser::deserialize(spec).unwrap();
    assert_eq!(
        tree_records(&parser.parse_glr("xy").unwrap()),
        [(0, "Main", 0, 2), (1, "A", 0, 1)]
    );
    let parser = parser.with_dynamic_precedence(&[(3, 2)]).unwrap();
    assert_eq!(
        tree_records(&parser.parse_glr("xy").unwrap()),
        [(0, "Main", 0, 2), (1, "B", 0, 1)]
    );
    for invalid in [
        &[(0, 2)][..],
        &[(4, 2)],
        &[(7, 2)],
        &[(3, 11)],
        &[(3, -11)],
        &[(3, 0), (3, 2)],
    ] {
        assert_eq!(
            Parser::deserialize(spec)
                .unwrap()
                .with_dynamic_precedence(invalid)
                .unwrap_err(),
            GrammarError::InvalidMetadata("dynamicPrecedences")
        );
    }
}

#[test]
fn merges_equivalent_branches_before_the_next_fork() {
    // @top Main { ((A | B) "y")+ }
    // A { "x" ~same }
    // B { "x" ~same }
    let parser = Parser::deserialize(GrammarSpec {
        tables: SerializedTables {
            states: "tOQOPOOOVOPO'#C^O_OPO'#C`QQOPOOOOOO,58z,58zOOOO-E6^-E6^",
            state_data: "d~OUPO~OVQXVRX~OVSO~O",
            goto: "`TPPUUYTQORQRORTR",
            token_data: "c~RQ#l#mX#m#n^~^OU~~cOV~",
            ..EMPTY
        },
        node_names: "⚠ Main A B",
        max_term: 7,
        repeat_node_count: 1,
        tokenizers: &[0],
        ..EMPTY_GRAMMAR
    })
    .unwrap();
    assert_eq!(
        tree_records(&parser.parse_glr("xyxyxyxyxy").unwrap()),
        [
            (0, "Main", 0, 10),
            (1, "A", 0, 1),
            (1, "B", 2, 3),
            (1, "B", 4, 5),
            (1, "B", 6, 7),
            (1, "B", 8, 9)
        ]
    );
    let source = "xy".repeat(256);
    let tree = parser.parse_glr(&source).unwrap();
    assert_eq!(tree.root().children().len(), 256);
    // Both forks allocate a new context, but the resulting values are equal.
    let parser = parser.with_context_tracker(
        ContextTracker::new(0_u32).with_reduce(|count, _, _, _| Ok(Some(count + 1))),
    );
    assert_eq!(
        tree_records(&parser.parse_glr(&source).unwrap()),
        tree_records(&tree)
    );
}

#[test]
fn splits_extended_external_tokens_with_different_end_offsets() {
    // @top Main { (Short "b" "c" | Long "c") "d" }
    // @external tokens short from "tokens" { Short }
    // @external tokens long from "tokens" { Long }
    let external = |term, text: &'static [u8], extend| {
        Tokenizer::External(ExternalTokenizer::new(
            ExternalOptions {
                extend,
                ..Default::default()
            },
            move |input, _| {
                for (offset, &byte) in text.iter().enumerate() {
                    if input.peek(offset as isize)? != Some(u16::from(byte)) {
                        return Ok(None);
                    }
                }
                input.advance(text.len())?;
                Ok(Some(input.token(term)))
            },
        ))
    };
    let parser = Parser::deserialize_with_hooks(
        GrammarSpec {
            tables: SerializedTables {
                states: "nOQOUOOOYOPOOO_OPOOOdOPOOQOOOOO",
                state_data: "i~OPPOQQO~OTQO~OURO~OVSO~O",
                token_data: "k~RR#U#V[#V#Wa#W#Xf~aOT~~fOU~~kOV~",
                ..EMPTY
            },
            node_names: "⚠ Short Long Main",
            max_term: 7,
            repeat_node_count: 0,
            tokenizers: &[
                Tokenizer::Internal(0),
                external(1, b"a", true),
                external(2, b"ab", false),
            ],
            top_rule: [0, 3],
            token_prec: 0,
        },
        &[],
    )
    .unwrap();
    assert_eq!(
        tree_records(&parser.parse_glr("abcd").unwrap()),
        [(0, "Main", 0, 4), (1, "Short", 0, 1)]
    );
}

// @top Main { Head (First | Second | Word) End }
// @external tokens high from "tokens" { First, Miss }
// @external tokens low from "tokens" { Second }
// @tokens { Head { ">" } Word { "x" } End { "!" } }
fn priority_spec(tokenizers: &[Tokenizer]) -> GrammarSpec<'_, Tokenizer> {
    GrammarSpec {
        tables: SerializedTables {
            states: "hOQOSOOOVOVOOObOSOOQOOOOO",
            state_data: "g~OTPO~OPQORQOUQO~OVRO~O",
            token_data: "k~RRqr[!`!aa#l#mf~aOV~~fOT~~kOU~",
            ..EMPTY
        },
        node_names: "⚠ First Miss Second Main Head Word End",
        max_term: 8,
        repeat_node_count: 0,
        tokenizers,
        top_rule: [0, 4],
        token_prec: 0,
    }
}

#[test]
fn selects_external_and_internal_tokens_in_pinned_priority_order() {
    for (high, fallback, extend, name, calls) in [
        (Some(1), true, false, Some("First"), &[0][..]),
        (Some(2), true, false, Some("Second"), &[0, 1]),
        (Some(2), false, false, None, &[0]),
        (None, false, false, Some("Word"), &[0, 1]),
        // These two tokens have the same shift. Pinned getActions keeps First.
        (Some(1), false, true, Some("First"), &[0, 1]),
    ] {
        let observed = Arc::new(Mutex::new(Vec::new()));
        let slot = |id, options, term: Option<u16>| {
            let observed = Arc::clone(&observed);
            Tokenizer::External(ExternalTokenizer::new(options, move |input, stack| {
                // The other states use only slot 2, including the accepting state.
                assert_eq!(
                    (stack.state(), stack.byte_offset(), input.position()),
                    (1, 1, 1)
                );
                assert_eq!(input.peek(-1)?, Some(u16::from(b'>')));
                assert_eq!(input.peek(0)?, Some(u16::from(b'x')));
                assert_eq!(input.peek(1)?, Some(u16::from(b'!')));
                assert_eq!(input.peek(isize::MIN)?, None);
                observed.lock().unwrap().push(id);
                Ok(term.map(|term| ExternalToken {
                    term,
                    end: input.position() + 1,
                }))
            }))
        };
        let tokenizers = [
            slot(
                0,
                ExternalOptions {
                    extend,
                    ..Default::default()
                },
                high,
            ),
            slot(
                1,
                ExternalOptions {
                    fallback,
                    ..Default::default()
                },
                high.map(|_| 3),
            ),
            Tokenizer::Internal(0),
        ];
        let parser = Parser::deserialize_with_hooks(priority_spec(&tokenizers), &[]).unwrap();
        let result = parser.parse(">x!");
        assert_eq!(*observed.lock().unwrap(), calls);
        if let Some(name) = name {
            assert_eq!(
                tree_records(&result.unwrap()),
                [
                    (0, "Main", 0, 3),
                    (1, "Head", 0, 1),
                    (1, name, 1, 2),
                    (1, "End", 2, 3)
                ]
            );
        } else {
            assert_eq!(
                result.unwrap_err(),
                ParseError::UnexpectedToken {
                    offset: 1,
                    state: 1,
                    term: Some(2)
                }
            );
        }
    }
}

// @top Main { Part Value | Mark Right }
// Part { Aux }
// Aux { Mark }
// @external tokens context from "tokens" { Mark, Value, Right }
fn context_spec(tokenizers: &[Tokenizer]) -> GrammarSpec<'_, Tokenizer> {
    GrammarSpec {
        tables: SerializedTables {
            states: "nOQOPOOOVOPO'#CbOOOP'#Ca'#CaO_OPOOQOOOOO",
            state_data: "d~OPPO~ORSOQUX~OQSO~O",
            goto: "^VPPPPPWZRRORQO",
            ..EMPTY
        },
        node_names: "⚠ Mark Value Right Main Part Aux",
        max_term: 7,
        repeat_node_count: 0,
        tokenizers,
        top_rule: [0, 4],
        token_prec: 0,
    }
}

#[test]
fn probes_reductions_without_mutating_the_stack_and_caches_by_slot() {
    for contextual in [false, true] {
        let observed = Arc::new(Mutex::new(Vec::new()));
        let record = Arc::clone(&observed);
        let tokenizers = [Tokenizer::External(ExternalTokenizer::new(
            ExternalOptions {
                contextual,
                ..Default::default()
            },
            move |input, stack| {
                let (value, right) = (stack.can_shift(2)?, stack.can_shift(3)?);
                record
                    .lock()
                    .unwrap()
                    .push((stack.state(), stack.byte_offset(), value, right));
                let term = match input.peek(0)? {
                    Some(33) => 1,
                    Some(120) if value => 2,
                    Some(121) if right => 3,
                    _ => return Ok(None),
                };
                input.advance(1)?;
                Ok(Some(input.token(term)))
            },
        ))];
        let parser = Parser::deserialize_with_hooks(context_spec(&tokenizers), &[]).unwrap();
        assert_eq!(
            tree_records(&parser.parse("!x").unwrap()),
            [
                (0, "Main", 0, 2),
                (1, "Part", 0, 1),
                (2, "Aux", 0, 1),
                (3, "Mark", 0, 1),
                (1, "Value", 1, 2),
            ]
        );
        assert_eq!(
            tree_records(&parser.parse("!y").unwrap()),
            [(0, "Main", 0, 2), (1, "Mark", 0, 1), (1, "Right", 1, 2),]
        );
        // Pinned callback traces. State 2 default-reduces Part without scanning.
        let mut expected = vec![(0, 0, false, false), (1, 1, true, true)];
        if contextual {
            expected.push((3, 1, true, false));
        }
        expected.extend([(0, 0, false, false), (1, 1, true, true)]);
        assert_eq!(*observed.lock().unwrap(), expected);
    }
}

#[test]
fn tracks_shifts_and_reductions_with_upstream_cursor_and_stack_positions() {
    // @top Main { Empty Item+ Tail }
    // Empty { }
    // @skip { space | Comment }
    // @skip {} { Comment { "#" "!" } }
    // @tokens { Item { "😀" } space { " " } Tail { @eof } }
    // @context track from "tokens"
    let observed = Arc::new(Mutex::new(Vec::new()));
    let record = |kind| {
        let observed = Arc::clone(&observed);
        move |context: &u16, term, stack: &StackView<'_>, input: &mut TokenInput<'_>| {
            assert_eq!(stack.context::<u16>(), Some(context));
            assert_eq!(stack.context::<bool>(), None);
            observed.lock().unwrap().push((
                kind,
                *context,
                term,
                stack.state(),
                stack.byte_offset(),
                input.position(),
                input.peek(0)?,
            ));
            Ok(Some(context + 1))
        }
    };
    let parser = Parser::deserialize(GrammarSpec {
        tables: SerializedTables {
            states: "!^OOQO?MsOPYOPOOO_QPOOPdOPO'#C]POOO)C>Y)C>YOOQO'#Cb'#CbOiQPOOPOOO,58w,58wOOQO-E6`-E6`QOQPOO",
            state_data: "q~OXOSYPQ~OYRO~OSTO~OZVO~OSTOTXO~O",
            goto: "dVPWPZPP^RSPRQOQUQRWU",
            token_data: "!R~RTpqbqrgstl;(c;(dq~~|~gOX~~lOZ~~qOY~~tP;Hf;Hgw~|OS~~!ROT~",
            ..EMPTY
        },
        node_names: "⚠ Comment Main Empty Item Tail",
        max_term: 11,
        repeat_node_count: 1,
        tokenizers: &[0],
        top_rule: [0, 2],
        ..EMPTY_GRAMMAR
    })
    .unwrap()
    .with_context_tracker(ContextTracker::new(0_u16).with_shift(record('S')).with_reduce(record('R')));
    let text = "😀 #! 😀😀";
    let trees = [parser.parse(text).unwrap(), parser.parse_glr(text).unwrap()];
    for tree in &trees {
        assert_eq!(
            tree_records(tree),
            [
                (0, "Main", 0, 16),
                (1, "Empty", 0, 0),
                (1, "Item", 0, 4),
                (1, "Comment", 5, 7),
                (1, "Item", 8, 12),
                (1, "Item", 12, 16),
                (1, "Tail", 16, 16),
            ]
        );
    }
    // Includes an epsilon rule, skip-and-stay, a scoped skip rule, anonymous
    // reductions, and an EOF shift. The goto into Comment does not call shift.
    let high = Some(0xd83d);
    let trace = [
        ('R', 0, 3, 2, 0, 0, high),
        ('S', 1, 4, 5, 4, 0, high),
        ('R', 2, 6, 6, 4, 0, high),
        ('S', 3, 9, 6, 5, 2, Some(32)),
        ('S', 4, 10, 3, 6, 3, Some(35)),
        ('S', 5, 11, 7, 7, 4, Some(33)),
        ('R', 6, 1, 4, 7, 3, Some(35)),
        ('R', 7, 8, 6, 7, 0, high),
        ('S', 8, 9, 6, 8, 5, Some(32)),
        ('S', 9, 4, 5, 12, 6, high),
        ('R', 10, 6, 8, 12, 6, high),
        ('R', 11, 6, 6, 12, 0, high),
        ('S', 12, 4, 5, 16, 8, high),
        ('R', 13, 6, 8, 16, 8, high),
        ('R', 14, 6, 6, 16, 0, high),
        ('S', 15, 5, 9, 16, 10, None),
    ];
    assert_eq!(*observed.lock().unwrap(), trace.repeat(2));
}

#[test]
fn invalidates_noncontextual_token_and_specialization_caches_on_a_reduction() {
    // @top Main { Part (Value | After) | Mark Right }
    // Part { Aux }
    // Aux { Mark }
    // @context track from "tokens"
    let observed = Arc::new(Mutex::new(Vec::new()));
    let record = Arc::clone(&observed);
    let scanner = Tokenizer::External(ExternalTokenizer::new(
        ExternalOptions::default(),
        move |input, stack| {
            let context = *stack.context::<bool>().unwrap();
            record
                .lock()
                .unwrap()
                .push((stack.state(), stack.byte_offset(), context));
            let term = match input.peek(0)? {
                Some(33) => 1,
                Some(120) => {
                    if context {
                        3
                    } else {
                        2
                    }
                }
                _ => return Ok(None),
            };
            input.advance(1)?;
            Ok(Some(input.token(term)))
        },
    ));
    // @external tokens scanner from "tokens" { Mark, Value, After, Right }
    let external = Parser::deserialize_with_hooks(
        GrammarSpec {
            tables: SerializedTables {
                states: "nOQOPOOOVOPO'#CcOOOP'#Cb'#CbObOPOOQOOOOO",
                state_data: "j~OPPO~OSSOQVXRVX~OQSORSO~O",
                goto: "_WPPPPPPX[RRORQO",
                ..EMPTY
            },
            node_names: "⚠ Mark Value After Right Main Part Aux",
            max_term: 8,
            repeat_node_count: 0,
            tokenizers: &[scanner],
            top_rule: [0, 5],
            token_prec: 0,
        },
        &[],
    )
    .unwrap();
    // @external specialize {word} specialize from "tokens" { Value, After, Right }
    // @tokens { Mark { "!" } word { "x" } }
    let record = Arc::clone(&observed);
    let internal = Parser::deserialize_with_hooks(
        GrammarSpec {
            tables: SerializedTables {
                states: "nOQOPOOOVOPO'#CbOOOO'#Ca'#CaObOPOOQOOOOO",
                state_data: "j~OVPO~ORSOPUXQUX~OPSOQSO~O",
                goto: "^VPPPPPWZRRORQO",
                token_data: "c~RQqrX#l#m^~^OV~~cOX~",
                ..EMPTY
            },
            node_names: "⚠ Value After Right Main Part Aux Mark",
            max_term: 9,
            repeat_node_count: 0,
            tokenizers: &[Tokenizer::Internal(0)],
            top_rule: [0, 4],
            token_prec: 0,
        },
        &[Specializer::new(9, move |text, stack| {
            assert_eq!(text, "x");
            let context = *stack.context::<bool>().unwrap();
            record
                .lock()
                .unwrap()
                .push((stack.state(), stack.byte_offset(), context));
            Ok(Some(Specialization::Replace(if context { 2 } else { 1 })))
        })],
    )
    .unwrap();
    for (parser, part) in [(external, 6), (internal, 5)] {
        let parser = parser.with_context_tracker(
            ContextTracker::new(false)
                .with_reduce(move |_, term, _, _| Ok((term == part).then_some(true))),
        );
        assert_eq!(
            tree_records(&parser.parse("!x").unwrap()),
            [
                (0, "Main", 0, 2),
                (1, "Part", 0, 1),
                (2, "Aux", 0, 1),
                (3, "Mark", 0, 1),
                (1, "After", 1, 2),
            ]
        );
    }
    assert_eq!(
        *observed.lock().unwrap(),
        [
            (0, 0, false),
            (1, 1, false),
            (3, 1, true),
            (1, 1, false),
            (3, 1, true),
        ]
    );
}

#[test]
fn keeps_glr_contexts_and_cached_misses_separate() {
    // @top Main { (A | B) "y" End }
    // A[@dynamicPrecedence=2] { "x" ~same }
    // B { "x" ~same }
    // @external tokens fin from "tokens" { End }
    // @context track from "tokens"
    for (allowed, rule) in [(3, "A"), (4, "B")] {
        let fin = Tokenizer::External(ExternalTokenizer::new(
            ExternalOptions::default(),
            move |input, stack| {
                if stack.context::<u16>() == Some(&allowed) && input.peek(0)? == Some(122) {
                    input.advance(1)?;
                    Ok(Some(input.token(1)))
                } else {
                    Ok(None)
                }
            },
        ));
        let parser = Parser::deserialize_with_hooks(
            GrammarSpec {
                tables: SerializedTables {
                    states: "nOQOPOOOVOPO'#C_O_OPOOOdOQOOQOOOOO",
                    state_data: "i~OUPO~OVRXVSX~OVRO~OPSO~O",
                    goto: "XTPPPUURQO",
                    token_data: "c~RQ#l#mX#m#n^~^OU~~cOV~",
                    ..EMPTY
                },
                node_names: "⚠ End Main A B",
                max_term: 7,
                repeat_node_count: 0,
                tokenizers: &[Tokenizer::Internal(0), fin],
                top_rule: [0, 2],
                token_prec: 0,
            },
            &[],
        )
        .unwrap()
        .with_dynamic_precedence(&[(3, 2)])
        .unwrap()
        .with_context_tracker(
            ContextTracker::new(0_u16)
                .with_reduce(|_, term, _, _| Ok(matches!(term, 3 | 4).then_some(term))),
        );
        // Pinned JS accepts with A and reports "No parse at 2" with B. It
        // merges on LR states alone and discards B before the tokenizer runs.
        assert_eq!(
            tree_records(&parser.parse_glr("xyz").unwrap()),
            [(0, "Main", 0, 3), (1, rule, 0, 1), (1, "End", 2, 3),]
        );
    }
}

#[test]
fn keeps_distinct_glr_frame_starts_for_reduce_hooks() {
    // @external tokens parts from "tokens" { Segment, End }
    // @top Main { (A | B) C End }
    // A { x ~same }
    // B[@dynamicPrecedence=2] { x ~same y }
    // C { Segment }
    // @tokens { x { "x" } y { "y" } }
    // @context track from "tokens"
    let parts = Tokenizer::External(ExternalTokenizer::new(
        ExternalOptions {
            extend: true,
            ..Default::default()
        },
        |input, stack| {
            let (term, len) = match input.peek(0)? {
                Some(121) => (1, 2), // A's Segment is "yz".
                Some(122) => (1, 1), // B's Segment is "z".
                Some(33) if stack.context::<u16>() == Some(&121) => (2, 1),
                _ => return Ok(None),
            };
            input.advance(len)?;
            Ok(Some(input.token(term)))
        },
    ));
    let parser = Parser::deserialize_with_hooks(
        GrammarSpec {
            tables: SerializedTables {
                states: "zOQOQOOOVORO'#C`O_OPOOOOOP,58{,58{OOOP'#Cb'#CbOdOPOOQOOOOO",
                state_data: "i~OWPO~OXROPSX~OPSO~OQUO~O",
                goto: "^VPPPPWWZRQORTQ",
                token_data: "c~RQ#l#mX#m#n^~^OW~~cOX~",
                ..EMPTY
            },
            node_names: "⚠ Segment End Main A B C",
            max_term: 9,
            repeat_node_count: 0,
            tokenizers: &[parts, Tokenizer::Internal(0)],
            top_rule: [0, 3],
            token_prec: 0,
        },
        &[],
    )
    .unwrap()
    .with_dynamic_precedence(&[(5, 2)])
    .unwrap()
    .with_context_tracker(ContextTracker::new(0_u16).with_reduce(|_, term, _, input| {
        if term == 6 { input.peek(0) } else { Ok(None) }
    }));
    // Pinned JS reports "No parse at 3" because it discards A. Before
    // reducing C, the contexts match, but C starts at a different offset.
    assert_eq!(
        tree_records(&parser.parse_glr("xyz!").unwrap()),
        [
            (0, "Main", 0, 4),
            (1, "A", 0, 1),
            (1, "C", 1, 3),
            (2, "Segment", 1, 3),
            (1, "End", 3, 4),
        ]
    );
}

#[test]
fn external_lookaround_reads_utf16_but_token_ends_are_checked() {
    let tokenizers = [Tokenizer::External(ExternalTokenizer::new(
        ExternalOptions::default(),
        |input, stack| {
            if stack.byte_offset() == 0 {
                assert_eq!(input.position(), 0);
                assert_eq!(input.peek(0)?, Some(0xd83d));
                assert_eq!(input.advance(1)?, Some(0xde00));
                assert_eq!(input.peek(1)?, Some(0x00e9));
                input.advance(1)?;
                Ok(Some(input.token(1)))
            } else {
                assert_eq!((stack.byte_offset(), input.position()), (4, 2));
                assert_eq!(input.peek(-1)?, Some(0xde00));
                assert_eq!(input.advance(usize::MAX)?, None);
                Ok(Some(input.token(2)))
            }
        },
    ))];
    // The Value specializer sees the original UTF-8 text, after a can_shift probe.
    let specializer = Specializer::new(2, |text, stack| {
        assert_eq!(text, "é");
        Ok(stack.can_shift(3)?.then_some(Specialization::Replace(3)))
    });
    let parser = Parser::deserialize_with_hooks(context_spec(&tokenizers), &[specializer]).unwrap();
    assert_eq!(
        tree_records(&parser.parse("😀é").unwrap()),
        [(0, "Main", 0, 6), (1, "Mark", 0, 4), (1, "Right", 4, 6),]
    );

    for (term, end, expected) in [
        (1, 1, ParseError::InvalidTokenEnd { offset: 0, end: 1 }),
        (
            1,
            usize::MAX,
            ParseError::InvalidTokenEnd {
                offset: 0,
                end: usize::MAX,
            },
        ),
        (0, 2, ParseError::InvalidTokenTerm { offset: 0, term: 0 }),
        (7, 2, ParseError::InvalidTokenTerm { offset: 0, term: 7 }),
        (8, 2, ParseError::InvalidTokenTerm { offset: 0, term: 8 }),
    ] {
        let slots = [Tokenizer::External(ExternalTokenizer::new(
            Default::default(),
            move |_, _| Ok(Some(ExternalToken { term, end })),
        ))];
        let parser = Parser::deserialize_with_hooks(context_spec(&slots), &[]).unwrap();
        assert_eq!(parser.parse("😀").unwrap_err(), expected);
        assert_eq!(parser.parse_recovering("😀").unwrap_err(), expected);
    }
    let slots = [Tokenizer::External(ExternalTokenizer::new(
        Default::default(),
        |_, stack| {
            Ok(Some(if stack.byte_offset() == 0 {
                ExternalToken { term: 1, end: 2 }
            } else {
                ExternalToken { term: 2, end: 0 }
            }))
        },
    ))];
    let parser = Parser::deserialize_with_hooks(context_spec(&slots), &[]).unwrap();
    assert_eq!(
        parser.parse("😀").unwrap_err(),
        ParseError::InvalidTokenEnd { offset: 4, end: 0 }
    );
}

#[test]
fn specializes_and_extends_dfa_tokens() {
    // @top Main { "#" extended | "!" Word | "?" (extended | Word) | "@" specialized }
    // extended { @extend[@name=Keyword]<word, "kw"> }
    // specialized { @specialize[@name=Let]<word, "let"> }
    // Word { word }
    // @tokens { word { $[a-z]+ } }
    let spec = GrammarSpec {
        tables: SerializedTables {
            states: "zOQOPOOO`OPOOOeOPOOOmOPOOOrOPOOOOOO'#C_'#C_QOOOOO",
            state_data: "w~OUROWPOXQOYSO~OVTO~OQUOVTO~OQUO~OSUO~O",
            goto: "XSPPPTTUPQ",
            token_data: "!O~RTqrbstg!a!bl!b!cq#T#ov~gOW~~lOU~~qOX~~vOY~~{PV~#T#ov",
            ..EMPTY
        },
        node_names: "⚠ Main Keyword Word Let",
        max_term: 10,
        repeat_node_count: 0,
        tokenizers: &[Tokenizer::Internal(0)],
        top_rule: [0, 1],
        token_prec: 0,
    };
    let parser = Parser::deserialize_with_hooks(
        spec.clone(),
        &[
            Specializer::new(7, |_, _| Ok(None)),
            Specializer::new(7, |text, _| {
                Ok(match text {
                    "kw" => Some(Specialization::Extend(2)),
                    "let" => Some(Specialization::Replace(4)),
                    _ => None,
                })
            }),
        ],
    )
    .unwrap();
    for (text, node) in [
        ("#kw", "Keyword"),
        ("!kw", "Word"),
        ("?abc", "Word"),
        ("@let", "Let"),
    ] {
        assert_eq!(
            tree_records(&parser.parse(text).unwrap()),
            [(0, "Main", 0, text.len()), (1, node, 1, text.len())]
        );
    }
    // Upstream returns distinct actions [6, Keyword, 3; 5, word, 3].
    assert_eq!(
        parser.parse("?kw").unwrap_err(),
        ParseError::Ambiguous {
            offset: 1,
            state: 2,
            term: 7
        }
    );
    assert_eq!(
        tree_records(&parser.parse_glr("?kw").unwrap()),
        [(0, "Main", 0, 3), (1, "Keyword", 1, 3)]
    );
    // Replacement is not a fallback to the base term.
    assert_eq!(
        parser.parse("!let").unwrap_err(),
        ParseError::UnexpectedToken {
            offset: 1,
            state: 1,
            term: Some(4)
        }
    );
    for term in [0, 5, 11] {
        for invalid in [
            Specializer::context_free(term, &[]),
            Specializer::context_free(7, &[("let", Specialization::Replace(term))]),
            Specializer::context_free(7, &[("kw", Specialization::Extend(term))]),
            Specializer::new(term, |_, _| Ok(None)),
        ] {
            assert_eq!(
                Parser::deserialize_with_hooks(spec.clone(), &[invalid]).unwrap_err(),
                GrammarError::InvalidMetadata("specialized")
            );
        }
        let invalid = Specializer::new(7, move |_, _| Ok(Some(Specialization::Replace(term))));
        let parser = Parser::deserialize_with_hooks(spec.clone(), &[invalid]).unwrap();
        assert_eq!(
            parser.parse("@let").unwrap_err(),
            ParseError::InvalidTokenTerm { offset: 1, term }
        );
    }
    let pure = Parser::deserialize_with_hooks(
        spec,
        &[
            Specializer::context_free(
                7,
                &[
                    ("kw", Specialization::Replace(4)),
                    ("kw", Specialization::Extend(2)),
                ],
            ),
            Specializer::context_free(
                7,
                &[
                    ("kw", Specialization::Replace(4)),
                    ("let", Specialization::Replace(4)),
                ],
            ),
        ],
    )
    .unwrap();
    assert_eq!(
        tree_records(&pure.parse_glr("?kw").unwrap()),
        tree_records(&parser.parse_glr("?kw").unwrap())
    );
    for text in ["#kw", "!kw", "?abc", "@let", "?kw", "!let"] {
        match parser.parse(text) {
            Ok(expected) => assert_eq!(
                tree_records(&pure.parse(text).unwrap()),
                tree_records(&expected)
            ),
            Err(error) => assert_eq!(pure.parse(text).unwrap_err(), error),
        }
    }
}

#[test]
fn parses_xml_delimiter_scans_with_external_tokenizers() {
    // @lezer/xml 1.0.6 rules for Comment, ProcessingInst, and Cdata, with a
    // narrowed top rule: @top Document { (Comment | ProcessingInst | Cdata)+ }
    // Tables generated with @lezer/generator 1.8.1. The tree below matches
    // both the excerpt and the full published grammar on @lezer/lr 1.4.13.
    fn scan_to(term: u16, delimiter: &'static [u8]) -> Tokenizer {
        Tokenizer::External(ExternalTokenizer::new(
            Default::default(),
            move |input, _| {
                let start = input.position();
                'scan: while let Some(next) = input.peek(0)? {
                    if next == u16::from(delimiter[0]) {
                        for (i, &unit) in delimiter.iter().enumerate().skip(1) {
                            if input.peek(i as isize)? != Some(u16::from(unit)) {
                                input.advance(1)?;
                                continue 'scan;
                            }
                        }
                        break;
                    }
                    input.advance(1)?;
                }
                Ok((input.position() > start).then(|| input.token(term)))
            },
        ))
    }
    let parser = Parser::deserialize_with_hooks(GrammarSpec {
        tables: SerializedTables {
            version: 14,
            states: "#xOQOWOOO]OXO'#C^OeOYO'#C_OmO[O'#C`OOOO'#Ca'#CaQQOWOOOOOP'#Cb'#CbOuOXO,58xOOOO,58x,58xOOOQ'#Cc'#CcO}OYO,58yOOOO,58y,58yOOOS'#Cd'#CdO!VO[O,58zOOOO,58z,58zOOOO-E6_-E6_OOOP-E6`-E6`OOOO1G.d1G.dOOOQ-E6a-E6aOOOO1G.e1G.eOOOS-E6b-E6bOOOO1G.f1G.f",
            state_data: "!_~O]PO_QOaRO~OYUO^WO~OZXO`ZO~O[[Ob^O~OYUO^aO~OZXO`cO~O[[ObeO~O",
            goto: "vXPPYYY^djpTSOTQTOR_TQVPR`VQYQRbYQ]RRd]",
            token_data: "$h~RS}!O_!^!_p!a!b#z#P#Q$V~bP}!Oe~hP!`!ak~pO^~~sQqry!a!b#u~|Q}!O!S!}#O!_~!VP}!O!Y~!_O]~~!bQ!e!f!h#V#W#]~!kP!f!g!n~!qP!c!d!t~!wP!v!w!z~!}P!c!d#Q~#TP!}#O#W~#]Oa~~#`P#W#X#c~#fP#T#U#i~#lP#h#i#o~#rP#T#U#Q~#zO_~~#}P!`!a$Q~$VO`~~$YP#P#Q$]~$`P!`!a$c~$hOb~",
        },
        node_names: "⚠ Document Comment ProcessingInst Cdata",
        max_term: 18,
        repeat_node_count: 4,
        tokenizers: &[scan_to(10, b"-->"), scan_to(11, b"?>"), scan_to(12, b"]]>"), Tokenizer::Internal(0)],
        top_rule: [0, 1],
        token_prec: 0,
    }, &[]).unwrap();
    let source = "<!--é😀--><?p?><![CDATA[🦀]]><!---->";
    let tree = parser.parse_incremental(source, None).unwrap();
    let changed = source.replace('🦀', "海");
    let next = parser
        .parse_incremental(
            &changed,
            Some(prefix_fragment(&tree, source.find('🦀').unwrap())),
        )
        .unwrap();
    assert_eq!(next.reuse_stats(), ReuseStats::default());
    assert_eq!(
        tree_records(&next),
        tree_records(&parser.parse(&changed).unwrap())
    );
    assert_eq!(
        tree_records(&tree),
        [
            (0, "Document", 0, 41),
            (1, "Comment", 0, 13),
            (1, "ProcessingInst", 13, 18),
            (1, "Cdata", 18, 34),
            (1, "Comment", 34, 41),
        ]
    );
}

#[test]
fn parses_published_wast_type_specializations() {
    // @codemirror/lang-wast 6.0.2, dist/index.js and spec_Keyword.
    // See docs/licenses/codemirror-lang-wast.txt. Traversal from @lezer/lr 1.4.13.
    let spec = GrammarSpec {
        tables: SerializedTables {
            version: 14,
            states: "!^Q]QPOOOqQPO'#CbOOQO'#Cd'#CdOOQO'#Cl'#ClOOQO'#Ch'#ChQ]QPOOOOQO,58|,58|OxQPO,58|OOQO-E6f-E6fOOQO1G.h1G.h",
            state_data: "!P~O_OSPOSQOS~OTPOVROXROYROZROaQO~OSUO~P]OSXO~P]O",
            goto: "xaPPPPPPbPbPPPhPPPrXROPTVQTOQVPTWTVXSOPTV",
            token_data: "0o~R^XY}YZ}]^}pq}rs!Stu#pxy'Uyz(e{|(j}!O(j!Q!R(s!R![*p!]!^.^#T#o.{~!SO_~~!VVOr!Srs!ls#O!S#O#P!q#P;'S!S;'S;=`#j<%lO!S~!qOZ~~!tRO;'S!S;'S;=`!};=`O!S~#QWOr!Srs!ls#O!S#O#P!q#P;'S!S;'S;=`#j;=`<%l!S<%lO!S~#mP;=`<%l!S~#siqr%bst%btu%buv%bvw%bwx%bz{%b{|%b}!O%b!O!P%b!P!Q%b!Q![%b![!]%b!^!_%b!_!`%b!`!a%b!a!b%b!b!c%b!c!}%b#Q#R%b#R#S%b#S#T%b#T#o%b#p#q%b#r#s%b~%giV~qr%bst%btu%buv%bvw%bwx%bz{%b{|%b}!O%b!O!P%b!P!Q%b!Q![%b![!]%b!^!_%b!_!`%b!`!a%b!a!b%b!b!c%b!c!}%b#Q#R%b#R#S%b#S#T%b#T#o%b#p#q%b#r#s%b~'ZPT~!]!^'^~'aTO!]'^!]!^'p!^;'S'^;'S;=`(_<%lO'^~'sVOy'^yz(Yz!]'^!]!^'p!^;'S'^;'S;=`(_<%lO'^~(_OQ~~(bP;=`<%l'^~(jOS~~(mQ!Q!R(s!R![*p~(xUY~!O!P)[!Q![*p!g!h){#R#S+U#X#Y){#l#m+[~)aRY~!Q![)j!g!h){#X#Y){~)oSY~!Q![)j!g!h){#R#S*j#X#Y){~*OR{|*X}!O*X!Q![*_~*[P!Q![*_~*dQY~!Q![*_#R#S*X~*mP!Q![)j~*uTY~!O!P)[!Q![*p!g!h){#R#S+U#X#Y){~+XP!Q![*p~+_R!Q![+h!c!i+h#T#Z+h~+mVY~!O!P,S!Q![+h!c!i+h!r!s-P#R#S+[#T#Z+h#d#e-P~,XTY~!Q![,h!c!i,h!r!s-P#T#Z,h#d#e-P~,mUY~!Q![,h!c!i,h!r!s-P#R#S.Q#T#Z,h#d#e-P~-ST{|-c}!O-c!Q![-o!c!i-o#T#Z-o~-fR!Q![-o!c!i-o#T#Z-o~-tSY~!Q![-o!c!i-o#R#S-c#T#Z-o~.TR!Q![,h!c!i,h#T#Z,h~.aP!]!^.d~.iSP~OY.dZ;'S.d;'S;=`.u<%lO.d~.xP;=`<%l.d~/QiX~qr.{st.{tu.{uv.{vw.{wx.{z{.{{|.{}!O.{!O!P.{!P!Q.{!Q![.{![!].{!^!_.{!_!`.{!`!a.{!a!b.{!b!c.{!c!}.{#Q#R.{#R#S.{#S#T.{#T#o.{#p#q.{#r#s.{",
        },
        node_names: "⚠ LineComment BlockComment Module ) ( App Identifier Type Keyword Number String",
        max_term: 17,
        repeat_node_count: 1,
        tokenizers: &[Tokenizer::Internal(0)],
        top_rule: [0, 3],
        token_prec: 0,
    };
    const TYPES: [&str; 12] = [
        "anyref",
        "dataref",
        "eqref",
        "externref",
        "i31ref",
        "funcref",
        "i8",
        "i16",
        "i32",
        "i64",
        "f32",
        "f64",
    ];
    let parser = Parser::deserialize_with_hooks(
        spec.clone(),
        &[Specializer::new(9, |text, _| {
            Ok(TYPES.contains(&text).then_some(Specialization::Replace(17)))
        })],
    )
    .unwrap();
    let pure = Parser::deserialize_with_hooks(
        spec,
        &[Specializer::context_free(
            9,
            &TYPES.map(|text| (text, Specialization::Replace(17))),
        )],
    )
    .unwrap();
    let prefix = format!("(module {} (result ", "(func (param i32)) ".repeat(40));
    let original = format!("{prefix}funcref))");
    let edit = format!("{prefix}funcrefExtra))");
    for (parser, expect_reuse) in [(&parser, false), (&pure, true)] {
        let before = parser.parse_incremental(&original, None).unwrap();
        let after = parser
            .parse_incremental(&edit, Some(prefix_fragment(&before, prefix.len() + 7)))
            .unwrap();
        let reuse = after.reuse_stats();
        if expect_reuse {
            assert!(reuse.nodes_reused > 100, "{reuse:?}");
            assert!((64..prefix.len()).contains(&reuse.resumed_at), "{reuse:?}");
        } else {
            assert_eq!(reuse, ReuseStats::default());
        }
        assert_eq!(
            tree_records(&after),
            tree_records(&pure.parse(&edit).unwrap())
        );
        assert_ne!(tree_records(&before), tree_records(&after));
    }
    let source = "(module (func (param i32) (result funcref)) (export \"é😀\"))";
    let tree = parser.parse_incremental(source, None).unwrap();
    let edit = source.replace("funcref", "i64");
    let updated = parser
        .parse_incremental(
            &edit,
            Some(prefix_fragment(&tree, source.find("funcref").unwrap())),
        )
        .unwrap();
    assert_eq!(updated.reuse_stats(), ReuseStats::default());
    assert_eq!(
        tree_records(&updated),
        tree_records(&parser.parse(&edit).unwrap())
    );
    assert_eq!(
        tree_records(&tree),
        [
            (0, "Module", 0, 62),
            (1, "App", 0, 62),
            (2, "(", 0, 1),
            (2, "Keyword", 1, 7),
            (2, "App", 8, 43),
            (3, "(", 8, 9),
            (3, "Keyword", 9, 13),
            (3, "App", 14, 25),
            (4, "(", 14, 15),
            (4, "Keyword", 15, 20),
            (4, "Type", 21, 24),
            (4, ")", 24, 25),
            (3, "App", 26, 42),
            (4, "(", 26, 27),
            (4, "Keyword", 27, 33),
            (4, "Type", 34, 41),
            (4, ")", 41, 42),
            (3, ")", 42, 43),
            (2, "App", 44, 61),
            (3, "(", 44, 45),
            (3, "Keyword", 45, 51),
            (3, "String", 52, 60),
            (3, ")", 60, 61),
            (2, ")", 61, 62),
        ]
    );
}

fn prefix_fragment(tree: &SyntaxTree, end: usize) -> TreeFragment<'_> {
    TreeFragment {
        tree,
        old_range: 0..end,
        new_range: 0..end,
    }
}

#[test]
fn resumes_published_json_after_repeated_utf8_edits() {
    let parser = Parser::deserialize(JSON_GRAMMAR).unwrap();
    let prefix = format!(
        "[{}",
        "{\"label\":\"é😀\",\"values\":[1,true,null]},".repeat(100)
    );
    let mut previous = parser
        .parse_incremental(&format!("{prefix}0]"), None)
        .unwrap();
    assert_eq!(previous.reuse_stats(), ReuseStats::default());
    for tail in ["{\"label\":\"🦀\"}]", "\"é\",false,null]", "0]", "[1,2,3]]"] {
        let source = format!("{prefix}{tail}");
        let next = parser
            .parse_incremental(&source, Some(prefix_fragment(&previous, prefix.len())))
            .unwrap();
        assert_eq!(
            tree_records(&next),
            tree_records(&parser.parse(&source).unwrap())
        );
        assert_eq!(
            tree_records(&next),
            tree_records(&parser.parse_glr(&source).unwrap())
        );
        let stats = next.reuse_stats();
        assert!(stats.resumed_at > prefix.len() - 128, "{stats:?}");
        assert!(stats.resumed_at <= prefix.len(), "{stats:?}");
        assert!(stats.nodes_reused > 1000, "{stats:?}");
        previous = next;
    }
    let invalid = format!("{prefix}]");
    assert_eq!(
        parser
            .parse_incremental(&invalid, Some(prefix_fragment(&previous, prefix.len())))
            .unwrap_err(),
        parser.parse(&invalid).unwrap_err(),
    );
}

#[test]
fn resumes_with_skip_rules_and_an_edit_before_previously_shared_nodes() {
    let parser = skip_parser();
    let prefix = "a/ # comment / spaces! ".repeat(80);
    let source = format!("{prefix}b/ #tail");
    let seed = parser.parse_incremental(&source, None).unwrap();
    // A late edit reuses most of the seed. A second edit branches back into
    // that shared history, preserving interior and trailing skipped nodes.
    let other = parser
        .parse_incremental(
            &format!("{prefix}abc/"),
            Some(prefix_fragment(&seed, prefix.len())),
        )
        .unwrap();
    assert!(other.reuse_stats().nodes_reused > 200);
    let cut = prefix.len() / 2;
    let edited = format!("{}# removed a pair! zz/ #tail", &prefix[..cut]);
    let next = parser
        .parse_incremental(&edited, Some(prefix_fragment(&other, cut)))
        .unwrap();
    assert!(next.reuse_stats().nodes_reused > 100);
    assert!(next.reuse_stats().resumed_at < other.reuse_stats().resumed_at);
    assert_eq!(
        tree_records(&next),
        tree_records(&parser.parse(&edited).unwrap())
    );
    assert_eq!(
        tree_records(&seed),
        tree_records(&parser.parse(&source).unwrap())
    );
}

#[test]
fn ignores_bad_ranges_changed_prefixes_and_trees_from_another_parser_or_mode() {
    let parser = Parser::deserialize(JSON_GRAMMAR).unwrap();
    let second = Parser::deserialize(JSON_GRAMMAR).unwrap();
    let source = format!("[{}0]", "\"é\",".repeat(100));
    let changed = source.replacen('é', "ñ", 1);
    let tree = parser.parse_incremental(&source, None).unwrap();
    let plain = parser.parse(&source).unwrap();
    let glr = parser.parse_glr(&source).unwrap();
    let recovering = parser.parse_recovering(&source).unwrap();
    let span = 0..source.len();
    let past_end = source.len() + 1;
    for (parser, input, fragment) in [
        (&parser, source.as_str(), prefix_fragment(&tree, 3)), // Mid-char.
        (&parser, &source, prefix_fragment(&tree, past_end)),
        (
            &parser,
            &source,
            TreeFragment {
                tree: &tree,
                old_range: span.clone(),
                new_range: 0..1,
            },
        ),
        (
            &parser,
            &source,
            TreeFragment {
                tree: &tree,
                old_range: 1..past_end,
                new_range: span.clone(),
            },
        ),
        (
            &parser,
            &source,
            TreeFragment {
                tree: &tree,
                old_range: 1..source.len(),
                new_range: 1..source.len(),
            },
        ),
        (&parser, &changed, prefix_fragment(&tree, source.len())),
        (&second, &source, prefix_fragment(&tree, source.len())),
        (&parser, &source, prefix_fragment(&plain, source.len())),
        (&parser, &source, prefix_fragment(&glr, source.len())),
        (&parser, &source, prefix_fragment(&recovering, source.len())),
    ] {
        let fresh = parser.parse(input).unwrap();
        let next = parser.parse_incremental(input, Some(fragment)).unwrap();
        assert_eq!(next.reuse_stats(), ReuseStats::default());
        assert_eq!(tree_records(&next), tree_records(&fresh));
    }
}

#[test]
fn bypasses_incremental_checkpoints_when_a_context_tracker_is_set() {
    let parser = Parser::deserialize(JSON_GRAMMAR).unwrap();
    let prefix = format!("[{}", "\"é\",".repeat(100));
    let original = format!("{prefix}0]");
    let edited = format!("{prefix}1]");
    let seed = parser.parse_incremental(&original, None).unwrap();
    let fragment = prefix_fragment(&seed, prefix.len());
    let reused = parser
        .parse_incremental(&edited, Some(fragment.clone()))
        .unwrap();
    assert!(reused.reuse_stats().resumed_at > 0);

    let starts = Arc::new(AtomicUsize::new(0));
    let count = Arc::clone(&starts);
    let parser = parser.with_context_tracker(ContextTracker::new(false).with_shift(
        move |started, _, stack, input| {
            if !started {
                assert_eq!(input.position(), 0);
                assert_eq!(stack.byte_offset(), 1);
                count.fetch_add(1, Ordering::Relaxed);
            }
            Ok((!*started).then_some(true))
        },
    ));
    let tracked = parser.parse_incremental(&original, None).unwrap();
    for fragment in [fragment, prefix_fragment(&tracked, prefix.len())] {
        let next = parser.parse_incremental(&edited, Some(fragment)).unwrap();
        assert_eq!(next.reuse_stats(), ReuseStats::default());
        assert_eq!(tree_records(&next), tree_records(&reused));
    }
    assert_eq!(tracked.reuse_stats(), ReuseStats::default());
    assert_eq!(starts.load(Ordering::Relaxed), 3);
}

#[test]
fn does_not_reuse_a_number_that_read_the_old_eof() {
    let parser = Parser::deserialize(JSON_GRAMMAR).unwrap();
    let previous = parser.parse_incremental("1", None).unwrap();
    let fragment = prefix_fragment(&previous, 1);
    let unchanged = parser
        .parse_incremental("1", Some(fragment.clone()))
        .unwrap();
    assert_eq!(unchanged.reuse_stats().nodes_reused, 1);
    let longer = parser.parse_incremental("12", Some(fragment)).unwrap();
    assert_eq!(longer.reuse_stats(), ReuseStats::default());
    assert_eq!(
        tree_records(&longer),
        [(0, "JsonText", 0, 2), (1, "Number", 0, 2)]
    );
}

// Encode hand-built tables for validation, limits, and lookahead tests. All valid fixture strings above
// come from the pinned packages, not this encoder.
fn encode(words: &[u32]) -> String {
    fn digit(n: u32) -> char {
        let mut byte = n as u8 + b' ';
        for skipped in [b'"', b'\\'] {
            if byte >= skipped {
                byte += 1;
            }
        }
        char::from(byte)
    }
    fn integer(output: &mut String, n: u32) {
        if n == u32::from(u16::MAX) {
            output.push('~');
            return;
        }
        let mut base = 1_u32;
        while n / base >= 46 {
            base *= 46;
        }
        let mut rest = n;
        while base > 1 {
            output.push(digit(rest / base));
            rest %= base;
            base /= 46;
        }
        output.push(digit(rest + 46));
    }
    let mut output = String::new();
    integer(&mut output, words.len() as u32);
    for &n in words {
        integer(&mut output, n);
    }
    output
}

fn long_lookahead_parser() -> Parser {
    // One accepting state shifts Short, Long, and Fill. Short is `a`, Long
    // is `a[ab]*c`, and Fill is `b` or `x`. The DFA can scan far past Short
    // looking for a `c` and then return Short instead.
    let states = encode(&[2, 2, 0, 1, 0, 0]);
    let state_data = encode(&[65535, 0, 2, 0, 0, 3, 0, 0, 4, 0, 0, 65535, 0]);
    let token_data = encode(&[
        1, 3, 3, 97, 98, 12, 98, 99, 32, 120, 121, 32, 1, 17, 2, 2, 1, 97, 99, 23, 99, 100, 37, 1,
        26, 2, 97, 99, 23, 99, 100, 37, 1, 37, 0, 4, 1, 1, 42, 0, 3, 1,
    ]);
    Parser::deserialize(GrammarSpec {
        tables: SerializedTables {
            states: &states,
            state_data: &state_data,
            token_data: &token_data,
            ..EMPTY
        },
        node_names: "⚠ Main Short Long Fill",
        max_term: 5,
        tokenizers: &[0],
        ..EMPTY_GRAMMAR
    })
    .unwrap()
}

#[test]
fn does_not_reuse_when_a_rejected_lookahead_edge_changes() {
    let parser = long_lookahead_parser();
    let prefix = format!("a{}", "b".repeat(150));
    let previous = parser
        .parse_incremental(&format!("{prefix}x"), None)
        .unwrap();
    assert_eq!(previous.root().children().next().unwrap().name(), "Short");
    let source = format!("{prefix}c");
    let next = parser
        .parse_incremental(&source, Some(prefix_fragment(&previous, prefix.len())))
        .unwrap();
    assert_eq!(next.reuse_stats(), ReuseStats::default());
    assert_eq!(
        tree_records(&next),
        [(0, "Main", 0, source.len()), (1, "Long", 0, source.len())]
    );
}

#[test]
fn charges_shared_work_against_the_new_document_limit() {
    let parser = long_lookahead_parser();
    let source = format!("{}x", "a".repeat(400));
    let seed_text = format!("{source}{}", "b".repeat(600));
    let previous = parser.parse_incremental(&seed_text, None).unwrap();
    let error = parser.parse(&source).unwrap_err();
    assert!(matches!(error, ParseError::LimitExceeded { .. }));
    assert_eq!(
        parser
            .parse_incremental(&source, Some(prefix_fragment(&previous, source.len())))
            .unwrap_err(),
        error,
    );
}

#[test]
fn rejects_bad_grammar_metadata_and_table_references() {
    for (spec, error) in [
        (
            GrammarSpec {
                top_rule: [0, 0],
                ..EMPTY_GRAMMAR
            },
            GrammarError::InvalidMetadata("topRules entry"),
        ),
        (
            GrammarSpec {
                tokenizers: &[16],
                ..EMPTY_GRAMMAR
            },
            GrammarError::InvalidMetadata("tokenizers"),
        ),
        (
            GrammarSpec {
                tokenizers: &[],
                ..JSON_GRAMMAR
            },
            GrammarError::InvalidState { state: 0 },
        ),
        (
            GrammarSpec {
                token_prec: 2,
                ..EMPTY_GRAMMAR
            },
            GrammarError::InvalidTokenPrecedence { offset: 2 },
        ),
    ] {
        assert_eq!(Parser::deserialize(spec).unwrap_err(), error);
    }
    let tables = SerializedTables {
        state_data: &encode(&[65535, 0, 1, 5, 0, 65535, 0]),
        states: &encode(&[2, 2, 0, 0, 0, 0]),
        ..EMPTY
    };
    assert_eq!(
        Parser::deserialize(GrammarSpec {
            tables,
            ..EMPTY_GRAMMAR
        })
        .unwrap_err(),
        GrammarError::InvalidAction { offset: 2 }
    );
    let tables = SerializedTables {
        goto: &encode(&[2, 1, 3, 3, 1, 0]),
        ..EMPTY
    };
    assert_eq!(
        Parser::deserialize(GrammarSpec {
            tables,
            ..EMPTY_GRAMMAR
        })
        .unwrap_err(),
        GrammarError::InvalidGoto { offset: 3 }
    );
    for data in [
        &[1, 2, 0][..],                   // Accept section before the header.
        &[1, 4, 0, 1],                    // Partial accept pair.
        &[1, 5, 0, 2, 1],                 // Accepts the reserved EOF term.
        &[1, 3, 1, 30, 40, 1],            // Target is not a row start.
        &[1, 3, 2, 30, 40, 0, 35, 50, 0], // Overlapping edges.
        &[1, 3, 1, 40, 30, 0],            // Backwards edge.
    ] {
        let tables = SerializedTables {
            token_data: &encode(data),
            ..EMPTY
        };
        assert_eq!(
            Parser::deserialize(GrammarSpec {
                tables,
                ..EMPTY_GRAMMAR
            })
            .unwrap_err(),
            GrammarError::InvalidTokenData { offset: 0 }
        );
    }
}

#[test]
fn handles_missing_stack_transitions_and_bounds_nonprogressing_reductions() {
    let spec = GrammarSpec {
        node_names: "⚠ Main A",
        max_term: 3,
        ..EMPTY_GRAMMAR
    };
    for (action, goto, error) in [
        (
            65538,
            EMPTY.goto,
            ParseError::InvalidTransition {
                offset: 0,
                state: 0,
            },
        ), // Missing goto.
        (
            589826,
            EMPTY.goto,
            ParseError::InvalidTransition {
                offset: 0,
                state: 0,
            },
        ), // Stack underflow.
        (
            65538,
            &encode(&[3, 1, 1, 4, 3, 0, 0]),
            ParseError::LimitExceeded { offset: 0 },
        ), // Epsilon cycle.
    ] {
        let tables = SerializedTables {
            states: &encode(&[0, 0, 0, 0, action, 0]),
            goto,
            ..EMPTY
        };
        let parser = Parser::deserialize(GrammarSpec { tables, ..spec }).unwrap();
        assert_eq!(parser.parse("").unwrap_err(), error);
        assert_eq!(parser.parse_glr("").unwrap_err(), error);
        assert_eq!(parser.parse_recovering("").unwrap_err(), error);
        // The longer input leaves ample total work for the idle limit to stop the loop.
        assert_eq!(parser.parse_glr(&"x".repeat(1024)).unwrap_err(), error);
    }
}

#[test]
fn recovery_rejects_loose_forced_gotos_and_bounds_forced_cycles() {
    let parser = |action, source| {
        let states = encode(&[0, 0, 0, 0, 0, action, 2, 0, 0, 0, 0, 0]);
        let goto = encode(&[3, 1, 1, 4, 3, source, source]);
        Parser::deserialize(GrammarSpec {
            tables: SerializedTables {
                states: &states,
                goto: &goto,
                ..EMPTY
            },
            node_names: "⚠ Main A",
            max_term: 3,
            ..EMPTY_GRAMMAR
        })
        .unwrap()
    };
    for parser in [parser(65538, 1), parser(589826, 0)] {
        // The first forced slot has only a loose goto from state 0.
        // The second forced slot needs a frame that does not exist.
        assert_recovered(&parser, "", &[(0, "Main", 0, 0), (1, "⚠", 0, 0)]);
    }
    let tree = parser(65538, 0).parse_recovering("").unwrap();
    let root = tree.root();
    assert_eq!(
        root.children().filter(|node| node.name() == "A").count(),
        10
    );
    assert!(root.children().next_back().unwrap().is_error());
}

#[test]
fn bounds_recovery_depth_when_a_cut_cannot_shrink() {
    let source = "x".repeat(2800);
    for (forced, target, from, offset) in [
        (0, 0, 0, 2799),      // No forced slot.
        (65539, 0, 0, 2799),  // An epsilon reduction would grow the stack.
        (589827, 1, 1, 2799), // No exact goto for the forced slot.
        (589827, 1, 0, 2800), // A depth-one reduction replaces one frame, then stalls.
        (589827, 0, 0, 2800), // A depth-one reduction loops without shrinking.
    ] {
        let tables = SerializedTables {
            states: &encode(&[2, 0, 0, 1, 0, forced, 2, 0, 0, 1, 0, 0]),
            state_data: &encode(&[2, 0, 0, 65535, 0]),
            goto: &encode(&[4, 1, 1, 1, 5, 3, target, from]),
            ..EMPTY
        };
        let lexer = ExternalTokenizer::new_deterministic(ExternalOptions::default(), |input, _| {
            if input.peek(0)?.is_none() {
                return Ok(None);
            }
            input.advance(1)?;
            Ok(Some(input.token(2)))
        });
        let parser = Parser::deserialize_with_hooks(
            GrammarSpec {
                tables,
                node_names: "⚠ Main Token A",
                max_term: 4,
                repeat_node_count: 0,
                tokenizers: &[Tokenizer::External(lexer)],
                top_rule: [0, 1],
                token_prec: 0,
            },
            &[],
        )
        .unwrap();
        assert!(parser.parse_glr(&source).is_ok());
        assert!(parser.parse_recovering(&source[..2799]).is_ok());
        assert_eq!(
            parser.parse_recovering(&source).unwrap_err(),
            ParseError::LimitExceeded { offset },
        );
    }
}

#[test]
fn cuts_deep_json_only_in_recovery() {
    let parser = Parser::deserialize(JSON_GRAMMAR).unwrap();
    let source = format!("{}0{}", "[".repeat(2800), "]".repeat(2800));
    let strict = parser.parse(&source).unwrap();
    let records = tree_records(&strict);
    assert!(records.iter().all(|&(_, name, ..)| name != "⚠"));
    assert_eq!(tree_records(&parser.parse_glr(&source).unwrap()), records);
    assert_eq!(
        tree_records(&parser.parse_incremental(&source, None).unwrap()),
        records,
    );
    let recovered = parser.parse_recovering(&source).unwrap();
    assert_eq!(recovered.root().range(), 0..source.len());
    let recovered = tree_records(&recovered);
    let errors = recovered.iter().filter(|&&(_, name, ..)| name == "⚠");
    // @lezer/json 1.0.3 with @lezer/lr 1.4.13 cuts at this depth too.
    assert_eq!(errors.clone().next(), Some(&(2801, "⚠", 2800, 2800)));
    assert_eq!(errors.count(), 1604);
    assert_eq!(recovered.len(), 10006);
    let hash = recovered
        .into_iter()
        .flat_map(|(depth, name, from, to)| format!("{depth}:{name}:{from}:{to};").into_bytes())
        .fold(2_166_136_261_u32, |hash, byte| {
            (hash ^ u32::from(byte)).wrapping_mul(16_777_619)
        });
    assert_eq!(hash, 2_850_852_908);
}

#[test]
fn shares_work_across_speculative_insertions() {
    let parser = |second| {
        let states = encode(&[2, 2, 0, 0, 0, 0, 0, 0, 0, 1, 0, 0, 0, 0, 0, 1, 0, 0]);
        let mut actions = vec![65535, 0, 2, 1, 0];
        if second {
            actions.extend([3, 2, 0]);
        }
        actions.extend([65535, 0]);
        Parser::deserialize_with_hooks(
            GrammarSpec {
                tables: SerializedTables {
                    states: &states,
                    state_data: &encode(&actions),
                    ..EMPTY
                },
                node_names: "⚠ Main A B",
                max_term: 4,
                repeat_node_count: 0,
                tokenizers: &[Tokenizer::External(ExternalTokenizer::new(
                    ExternalOptions {
                        contextual: true,
                        ..Default::default()
                    },
                    |input, _| {
                        for _ in 0..900 {
                            input.peek(0)?;
                        }
                        Ok(None)
                    },
                ))],
                top_rule: [0, 1],
                token_prec: 0,
            },
            &[],
        )
        .unwrap()
    };
    assert_recovered(&parser(false), "x", &[(0, "Main", 0, 1), (1, "⚠", 0, 1)]);
    assert_eq!(
        parser(true).parse_recovering("x").unwrap_err(),
        ParseError::LimitExceeded { offset: 0 },
    );
}

#[test]
fn bounds_live_glr_branches_and_shares_work_at_a_fork() {
    let parser = |branches, per_scan| {
        let mut states = vec![0, 2, 0, 1, 0, 0];
        let mut actions = vec![65535, 0];
        for target in 1..=branches {
            actions.extend([2, target, 0]);
            states.extend([2, 0, 0, 1, 0, 0]);
        }
        actions.extend([65535, 0]);
        Parser::deserialize_with_hooks(
            GrammarSpec {
                tables: SerializedTables {
                    states: &encode(&states),
                    state_data: &encode(&actions),
                    ..EMPTY
                },
                node_names: "⚠ Main Token",
                max_term: 3,
                repeat_node_count: 0,
                tokenizers: &[Tokenizer::External(ExternalTokenizer::new(
                    ExternalOptions {
                        contextual: true,
                        ..Default::default()
                    },
                    move |input, stack| {
                        if stack.state() == 0 {
                            input.advance(1)?;
                            return Ok(Some(input.token(2)));
                        }
                        for _ in 0..per_scan {
                            input.peek(0)?;
                        }
                        Ok(None)
                    },
                ))],
                top_rule: [0, 1],
                token_prec: 0,
            },
            &[],
        )
        .unwrap()
    };
    assert!(parser(32, 0).parse_glr("x").is_ok());
    assert_eq!(
        parser(33, 0).parse_glr("x").unwrap_err(),
        ParseError::LimitExceeded { offset: 0 }
    );
    assert_eq!(
        parser(33, 0).parse_recovering("x").unwrap_err(),
        ParseError::LimitExceeded { offset: 0 }
    );
    assert!(parser(1, 600).parse_glr("x").is_ok());
    assert_eq!(
        parser(3, 600).parse_glr("x").unwrap_err(),
        ParseError::LimitExceeded { offset: 1 }
    );
}

#[test]
fn counts_branches_from_multiple_forks_against_the_same_cap() {
    // Every byte creates two branches. No pair has the same frame states.
    let states = encode(&[0, 2, 0, 1, 0, 0, 2, 2, 0, 1, 0, 0, 2, 2, 0, 1, 0, 0]);
    let state_data = encode(&[65535, 0, 2, 1, 0, 2, 2, 0, 65535, 0]);
    let parser = Parser::deserialize_with_hooks(
        GrammarSpec {
            tables: SerializedTables {
                states: &states,
                state_data: &state_data,
                ..EMPTY
            },
            node_names: "⚠ Main Token",
            max_term: 3,
            repeat_node_count: 0,
            tokenizers: &[Tokenizer::External(ExternalTokenizer::new(
                Default::default(),
                |input, _| {
                    if input.peek(0)?.is_none() {
                        return Ok(None);
                    }
                    input.advance(1)?;
                    Ok(Some(input.token(2)))
                },
            ))],
            top_rule: [0, 1],
            token_prec: 0,
        },
        &[],
    )
    .unwrap();
    assert_eq!(
        parser.parse_glr("xxxxx").unwrap().root().children().len(),
        5
    );
    assert_eq!(
        parser.parse_glr("xxxxxx").unwrap_err(),
        ParseError::LimitExceeded { offset: 5 }
    );
}

#[test]
fn bounds_external_work_stack_probes_and_zero_width_cycles() {
    let ordinary = [Tokenizer::External(ExternalTokenizer::new(
        Default::default(),
        |input, _| Ok(Some(input.token(2))),
    ))];
    let peek_loop = [Tokenizer::External(ExternalTokenizer::new(
        Default::default(),
        |input, _| {
            // Even if a callback handles the error itself, parsing cannot resume.
            while input.peek(0).is_ok() {}
            Ok(None)
        },
    ))];
    let probe_loop = [Tokenizer::External(ExternalTokenizer::new(
        Default::default(),
        |_, stack| {
            stack.can_shift(2)?;
            Ok(None)
        },
    ))];
    for (slots, high, goto) in [
        (&ordinary[..], 0, EMPTY.goto),
        (&peek_loop[..], 0, EMPTY.goto),
        (&probe_loop[..], 1, &encode(&[3, 1, 1, 4, 3, 0, 0])),
    ] {
        let parser = Parser::deserialize_with_hooks(
            GrammarSpec {
                tables: SerializedTables {
                    states: &encode(&[2, 2, 0, 1, 0, 0]),
                    state_data: &encode(&[
                        65535,
                        0,
                        2,
                        if high == 0 { 0 } else { 2 },
                        high,
                        65535,
                        0,
                    ]),
                    goto,
                    ..EMPTY
                },
                node_names: "⚠ Main A",
                max_term: 3,
                repeat_node_count: 0,
                tokenizers: slots,
                top_rule: [0, 1],
                token_prec: 0,
            },
            &[],
        )
        .unwrap();
        assert_eq!(
            parser.parse("").unwrap_err(),
            ParseError::LimitExceeded { offset: 0 }
        );
        assert_eq!(
            parser.parse_glr("").unwrap_err(),
            ParseError::LimitExceeded { offset: 0 }
        );
        assert_eq!(
            parser.parse_recovering("").unwrap_err(),
            ParseError::LimitExceeded { offset: 0 }
        );
    }
}

#[test]
fn bounds_dfa_cycles_at_eof_and_repeated_long_lookahead() {
    let states = encode(&[2, 2, 0, 1, 0, 0]);
    let state_data = encode(&[65535, 0, 2, 0, 0, 65535, 0]);
    let spec = GrammarSpec {
        node_names: "⚠ Main A",
        max_term: 3,
        tokenizers: &[0],
        ..EMPTY_GRAMMAR
    };
    let tables = SerializedTables {
        states: &states,
        state_data: &state_data,
        token_data: &encode(&[1, 3, 1, 65535, 65535, 0]),
        ..EMPTY
    };
    let parser = Parser::deserialize(GrammarSpec { tables, ..spec }).unwrap();
    assert_eq!(
        parser.parse("").unwrap_err(),
        ParseError::LimitExceeded { offset: 0 }
    );

    // Accept one 'a' but scan to the end before returning it. Without a
    // shared work limit, scanning again at the next byte takes quadratic time.
    let tables = SerializedTables {
        token_data: &encode(&[
            1, 3, 1, 97, 98, 6, 1, 11, 1, 2, 1, 97, 98, 14, 1, 17, 1, 97, 98, 14,
        ]),
        ..tables
    };
    let parser = Parser::deserialize(GrammarSpec { tables, ..spec }).unwrap();
    assert!(matches!(
        parser.parse(&"a".repeat(1024)),
        Err(ParseError::LimitExceeded { .. })
    ));
}
