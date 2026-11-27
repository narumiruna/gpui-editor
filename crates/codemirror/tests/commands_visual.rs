//! The fixture records real EditorView scenarios in Google Chrome Headless
//! 158.0.8127.37. The SHA-512 hashes of the downloaded `commands` 6.11.2,
//! `view` 6.44.0, `state` 6.7.6, and `language` 6.13.1 tarballs matched
//! `docs/codemirror-upstream.json`. Calls and raw geometry results come from a
//! second EditorView of the same size, styles, and state, without atomic ranges.
//! The horizontal fixture also uses SHA-512-verified `lang-json` 6.0.2 and
//! `@lezer/json` 1.0.3 for syntax trees, marked brackets, and matched handles.
//! Offsets are UTF-8 byte lengths of `sliceDoc(0, pos)` (including CRLF).
//!
//! The tests replay geometry and check typed scroll requests. They do not test
//! measurement or rendering. Deletion does not retain browser cursor affinity.

use std::{
    cell::Cell,
    ops::Range,
    sync::{Arc, LazyLock},
};

use gpui_editor_codemirror::{
    Assoc, EditError, EditorSelection, EditorState, Extension, PHRASES, READ_ONLY, SelectionRange,
    Transaction,
    commands::visual::{
        self, PageScrollGeometry, TextDirection, VerticalDistance, VisualCursor, VisualDirection,
        VisualGeometry, VisualSelection, VisualSyntax, add_cursor_above, add_cursor_below,
        cursor_line_down, cursor_line_up, delete_line_boundary_backward,
        delete_line_boundary_forward, select_line_down, select_line_up, select_page_down,
        select_page_up,
    },
    language::lezer::{GrammarSpec, Parser, SerializedTables, SyntaxNode, SyntaxTree},
    view::{ANNOUNCE, SCROLL_INTO_VIEW, ScrollStrategy},
    word_characters,
};
use serde_json::{Value, json};

fn number(value: &Value) -> usize {
    value.as_u64().unwrap().try_into().unwrap()
}

fn cursor(value: &Value) -> VisualCursor {
    VisualCursor {
        head: number(&value[0]),
        assoc: match value[1].as_i64().unwrap() {
            -1 => Some(Assoc::Before),
            0 => None,
            1 => Some(Assoc::After),
            assoc => panic!("unexpected affinity: {assoc}"),
        },
        bidi_level: None,
        goal_column: value[2].as_f64(),
    }
}

fn cursors(value: &Value) -> Vec<VisualCursor> {
    value["heads"]
        .as_array()
        .unwrap()
        .iter()
        .map(cursor)
        .collect()
}

fn selection(value: &Value) -> EditorSelection {
    let ranges = value["ranges"]
        .as_array()
        .unwrap()
        .iter()
        .map(|pair| SelectionRange::new(number(&pair[0]), number(&pair[1])))
        .collect();
    EditorSelection::new(ranges, number(&value["main"])).unwrap()
}

fn span(value: &Value) -> Range<usize> {
    number(&value[0])..number(&value[1])
}

struct Replay<'a> {
    name: &'a str,
    calls: &'a [Value],
    used: Cell<usize>,
    atoms: &'a [Vec<Range<usize>>],
    page: &'a Value,
    page_head: usize,
}

impl Replay<'_> {
    fn call(&self, method: &str) -> &Value {
        let name = self.name;
        let index = self.used.get();
        self.used.set(index + 1);
        let call = self
            .calls
            .get(index)
            .unwrap_or_else(|| panic!("{name}: unexpected {method} query {index}"));
        assert_eq!(call["method"], method, "{name}: query {index} method");
        call
    }

    fn motion(&self, method: &str, start: VisualCursor, direction: VisualDirection) -> &Value {
        let call = self.call(method);
        let name = self.name;
        let index = self.used.get() - 1;
        assert_eq!(
            call["forward"],
            direction == VisualDirection::Forward,
            "{name}: query {index} direction"
        );
        assert_eq!(cursor(&call["start"]), start, "{name}: query {index} input");
        call
    }

    fn query(&self, method: &str, start: VisualCursor, direction: VisualDirection) -> VisualCursor {
        cursor(&self.motion(method, start, direction)["result"])
    }
}

impl VisualGeometry for Replay<'_> {
    fn move_vertical(
        &self,
        start: VisualCursor,
        direction: VisualDirection,
        distance: VerticalDistance,
    ) -> VisualCursor {
        let method = match distance {
            VerticalDistance::Line => "vertical",
            VerticalDistance::Page => "page",
        };
        self.query(method, start, direction)
    }

    fn line_boundary(&self, start: VisualCursor, direction: VisualDirection) -> VisualCursor {
        self.query("boundary", start, direction)
    }

    fn text_direction_at(&self, head: usize) -> Option<TextDirection> {
        let call = self.call("direction");
        assert_eq!(number(&call["head"]), head, "{}: basis", self.name);
        Some(if call["ltr"] == true {
            TextDirection::LeftToRight
        } else {
            TextDirection::RightToLeft
        })
    }

    fn move_by_char(
        &self,
        start: VisualCursor,
        direction: VisualDirection,
        accept: Option<&mut dyn FnMut(&str) -> bool>,
    ) -> Option<VisualCursor> {
        let call = self.motion(
            if accept.is_some() { "walk" } else { "char" },
            start,
            direction,
        );
        if let Some(accept) = accept {
            for cluster in call["clusters"].as_array().unwrap() {
                assert_eq!(
                    accept(cluster[0].as_str().unwrap()),
                    cluster[1].as_bool().unwrap(),
                    "{}: group category at {cluster}",
                    self.name
                );
            }
        }
        Some(cursor(&call["result"]))
    }

    fn line_block(&self, head: usize) -> Option<Range<usize>> {
        let call = self.call("lineBlock");
        assert_eq!(number(&call["head"]), head, "{}: block head", self.name);
        Some(span(&call["block"]))
    }

    fn unwrapped_line_boundary(
        &self,
        start: VisualCursor,
        direction: VisualDirection,
    ) -> Option<VisualCursor> {
        Some(self.query("unwrapped", start, direction))
    }

    fn page_scroll_geometry(&self, head: usize) -> Option<PageScrollGeometry> {
        assert_eq!(
            head, self.page_head,
            "{}: old main for page scrolling",
            self.name
        );
        self.page.as_array().map(|coords| PageScrollGeometry {
            scroll_top: coords[0].as_f64().unwrap(),
            scroll_bottom: coords[1].as_f64().unwrap(),
            caret_top: coords[2].as_f64().unwrap(),
            caret_bottom: coords[3].as_f64().unwrap(),
        })
    }

    fn atomic_ranges(&self) -> &[Vec<Range<usize>>] {
        self.atoms
    }
}

fn fixture_atoms(case: &Value) -> Vec<Vec<Range<usize>>> {
    case["atoms"]
        .as_array()
        .into_iter()
        .flatten()
        .map(|set| set.as_array().unwrap().iter().map(span).collect())
        .collect()
}

fn fixture_state(case: &Value) -> EditorState {
    let mut extensions = vec![READ_ONLY.of(case["readOnly"] == true)];
    if let Some(phrase) = case["phrase"].as_str() {
        extensions.push(PHRASES.of(Arc::new(
            [("Selection deleted".to_owned(), phrase.to_owned())].into(),
        )));
    }
    if let Some(chars) = case["wordChars"].as_str() {
        let chars = Arc::<str>::from(chars);
        extensions.push(word_characters(move |_, _| Some(Arc::clone(&chars))));
    }
    let mut state = EditorState::with_extensions(
        case["source"].as_str().unwrap(),
        Extension::group(extensions),
    )
    .unwrap();
    state
        .update(Transaction::default().with_selection(selection(&case["before"])))
        .unwrap();
    state
}

static JSON_PARSER: LazyLock<Parser> = LazyLock::new(|| {
    let grammar: Value =
        serde_json::from_str(include_str!("fixtures/commands_logical_json_grammar.json")).unwrap();
    let text = |key: &str| grammar[key].as_str().unwrap();
    let term = |value: &Value| u16::try_from(number(value)).unwrap();
    let tokenizers: Vec<_> = grammar["tokenizers"]
        .as_array()
        .unwrap()
        .iter()
        .map(|value| number(value).try_into().unwrap())
        .collect();
    Parser::deserialize(GrammarSpec {
        tables: SerializedTables {
            version: number(&grammar["version"]).try_into().unwrap(),
            states: text("states"),
            state_data: text("stateData"),
            goto: text("goto"),
            token_data: text("tokenData"),
        },
        node_names: text("nodeNames"),
        max_term: term(&grammar["maxTerm"]),
        repeat_node_count: term(&grammar["repeatNodeCount"]),
        tokenizers: &tokenizers,
        top_rule: [term(&grammar["topRule"][0]), term(&grammar["topRule"][1])],
        token_prec: number(&grammar["tokenPrec"]),
    })
    .unwrap()
});

struct ReplaySyntax<'a> {
    tree: SyntaxTree,
    brackets: &'a [Value],
}

impl ReplaySyntax<'_> {
    fn marked(&self, node: SyntaxNode<'_>, direction: VisualDirection) -> Option<&Value> {
        self.brackets.iter().find(|value| {
            span(&value["span"]) == node.range()
                && value["forward"] == (direction == VisualDirection::Forward)
        })
    }
}

impl VisualSyntax for ReplaySyntax<'_> {
    fn tree(&self) -> &SyntaxTree {
        &self.tree
    }

    fn is_bracket(&self, node: SyntaxNode<'_>, direction: VisualDirection) -> bool {
        self.marked(node, direction).is_some()
    }

    fn matching_bracket(
        &self,
        node: SyntaxNode<'_>,
        direction: VisualDirection,
    ) -> Option<Range<usize>> {
        let partner = &self.marked(node, direction)?["partner"];
        partner.is_array().then(|| span(partner))
    }
}

#[test]
fn visual_commands_replay_pinned_chrome() {
    for case in include_str!("fixtures/commands_visual.jsonl")
        .lines()
        .chain(include_str!("fixtures/commands_visual_horizontal.jsonl").lines())
    {
        let case: Value = serde_json::from_str(case).unwrap();
        let name = case["name"].as_str().unwrap();
        let mut state = fixture_state(&case);
        let mut visual = VisualSelection::new(state.selection(), cursors(&case["before"])).unwrap();
        let atoms = fixture_atoms(&case);
        let syntax = (case["syntax"] == true).then(|| ReplaySyntax {
            tree: JSON_PARSER.parse(state.text().as_str()).unwrap(),
            brackets: case["brackets"].as_array().unwrap(),
        });
        let syntax = syntax.as_ref().map(|syntax| syntax as &dyn VisualSyntax);
        for step in case["steps"].as_array().unwrap() {
            let calls = step["calls"].as_array().unwrap();
            let geometry = Replay {
                name,
                calls,
                used: Cell::new(0),
                atoms: &atoms,
                page: &step["pageGeometry"],
                page_head: state.selection().main().head(),
            };
            let command = step["command"].as_str().unwrap();
            let (transaction, next_visual) = match command {
                "deleteLineBoundaryBackward" => (
                    delete_line_boundary_backward(&state, &geometry, &visual),
                    None,
                ),
                "deleteLineBoundaryForward" => (
                    delete_line_boundary_forward(&state, &geometry, &visual),
                    None,
                ),
                _ => {
                    let movement = match command {
                        "cursorSyntaxLeft" => {
                            visual::cursor_syntax_left(&state, &geometry, &visual, syntax)
                        }
                        "cursorSyntaxRight" => {
                            visual::cursor_syntax_right(&state, &geometry, &visual, syntax)
                        }
                        "selectSyntaxLeft" => {
                            visual::select_syntax_left(&state, &geometry, &visual, syntax)
                        }
                        "selectSyntaxRight" => {
                            visual::select_syntax_right(&state, &geometry, &visual, syntax)
                        }
                        _ => {
                            let command = match command {
                                "addCursorAbove" => add_cursor_above,
                                "addCursorBelow" => add_cursor_below,
                                "cursorLineUp" => cursor_line_up,
                                "cursorLineDown" => cursor_line_down,
                                "selectLineUp" => select_line_up,
                                "selectLineDown" => select_line_down,
                                "selectPageUp" => select_page_up,
                                "selectPageDown" => select_page_down,
                                "cursorPageUp" => visual::cursor_page_up,
                                "cursorPageDown" => visual::cursor_page_down,
                                "cursorCharLeft" => visual::cursor_char_left,
                                "cursorCharRight" => visual::cursor_char_right,
                                "cursorCharForward" => visual::cursor_char_forward,
                                "cursorCharBackward" => visual::cursor_char_backward,
                                "selectCharLeft" => visual::select_char_left,
                                "selectCharRight" => visual::select_char_right,
                                "selectCharForward" => visual::select_char_forward,
                                "selectCharBackward" => visual::select_char_backward,
                                "cursorGroupLeft" => visual::cursor_group_left,
                                "cursorGroupRight" => visual::cursor_group_right,
                                "cursorGroupForward" => visual::cursor_group_forward,
                                "cursorGroupBackward" => visual::cursor_group_backward,
                                "selectGroupLeft" => visual::select_group_left,
                                "selectGroupRight" => visual::select_group_right,
                                "selectGroupForward" => visual::select_group_forward,
                                "selectGroupBackward" => visual::select_group_backward,
                                "cursorGroupForwardWin" => visual::cursor_group_forward_win,
                                "selectGroupForwardWin" => visual::select_group_forward_win,
                                "cursorLineBoundaryLeft" => visual::cursor_line_boundary_left,
                                "cursorLineBoundaryRight" => visual::cursor_line_boundary_right,
                                "cursorLineBoundaryForward" => visual::cursor_line_boundary_forward,
                                "cursorLineBoundaryBackward" => {
                                    visual::cursor_line_boundary_backward
                                }
                                "selectLineBoundaryLeft" => visual::select_line_boundary_left,
                                "selectLineBoundaryRight" => visual::select_line_boundary_right,
                                "selectLineBoundaryForward" => visual::select_line_boundary_forward,
                                "selectLineBoundaryBackward" => {
                                    visual::select_line_boundary_backward
                                }
                                "cursorLineStart" => visual::cursor_line_start,
                                "cursorLineEnd" => visual::cursor_line_end,
                                "selectLineStart" => visual::select_line_start,
                                "selectLineEnd" => visual::select_line_end,
                                _ => panic!("unknown command: {command}"),
                            };
                            command(&state, &geometry, &visual)
                        }
                    };
                    match movement.unwrap_or_else(|error| panic!("{name}: {command}: {error:?}")) {
                        Some(update) => (Ok(Some(update.transaction)), Some(update.selection)),
                        None => (Ok(None), Some(visual.clone())),
                    }
                }
            };
            let transaction = transaction.unwrap();
            assert_eq!(geometry.used.get(), calls.len(), "{name}: query count");
            assert_eq!(
                transaction.is_some(),
                step["handled"] == true,
                "{name}: handled"
            );
            let targets: Vec<_> = transaction
                .iter()
                .flat_map(|transaction| transaction.effects())
                .filter_map(|effect| effect.get(&SCROLL_INTO_VIEW))
                .map(|target| {
                    json!({
                        "range": [target.range.anchor(),target.range.head()],
                        "x": scroll_strategy(target.x), "y": scroll_strategy(target.y),
                        "xMargin": target.x_margin, "yMargin": target.y_margin,
                    })
                })
                .collect();
            let expected_targets = step["scrollTargets"]
                .as_array()
                .map_or(&[][..], Vec::as_slice);
            // serde_json compares integer and float numbers by representation.
            for (actual, expected) in targets.iter().zip(expected_targets) {
                for key in ["range", "x", "y"] {
                    assert_eq!(actual[key], expected[key], "{name}: scroll {key}");
                }
                for key in ["xMargin", "yMargin"] {
                    assert_eq!(
                        actual[key].as_f64(),
                        expected[key].as_f64(),
                        "{name}: scroll {key}"
                    );
                }
            }
            assert_eq!(
                targets.len(),
                expected_targets.len(),
                "{name}: typed scroll count"
            );
            let events = if let Some(transaction) = transaction {
                assert!(
                    transaction.selection().is_some(),
                    "{name}: explicit selection"
                );
                let events = json!([{
                    "userEvent": transaction.user_event(),
                    "scroll": transaction.scroll_into_view(),
                    "docChanged": !transaction.changes().is_empty(),
                    "announce": transaction.effects().iter().filter_map(|effect| effect.get(&ANNOUNCE)).collect::<Vec<_>>(),
                }]);
                state.update(transaction).unwrap();
                events
            } else {
                json!([])
            };
            assert_eq!(events, step["events"], "{name}: events");
            assert_eq!(
                state.selection(),
                &selection(&step["after"]),
                "{name}: selection"
            );
            if let Some(units) = step["utf16"].as_array() {
                // The browser deletes one UTF-16 unit, leaving a lone low surrogate.
                assert_eq!(name, "delete-at-wrap-edge-before-astral-scalar");
                let units: Vec<u16> = units
                    .iter()
                    .map(|unit| number(unit).try_into().unwrap())
                    .collect();
                assert!(String::from_utf16(&units).is_err());
                assert_eq!(
                    state.text().as_str(),
                    "0123456789 bcdefghij",
                    "{name}: scalar fallback"
                );
            } else {
                let expected = step["text"]
                    .as_str()
                    .unwrap_or_else(|| case["source"].as_str().unwrap());
                assert_eq!(state.text().as_str(), expected, "{name}: content");
            }
            if let Some(next_visual) = next_visual {
                assert_eq!(
                    next_visual.cursors(),
                    cursors(&step["after"]),
                    "{name}: visual metadata"
                );
                visual = next_visual;
            } else {
                visual = VisualSelection::from_selection(state.selection());
            }
        }
    }
}

fn scroll_strategy(strategy: ScrollStrategy) -> &'static str {
    match strategy {
        ScrollStrategy::Nearest => "nearest",
        ScrollStrategy::Start => "start",
        ScrollStrategy::Center => "center",
        ScrollStrategy::End => "end",
    }
}

struct TestGeometry {
    atoms: Vec<Vec<Range<usize>>>,
    project: fn(VisualCursor) -> VisualCursor,
}

impl VisualGeometry for TestGeometry {
    fn move_vertical(
        &self,
        start: VisualCursor,
        _: VisualDirection,
        _: VerticalDistance,
    ) -> VisualCursor {
        (self.project)(start)
    }

    fn line_boundary(&self, start: VisualCursor, _: VisualDirection) -> VisualCursor {
        (self.project)(start)
    }

    fn atomic_ranges(&self) -> &[Vec<Range<usize>>] {
        &self.atoms
    }
}

#[test]
fn rejects_invalid_geometry_or_stale_selection_metadata() {
    let state = EditorState::new("éa\nb");
    let visual = VisualSelection::from_selection(state.selection());
    let mut geometry = TestGeometry {
        atoms: Vec::new(),
        project: |_| VisualCursor::new(1),
    };
    assert_eq!(
        select_page_down(&state, &geometry, &visual).unwrap_err(),
        EditError::InvalidSelection
    );
    assert_eq!(
        delete_line_boundary_forward(&state, &geometry, &visual).unwrap_err(),
        EditError::InvalidSelection
    );

    geometry.atoms.push(std::iter::once(1..3).collect());
    assert_eq!(
        cursor_line_down(&state, &geometry, &visual).unwrap_err(),
        EditError::InvalidRange
    );

    geometry.atoms.clear();
    let stale = VisualSelection::from_selection(&EditorSelection::single(2, 2));
    assert_eq!(
        cursor_line_down(&state, &geometry, &stale).unwrap_err(),
        EditError::InvalidSelection
    );
    let invalid = VisualCursor {
        goal_column: Some(f64::NAN),
        ..VisualCursor::new(0)
    };
    assert_eq!(
        VisualSelection::new(state.selection(), vec![invalid]).unwrap_err(),
        EditError::InvalidSelection
    );
}

#[test]
fn horizontal_motion_does_not_guess_missing_layout() {
    let state = EditorState::new("אב\na");
    let visual = VisualSelection::from_selection(state.selection());
    let geometry = TestGeometry {
        atoms: Vec::new(),
        project: |_| VisualCursor::new(5),
    };
    assert!(
        visual::cursor_char_left(&state, &geometry, &visual)
            .unwrap()
            .is_none()
    );
    assert!(
        visual::cursor_char_forward(&state, &geometry, &visual)
            .unwrap()
            .is_none()
    );
    assert!(
        visual::select_group_backward(&state, &geometry, &visual)
            .unwrap()
            .is_none()
    );
    assert!(
        visual::cursor_line_boundary_forward(&state, &geometry, &visual)
            .unwrap()
            .is_none()
    );
    assert!(
        visual::cursor_line_end(&state, &geometry, &visual)
            .unwrap()
            .is_none()
    );
    let update = visual::cursor_page_down(&state, &geometry, &visual)
        .unwrap()
        .unwrap();
    assert_eq!(update.selection.cursors(), &[VisualCursor::new(5)]);
    assert!(update.transaction.scroll_into_view());
    assert!(update.transaction.effects().is_empty());
}

#[test]
fn add_cursor_rejects_a_cycle_within_one_logical_line() {
    let state = EditorState::new("abcdef\nx");
    let visual = VisualSelection::from_selection(state.selection());
    let geometry = TestGeometry {
        atoms: Vec::new(),
        project: |cursor| VisualCursor::new(if cursor.head == 0 { 2 } else { 0 }),
    };
    assert_eq!(
        add_cursor_below(&state, &geometry, &visual).unwrap_err(),
        EditError::InvalidSelection
    );
}
