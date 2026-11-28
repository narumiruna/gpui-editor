use gpui_editor_codemirror::{
    ALLOW_MULTIPLE_SELECTIONS, Assoc, Change, ChangeDesc, ChangeSet, ChangeSpec,
    ConfigurationError, EditError, EditorSelection, EditorState, Extension, History,
    LINE_SEPARATOR, MapMode, RangeTouch, SelectionRange, SerializationError, StateField, Text,
    Transaction,
};
use serde_json::{Value, json};

#[test]
fn utf16_wire_matches_pinned_changes_and_selections_at_scalar_boundaries() {
    for line in include_str!("fixtures/change_utf16_wire.jsonl").lines() {
        let row: Value = serde_json::from_str(line).unwrap();
        let name = &row["name"];
        let source = Text::new(row["source"].as_str().unwrap());
        let selection = EditorSelection::from_utf16_json(&row["selected"], &source).unwrap();
        assert_eq!(selection.to_json(), row["selectionBytes"], "{name}");
        assert_eq!(
            selection.to_utf16_json(&source).unwrap(),
            row["selected"],
            "{name}"
        );
        let input = row.get("input").unwrap_or(&row["wire"]);
        let set = ChangeSet::from_utf16_json(input, &source).unwrap();
        assert_eq!(set.to_utf16_json(&source).unwrap(), row["wire"], "{name}");
        let mut target = source.clone();
        set.apply_to(&mut target).unwrap();
        assert_eq!(target.as_str(), row["result"], "{name}");
        let geometry = set.desc(source.len()).unwrap();
        assert_eq!(
            geometry.to_utf16_json(&source, &target).unwrap(),
            row["desc"],
            "{name}"
        );
        let decoded = ChangeDesc::from_utf16_json(&row["desc"], &source, &target).unwrap();
        assert_eq!(decoded, geometry, "{name}");
        let ranges: Vec<_> = decoded
            .changed_ranges()
            .map(|(a, b)| [a.start, a.end, b.start, b.end])
            .collect();
        assert_eq!(json!(ranges), row["geometry"], "{name}");
        for probe in row["positions"].as_array().unwrap() {
            let pos = usize::try_from(probe[0].as_u64().unwrap()).unwrap();
            let side = if probe[1] == -1 {
                Assoc::Before
            } else {
                Assoc::After
            };
            let mode = match probe[2].as_str().unwrap() {
                "simple" => MapMode::Simple,
                "delete" => MapMode::TrackDel,
                "before" => MapMode::TrackBefore,
                "after" => MapMode::TrackAfter,
                other => panic!("unknown map mode: {other}"),
            };
            assert_eq!(
                json!(decoded.map_pos(pos, side, mode)),
                probe[3],
                "{name}: {probe}"
            );
            assert_eq!(
                json!(set.map_pos_mode(pos, side, mode)),
                probe[3],
                "{name}: {probe}"
            );
        }
        if let Some(transforms) = row.get("transforms") {
            check_marker_transforms(&set, &source, &target, transforms);
        }
        let inverse = set.inverted(&source).unwrap();
        assert_eq!(
            inverse.to_utf16_json(&target).unwrap(),
            row["inverse"],
            "{name}"
        );
        inverse.apply_to(&mut target).unwrap();
        assert_eq!(target, source, "{name}");
    }
}

fn check_marker_transforms(set: &ChangeSet, source: &Text, target: &Text, expected: &Value) {
    let length = source.len();
    let identity = ChangeSet::default();
    let over = identity.desc(length).unwrap();
    let edit = ChangeSet::from_utf16_json(&expected["edit"], target).unwrap();
    let wrong = identity.desc(length + 1).unwrap();
    for (name, result) in [
        ("map", set.map(&over, false)),
        ("mapDesc", set.map_desc(&over, false)),
        ("composeBefore", identity.compose(set, length)),
        ("composeAfter", set.compose(&identity, length)),
        ("composeEdit", set.compose(&edit, length)),
    ] {
        let result = result.unwrap();
        assert_eq!(result.to_utf16_json(source).unwrap(), expected[name]);
        assert_eq!(result.desc(length + 1), Err(EditError::MismatchedLength));
    }
    assert_eq!(set.map(&wrong, false), Err(EditError::MismatchedLength));
    assert_eq!(
        set.map_desc(&wrong, false),
        Err(EditError::MismatchedLength)
    );
    assert_eq!(
        set.compose(&identity, length + 1),
        Err(EditError::MismatchedLength)
    );
    assert_eq!(set.compose(set, length), Err(EditError::MismatchedLength));
}

#[test]
fn utf16_wire_rejects_split_scalars_wrong_lengths_and_unequal_gaps() {
    let split_scalar = SerializationError::Edit(EditError::InvalidRange);
    let length = SerializationError::Edit(EditError::MismatchedLength);
    let invalid = SerializationError::InvalidFormat("change description");
    let source = Text::new("😀z");
    let selection = |anchor, head| json!({"ranges":[{"anchor":anchor,"head":head}], "main":0});
    assert_eq!(
        EditorSelection::from_utf16_json(&selection(0, 1), &source),
        Err(split_scalar)
    );
    assert_eq!(
        EditorSelection::from_utf16_json(&selection(4, 0), &source),
        Err(length)
    );
    assert_eq!(
        EditorSelection::single(0, 1).to_utf16_json(&source),
        Err(split_scalar)
    );
    assert_eq!(
        ChangeSet::from_utf16_json(&json!([1, [1, "x"], 1]), &source),
        Err(split_scalar)
    );
    assert_eq!(
        ChangeSet::from_utf16_json(&json!([1, [0], 2]), &source),
        Err(split_scalar)
    );
    assert_eq!(
        ChangeSet::from_utf16_json(&json!([3, 0]), &source),
        Err(SerializationError::InvalidFormat("change set"))
    );
    assert_eq!(
        ChangeSet::from_utf16_json(&json!([4]), &source),
        Err(length)
    );
    assert_eq!(
        ChangeSet::from_utf16_json(&json!([2]), &source),
        Err(length)
    );
    assert!(ChangeSet::from_json(&json!([3]), &source).is_err());
    assert_eq!(
        ChangeDesc::from_utf16_json(&json!([1, -1, 2, -1]), &source, &source),
        Err(split_scalar)
    );
    assert_eq!(
        ChangeDesc::from_utf16_json(&json!([3, -1, 0, -1]), &source, &source),
        Err(invalid)
    );
    let unchanged = json!([3, -1]);
    let altered = Text::new("🦀z");
    assert_eq!(
        ChangeDesc::from_utf16_json(&unchanged, &source, &altered),
        Err(invalid)
    );
    let geometry = ChangeDesc::from_utf16_json(&unchanged, &source, &source).unwrap();
    assert_eq!(geometry.to_utf16_json(&source, &altered), Err(invalid));
    assert_eq!(
        geometry.to_utf16_json(&source, &Text::new("z")),
        Err(length)
    );
    let altered = Text::new("¨z   ");
    assert_eq!(geometry.to_utf16_json(&source, &altered), Err(length));
    let partial = ChangeDesc::from_json(&json!([1, -1, 4, -1])).unwrap();
    assert_eq!(partial.to_utf16_json(&source, &source), Err(split_scalar));
}

#[test]
fn utf16_state_json_matches_pinned_separator_selection_and_field_restore() {
    for line in include_str!("fixtures/state_utf16_wire.jsonl").lines() {
        let row: Value = serde_json::from_str(line).unwrap();
        let name = row["name"].as_str().unwrap();
        let field = StateField::new(
            |_| Err(ConfigurationError::InvalidFieldData),
            |value: &Value, _| Ok(value.clone()),
        );
        let selection = row["nativeSelection"].clone();
        let text = row["nativeText"].as_str().unwrap().to_owned();
        let codec = field.json(
            "seen",
            |value, _| Ok(value.clone()),
            move |value, state| {
                assert_eq!(state.selection().to_json(), selection);
                assert_eq!(state.text().as_str(), text);
                Ok(value.clone())
            },
        );
        let mut extensions = Vec::new();
        if let Some(separator) = row["separator"].as_str() {
            extensions.push(LINE_SEPARATOR.of(separator.to_owned()));
        }
        if row["multiple"] == true {
            extensions.push(ALLOW_MULTIPLE_SELECTIONS.of(true));
        }
        let codecs = [codec];
        let state = EditorState::from_utf16_json_with_fields(
            &row["value"],
            Extension::group(extensions),
            &codecs,
        )
        .unwrap_or_else(|error| panic!("{name}: {error}"));
        assert_eq!(state.text().as_str(), row["nativeText"], "{name}");
        assert_eq!(
            state.selection().to_json(),
            row["nativeSelection"],
            "{name}"
        );
        assert_eq!(
            state.to_utf16_json_with_fields(&codecs).unwrap(),
            row["output"],
            "{name}"
        );
    }
}

#[test]
fn utf16_state_json_projects_raw_line_breaks_but_rejects_interior_endpoints() {
    let mut state = EditorState::new("a\r\n😀z");
    let select =
        |anchor, head| Transaction::default().with_selection(EditorSelection::single(anchor, head));
    state.update(select(7, 3)).unwrap();
    let encoded = json!({"doc":"a\n😀z","selection":{"ranges":[{"anchor":4,"head":2}],"main":0}});
    assert_eq!(state.to_utf16_json().unwrap(), encoded);
    let restored = EditorState::from_utf16_json(&encoded).unwrap();
    assert_eq!(restored.text().as_str(), "a\n😀z");
    assert_eq!(restored.selection().main(), SelectionRange::new(6, 2));
    assert_eq!(restored.to_utf16_json().unwrap(), encoded);
    state.update(select(2, 2)).unwrap();
    let split = SerializationError::Edit(EditError::InvalidRange);
    assert_eq!(state.to_utf16_json(), Err(split));
    let mut invalid = encoded.clone();
    invalid["selection"]["ranges"][0]["head"] = json!(3);
    assert_eq!(EditorState::from_utf16_json(&invalid).unwrap_err(), split);
    let unmatched_lf =
        json!({"doc":"a\nb||c","selection":{"ranges":[{"anchor":2,"head":2}],"main":0}});
    assert_eq!(
        EditorState::from_utf16_json_with_fields(
            &unmatched_lf,
            LINE_SEPARATOR.of("||".into()),
            &[]
        )
        .unwrap_err(),
        SerializationError::InvalidFormat("editor state")
    );
    let field = StateField::new(|_| Ok(0), |value, _| Ok(*value));
    let reserved = field.json("doc", |v, _| Ok(json!(v)), |_, _| Ok(0));
    assert_eq!(
        EditorState::from_utf16_json_with_fields(&encoded, Extension::default(), &[reserved])
            .unwrap_err(),
        SerializationError::InvalidFormat("state field names")
    );
}

#[test]
fn pinned_state_field_json_round_trip_uses_configured_dependencies() {
    let dep = StateField::new(|_| Ok(3), |n, _| Ok(*n));
    let number = StateField::new(|_| Ok(0), |n, _| Ok(*n));
    let absent = StateField::new(|_| Ok(0), |n, _| Ok(*n));
    let fields = [
        number.json(
            "n",
            {
                let dep = dep.clone();
                move |v, state| {
                    let base = state.field(&dep)?.ok_or(ConfigurationError::MissingField)?;
                    Ok(json!(v + *base))
                }
            },
            {
                let dep = dep.clone();
                move |value, state| {
                    let n = value.as_i64().ok_or(ConfigurationError::InvalidFieldData)?;
                    let base = state.field(&dep)?.ok_or(ConfigurationError::MissingField)?;
                    assert_eq!(state.selection().main(), SelectionRange::new(7, 5));
                    Ok(n - *base)
                }
            },
        ),
        absent.json("absent", |_, _| Ok(json!(false)), |_, _| Ok(0)),
    ];
    let wire =
        json!({"doc":"abc\nxyz", "selection":{"ranges":[{"anchor":7,"head":5}], "main":0}, "n":18});
    let mut original = EditorState::with_extensions(
        "abc\nxyz",
        Extension::group([dep.extension(), number.init(|_| Ok(15))]),
    )
    .unwrap();
    original
        .update(Transaction::default().with_selection(EditorSelection::single(7, 5)))
        .unwrap();
    assert_eq!(original.to_json_with_fields(&fields).unwrap(), wire);
    let restored = EditorState::from_json_with_fields(&wire, dep.extension(), &fields).unwrap();
    assert_eq!(*restored.context().field(&number).unwrap().unwrap(), 15);
    assert_eq!(restored.to_json_with_fields(&fields).unwrap(), wire);
}

#[test]
fn field_json_names_must_not_override_document_or_selection() {
    let field = StateField::new(|_| Ok(0), |n, _| Ok(*n));
    let make = |name| field.json(name, |n, _| Ok(json!(n)), |_, _| Ok(0));
    let state = EditorState::with_extensions("abc", field.extension()).unwrap();
    for fields in [
        vec![make("doc")],
        vec![make("selection")],
        vec![make("n"), make("n")],
    ] {
        let error = SerializationError::InvalidFormat("state field names");
        assert_eq!(state.to_json_with_fields(&fields), Err(error));
        assert_eq!(
            EditorState::from_json_with_fields(&state.to_json(), field.extension(), &fields)
                .unwrap_err(),
            error
        );
    }
}

#[test]
fn invalid_or_absent_field_json_is_handled_before_returning_the_state() {
    let field = StateField::new(|_| Ok(9), |n, _| Ok(*n));
    let codecs = [field.json(
        "n",
        |n, _| Ok(json!(n)),
        |value, _| value.as_i64().ok_or(ConfigurationError::InvalidFieldData),
    )];
    let mut wire = EditorState::new("字\r\n").to_json();
    let default = EditorState::from_json_with_fields(&wire, field.extension(), &codecs).unwrap();
    assert_eq!(*default.context().field(&field).unwrap().unwrap(), 9);
    wire["n"] = json!("invalid");
    assert_eq!(
        EditorState::from_json_with_fields(&wire, field.extension(), &codecs).unwrap_err(),
        SerializationError::Edit(EditError::Configuration(
            ConfigurationError::InvalidFieldData
        ))
    );
}

#[test]
fn changes_match_codemirror_6_7_6_for_ascii_lf() {
    // Captured separately from ChangeSet.of(spec, 6).toJSON() and set.desc.toJSON().
    // Specs: replace [1, 2) with "XY\nZ\n", delete [3, 5), insert "!" at 6.
    let wire = json!([1, [1, "XY", "Z", ""], 1, [2], 1, [0, "!"]]);
    let geometry = json!([1, -1, 1, 5, 1, -1, 2, 0, 1, -1, 0, 1]);
    let source = Text::new("abcdef");
    let changes = ChangeSet::new(vec![
        Change::new(1..2, "XY\nZ\n"),
        Change::new(3..5, ""),
        Change::new(6..6, "!"),
    ])
    .unwrap();
    assert_eq!(changes.to_json(source.len()).unwrap(), wire);
    assert_eq!(changes.desc(source.len()).unwrap().to_json(), geometry);

    let decoded = ChangeSet::from_json(&wire, &source).unwrap();
    assert!(decoded.changes().eq(changes.changes()));
    let mut result = source.clone();
    decoded.apply_to(&mut result).unwrap();
    assert_eq!(result.as_str(), "aXY\nZ\ncf!");
    let desc = ChangeDesc::from_json(&geometry).unwrap();
    assert_eq!(desc.length(), 6);
    assert_eq!(desc.new_length(), 9);
    assert_eq!(
        desc.changed_ranges().collect::<Vec<_>>(),
        [(1..2, 1..6), (3..5, 7..7), (6..6, 8..9)]
    );
    assert_eq!(
        desc.gaps().collect::<Vec<_>>(),
        [(0..1, 0..1), (2..3, 6..7), (5..6, 7..8)]
    );
}

#[test]
fn serializes_empty_and_unchanged_documents() {
    // ChangeSet.empty(0) and ChangeSet.empty(3), including their descriptions.
    for (doc, wire, desc) in [
        ("", json!([]), json!([])),
        ("abc", json!([3]), json!([3, -1])),
    ] {
        let text = Text::new(doc);
        assert_eq!(ChangeSet::default().to_json(text.len()).unwrap(), wire);
        let set = ChangeSet::from_json(&wire, &text).unwrap();
        assert!(set.is_empty());
        assert_eq!(
            set.to_json(text.len() + 1),
            Err(EditError::MismatchedLength.into())
        );
        let decoded = ChangeDesc::from_json(&desc).unwrap();
        assert_eq!(decoded.length(), text.len());
        assert_eq!(decoded.new_length(), text.len());
        assert!(decoded.is_empty());
        assert_eq!(decoded.to_json(), desc);
    }
}

#[test]
fn preserves_individual_changes_that_share_a_boundary() {
    let mut source = Text::new("a");
    let inserts = ChangeSet::new(vec![Change::new(0..0, "x"), Change::new(0..0, "y")]).unwrap();
    let inverse = inserts.inverted(&source).unwrap();
    let wire = json!([[0, "x"], [0, "y"], 1]);
    assert_eq!(inserts.to_json(1).unwrap(), wire);
    let decoded = ChangeSet::from_json(&wire, &source).unwrap();
    assert!(decoded.changes().eq(inserts.changes()));
    assert_eq!(decoded.to_json(source.len()).unwrap(), wire);
    decoded.apply_to(&mut source).unwrap();
    assert_eq!(source.as_str(), "xya");

    let wire = json!([[1], [1], 1]);
    assert_eq!(inverse.to_json(source.len()).unwrap(), wire);
    let decoded = ChangeSet::from_json(&wire, &source).unwrap();
    assert!(decoded.changes().eq(inverse.changes()));
    assert_eq!(decoded.to_json(source.len()).unwrap(), wire);
    assert_eq!(
        decoded.map_pos_mode(1, Assoc::Before, MapMode::TrackDel),
        Some(0)
    );
    decoded.apply_to(&mut source).unwrap();
    assert_eq!(source.as_str(), "a");

    // Upstream ChangeDesc.fromJSON preserves adjacent sections verbatim.
    let wire = json!([1, 0, 1, 0, 1, -1]);
    let desc = ChangeDesc::from_json(&wire).unwrap();
    assert_eq!(desc.to_json(), wire);
    assert_eq!(desc.map_pos(1, Assoc::Before, MapMode::TrackDel), Some(0));
}

#[test]
fn selection_and_state_match_codemirror_6_7_6_for_ascii_lf() {
    // Captured from EditorState.create with EditorState.allowMultipleSelections.of(true).
    let wire_selection = json!({"ranges": [
        {"anchor": 1, "head": 1}, {"anchor": 6, "head": 3}, {"anchor": 7, "head": 7}
    ], "main": 1});
    let wire_state = json!({"doc": "ab\ncdef", "selection": wire_selection});
    let expected_selection = EditorSelection::new(
        vec![
            SelectionRange::new(1, 1),
            SelectionRange::new(6, 3),
            SelectionRange::new(7, 7),
        ],
        1,
    )
    .unwrap();
    assert_eq!(expected_selection.to_json(), wire_selection);
    assert_eq!(
        EditorSelection::from_json(&wire_selection).unwrap(),
        expected_selection
    );

    let mut expected_state = EditorState::new("ab\ncdef");
    expected_state
        .update(Transaction::default().with_selection(expected_selection.clone()))
        .unwrap();
    assert_eq!(expected_state.to_json(), wire_state);
    let state = EditorState::from_json(&wire_state).unwrap();
    assert_eq!(state.text(), expected_state.text());
    assert_eq!(state.selection(), &expected_selection);
    assert_eq!(state.selection().main(), SelectionRange::new(6, 3));
}

#[test]
fn native_wire_keeps_utf8_offsets_and_original_line_endings() {
    let original = "é\r\n🦊\rx\n";
    let source = Text::new(original);
    let changes = ChangeSet::single(Change::new(4..8, "Ω\r\nz\ry\n")).unwrap();
    // For the same edit, upstream default line splitting and UTF-16 lengths
    // give [2,[2,"Ω","z","y",""],3] and [2,-1,2,6,3,-1].
    let native = json!([4, [4, "Ω\r", "z\ry", ""], 3]);
    assert_eq!(changes.to_json(source.len()).unwrap(), native);
    assert_eq!(
        changes.desc(source.len()).unwrap().to_json(),
        json!([4, -1, 4, 8, 3, -1])
    );
    let decoded = ChangeSet::from_json(&native, &source).unwrap();
    assert_eq!(decoded.to_json(source.len()).unwrap(), native);
    let mut result = source.clone();
    decoded.apply_to(&mut result).unwrap();
    assert_eq!(result.as_str(), "é\r\nΩ\r\nz\ry\n\rx\n");

    // CR and LF are separate bytes. The offset between them is valid.
    let between = ChangeSet::from_json(&json!([3, [0, "!"], 8]), &source).unwrap();
    let mut result = source.clone();
    between.apply_to(&mut result).unwrap();
    assert_eq!(result.as_str(), "é\r!\n🦊\rx\n");

    let wire = json!({"doc": original, "selection": {"ranges": [
        {"anchor": 3, "head": 3}, {"anchor": 8, "head": 4}, {"anchor": 10, "head": 10}
    ], "main": 1}});
    let state = EditorState::from_json(&wire).unwrap();
    assert_eq!(state.text().as_str().as_bytes(), original.as_bytes());
    assert_eq!(state.selection().main(), SelectionRange::new(8, 4));
    assert_eq!(state.to_json(), wire);
}

#[test]
fn native_wire_preserves_empty_replacement_markers() {
    let mut source = Text::new("");
    let desc = ChangeDesc::from_json(&json!([0, 0])).unwrap();
    assert!(!desc.is_empty());
    assert_eq!(desc.touches_range(0..0), RangeTouch::Touch);
    assert_eq!(desc.touches_range(1..1), RangeTouch::None);
    assert_eq!(desc.inverted().to_json(), json!([0, 0]));
    for wire in [json!([[0]]), json!([[0, ""]])] {
        let set = ChangeSet::from_json(&wire, &source).unwrap();
        assert!(!set.is_empty());
        assert_eq!(set.desc(0).unwrap(), desc);
        assert_eq!(set.to_json(0).unwrap(), json!([[0]]));
        assert_eq!(set.inverted(&source).unwrap(), set);
        set.apply_to(&mut source).unwrap();
        assert_eq!(source.as_str(), "");

        let state = EditorState::new("");
        let spec = state.changes_from_specs(&[ChangeSpec::Set(&set)]).unwrap();
        assert_eq!(spec, set);
        let mut history = History::new(state);
        history
            .update(Transaction::new(set), (), |_| (None, ()))
            .unwrap();
        assert!(!history.can_undo());
    }
    assert!(ChangeSet::single(Change::new(0..0, "")).unwrap().is_empty());
    assert!(ChangeDesc::new(0, vec![(0..0, 0)]).unwrap().is_empty());
}

#[test]
fn rejects_invalid_change_sets_before_application() {
    let text = Text::new("é!");
    for bad in [
        json!(null),
        json!({}),
        json!([-1]),
        json!([3.0]),
        json!([[]]),
        json!([0, 3]),
        json!([3, 0]),
        json!([[3, 1]]),
        json!([[3, "X\nY"]]),
    ] {
        assert!(
            matches!(
                ChangeSet::from_json(&bad, &text),
                Err(SerializationError::InvalidFormat(_))
            ),
            "{bad}"
        );
    }
    for bad in [json!([]), json!([2]), json!([4]), json!([3, [1, "z"]])] {
        assert_eq!(
            ChangeSet::from_json(&bad, &text),
            Err(EditError::MismatchedLength.into()),
            "{bad}"
        );
    }
    for bad in [
        json!([1, 2]),
        json!([[1], 2]),
        json!([1, [0], 2]),
        json!([1, [0, "z"], 2]),
    ] {
        assert_eq!(
            ChangeSet::from_json(&bad, &text),
            Err(EditError::InvalidRange.into()),
            "{bad}"
        );
    }
    assert_eq!(text.as_str(), "é!");
    assert_eq!(
        ChangeSet::single(Change::new(1..4, ""))
            .unwrap()
            .to_json(text.len()),
        Err(EditError::InvalidRange.into())
    );
    assert_eq!(
        ChangeSet::from_json(&json!([3, usize::MAX]), &text),
        Err(SerializationError::LengthOverflow)
    );
}

#[test]
fn decoded_sets_remain_bound_after_composition_mapping_and_state_specs() {
    let source = Text::new("abcd");
    let base = ChangeSet::from_json(&json!([1, 3]), &source).unwrap();
    let empty = ChangeSet::from_json(&json!([4]), &source).unwrap();
    let native = ChangeSet::single(Change::new(0..0, "X")).unwrap();
    let composed = empty.compose(&native, 4).unwrap();
    let edited = Text::new("Xabcd");
    let after = ChangeSet::from_json(&json!([5]), &edited).unwrap();
    let insert = native.desc(4).unwrap();
    let mapped = base.map(&insert, false).unwrap();
    assert_eq!(mapped.to_json(5).unwrap(), json!([5]));
    assert_eq!(mapped.to_json(4), Err(EditError::MismatchedLength.into()));
    assert_eq!(base.map_desc(&insert, false).unwrap(), mapped);
    assert_eq!(native.to_json(5).unwrap(), json!([[0, "X"], 5]));

    let state = EditorState::from_text(source.clone());
    let spec = state.changes_from_specs(&[ChangeSpec::Set(&base)]).unwrap();
    let plain = ChangeSet::default();
    for set in [
        &base,
        &empty,
        &spec,
        &composed,
        &native.compose(&after, 4).unwrap(),
    ] {
        let mut wrong = edited.clone();
        assert_eq!(set.apply_to(&mut wrong), Err(EditError::MismatchedLength));
        assert_eq!(wrong, edited);
        assert_eq!(set.inverted(&edited), Err(EditError::MismatchedLength));
        assert_eq!(set.desc(5), Err(EditError::MismatchedLength));
        assert_eq!(set.to_json(5), Err(EditError::MismatchedLength.into()));
        assert_eq!(set.compose(&plain, 5), Err(EditError::MismatchedLength));
        assert_eq!(
            set.map(&plain.desc(5).unwrap(), false),
            Err(EditError::MismatchedLength)
        );
        assert_eq!(
            set.map_desc(&plain.desc(5).unwrap(), false),
            Err(EditError::MismatchedLength)
        );
    }
    assert_eq!(native.compose(&empty, 4), Err(EditError::MismatchedLength));
    assert_eq!(
        EditorState::from_text(edited).changes_from_specs(&[ChangeSpec::Set(&base)]),
        Err(EditError::MismatchedLength)
    );
    let mut extended = Text::new("Xabcd!");
    assert_eq!(
        mapped.apply_to(&mut extended),
        Err(EditError::MismatchedLength)
    );
    assert_eq!(extended.as_str(), "Xabcd!");
}

#[test]
fn application_rechecks_split_gaps_on_a_same_length_document() {
    let wire = json!([1, 3]);
    let set = ChangeSet::from_json(&wire, &Text::new("abcd")).unwrap();
    let mut source = Text::new("éab");
    assert_eq!(set.apply_to(&mut source), Err(EditError::InvalidRange));
    assert_eq!(source.as_str(), "éab");
    let mut state = EditorState::from_text(source);
    assert_eq!(
        state.changes_from_specs(&[ChangeSpec::Set(&set)]),
        Err(EditError::InvalidRange)
    );
    assert_eq!(
        state.update(Transaction::new(set)),
        Err(EditError::InvalidRange)
    );
    assert_eq!(state.text().as_str(), "éab");
}

#[test]
fn rejects_invalid_and_overflowing_change_descriptions() {
    for bad in [
        json!(null),
        json!([1]),
        json!([-1, 0]),
        json!([1, -2]),
        json!([1.0, -1]),
        json!([1, "2"]),
        json!([0, -1]),
    ] {
        assert!(
            matches!(
                ChangeDesc::from_json(&bad),
                Err(SerializationError::InvalidFormat(_))
            ),
            "{bad}"
        );
    }
    for bad in [json!([usize::MAX, 0, 1, 0]), json!([0, usize::MAX, 0, 1])] {
        assert_eq!(
            ChangeDesc::from_json(&bad),
            Err(SerializationError::LengthOverflow)
        );
    }
    let largest = json!([usize::MAX, -1]);
    assert_eq!(ChangeDesc::from_json(&largest).unwrap().to_json(), largest);
}

#[test]
fn rejects_invalid_selections_and_state_documents() {
    for bad in [
        json!(null),
        json!({"ranges": [], "main": 0}),
        json!({"ranges": [{"anchor": 0, "head": 0}], "main": -1}),
        json!({"ranges": [{"anchor": 0, "head": 0}], "main": 1}),
        json!({"ranges": [{"anchor": 0}], "main": 0}),
        json!({"ranges": [{"anchor": 0, "head": 1.5}], "main": 0}),
        json!({"ranges": [{"anchor": 1, "head": 1}, {"anchor": 0, "head": 0}], "main": 1}),
        json!({"ranges": [{"anchor": 3, "head": 0}, {"anchor": 2, "head": 2}], "main": 1}),
    ] {
        assert!(
            matches!(
                EditorSelection::from_json(&bad),
                Err(SerializationError::InvalidFormat(_))
            ),
            "{bad}"
        );
    }
    for head in [1, 7] {
        let bad = json!({"doc": "é🦊", "selection": {"ranges": [{"anchor": 2, "head": head}], "main": 0}});
        assert!(matches!(
            EditorState::from_json(&bad),
            Err(SerializationError::Edit(EditError::InvalidSelection))
        ));
    }
    for bad in [
        json!({"doc": "abc"}),
        json!({"doc": 123, "selection": EditorSelection::single(0, 0).to_json()}),
    ] {
        assert!(matches!(
            EditorState::from_json(&bad),
            Err(SerializationError::InvalidFormat(_))
        ));
    }
}
