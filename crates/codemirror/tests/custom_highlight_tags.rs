use gpui_editor_codemirror::{
    EditorState,
    language::{
        Color, CustomTagModifier, HighlightProperties, HighlightRule, HighlightSelector,
        HighlightStyle, HighlightTag, TagModifier, wast::style_tags,
    },
    language_data::{attached_language_snapshot, wast_language},
    theme_one_dark::{COLOR, ONE_DARK_HIGHLIGHT_STYLE, highlight_style},
};
use serde_json::{Value, json};

fn foreground<'a>(selector: &'a HighlightSelector<'a>, value: u32) -> HighlightRule<'a> {
    HighlightRule::new(
        std::slice::from_ref(selector),
        HighlightProperties {
            foreground: Some(Color::from_rgba32((value << 8) | 0xff)),
            ..Default::default()
        },
    )
}

#[test]
fn custom_definitions_use_pinned_identity_and_ancestry() {
    let isolated = HighlightTag::define("number");
    let numeric = HighlightTag::derive("same", &HighlightTag::new("number")).unwrap();
    let keyword = HighlightTag::derive("same", &HighlightTag::new("keyword")).unwrap();
    let child = HighlightTag::derive("child", &numeric).unwrap();
    let call = HighlightTag::derive("call", &HighlightTag::new("variableName")).unwrap();
    let nested_call = HighlightTag::derive("call", &call).unwrap();
    assert_ne!(numeric, keyword);
    assert_eq!(numeric, numeric.clone());
    assert_ne!(isolated, HighlightTag::new("number"));

    let numeric_selector = HighlightSelector::from(&numeric);
    let keyword_selector = HighlightSelector::from(&keyword);
    let modified = numeric_selector.with_modifier(TagModifier::Function);
    let child_selector = HighlightSelector::from(&child);
    let direct = [
        foreground(&numeric_selector, 0x123456),
        foreground(&keyword_selector, 0x654321),
    ];
    let modifiers = [
        foreground(&modified, 0x112233),
        foreground(&numeric_selector, 0x445566),
    ];
    let specificity = [
        foreground(&child_selector, 0x334455),
        foreground(&modified, 0x667788),
    ];
    let direct = HighlightStyle::define(&direct);
    let modifiers = HighlightStyle::define(&modifiers);
    let specificity = HighlightStyle::define(&specificity);

    let pin = include_str!("fixtures/custom_highlight_tags.jsonl");
    for row in pin.lines() {
        let row: Value = serde_json::from_str(row).unwrap();
        let name = row["name"].as_str().unwrap();
        let stock = ONE_DARK_HIGHLIGHT_STYLE;
        let (tag, style) = match name {
            "isolated name" => (isolated.clone(), stock),
            "standard parent" => (numeric.clone(), stock),
            "standard ancestor" => (child.clone(), stock),
            "same name other parent" => (keyword.clone(), stock),
            "modified standard ancestor" => (
                nested_call.clone().with_modifier(TagModifier::Function),
                stock,
            ),
            "multiple modifiers" => (
                nested_call
                    .clone()
                    .with_modifier(TagModifier::Function)
                    .with_modifier(TagModifier::Definition),
                stock,
            ),
            "concrete identity" => (numeric.clone(), direct),
            "distinct same name identity" => (keyword.clone(), direct),
            "custom parent identity" => (child.clone(), direct),
            "independent same name" => (HighlightTag::define("same"), direct),
            "modified custom parent" => (
                child.clone().with_modifier(TagModifier::Function),
                modifiers,
            ),
            "unmodified custom parent" => (child.clone(), modifiers),
            "child unmodified first" => (
                child.clone().with_modifier(TagModifier::Function),
                specificity,
            ),
            "modified parent rejected" => {
                let modified = numeric.clone().with_modifier(TagModifier::Function);
                assert_eq!(
                    json!(HighlightTag::derive("invalid", &modified).is_none()),
                    row["rejected"]
                );
                continue;
            }
            _ => panic!("unknown fixture row {name}"),
        };
        let props = style.resolve(&[tag], None);
        let rgb = props
            .foreground
            .map(|color| format!("{:06x}", color.rgba32() >> 8));
        assert_eq!(json!(rgb), row["rgb"], "{name}");
    }
}

#[test]
fn custom_modifiers_match_pinned_identity_and_subset_order() {
    let number = HighlightTag::new("number");
    let root = HighlightTag::derive("root", &number).unwrap();
    let leaf = HighlightTag::derive("leaf", &root).unwrap();
    let a = CustomTagModifier::define("same");
    let b = CustomTagModifier::define("same");
    let c = CustomTagModifier::define("c");
    let d = CustomTagModifier::define("d");
    assert_eq!(a.name(), b.name());
    let tag = |value: &Value| {
        let mut tag = match value[0].as_str().unwrap() {
            "root" => root.clone(),
            "leaf" => leaf.clone(),
            "number" => number.clone(),
            base => panic!("unknown tag base {base}"),
        };
        for name in value[1].as_array().unwrap() {
            tag = match name.as_str().unwrap() {
                "a" => tag.with_custom_modifier(&a),
                "b" => tag.with_custom_modifier(&b),
                "c" => tag.with_custom_modifier(&c),
                "d" => tag.with_custom_modifier(&d),
                "function" => tag.with_modifier(TagModifier::Function),
                "definition" => tag.with_modifier(TagModifier::Definition),
                name => panic!("unknown modifier {name}"),
            };
        }
        tag
    };
    for line in include_str!("fixtures/custom_highlight_modifiers.jsonl").lines() {
        let row: Value = serde_json::from_str(line).unwrap();
        if let Some(name) = row["name"].as_str() {
            let tags: Vec<_> = row["rules"].as_array().unwrap().iter().map(tag).collect();
            let selectors: Vec<_> = tags.iter().map(HighlightSelector::from).collect();
            let rules: Vec<_> = selectors
                .iter()
                .zip([0x123456, 0x654321])
                .map(|(selector, color)| foreground(selector, color))
                .collect();
            let rgb = HighlightStyle::define(&rules)
                .resolve(&[tag(&row["query"])], None)
                .foreground
                .map(|color| format!("{:06x}", color.rgba32() >> 8));
            assert_eq!(json!(rgb), row["rgb"], "{name}");
        } else {
            let ma = root.clone().with_custom_modifier(&a);
            let mb = root.clone().with_custom_modifier(&b);
            assert_eq!(
                json!(ma.clone().with_custom_modifier(&a.clone()) == ma),
                row["idempotent"]
            );
            assert_eq!(
                json!(ma.clone().with_custom_modifier(&b) == mb.clone().with_custom_modifier(&a)),
                row["commutes"]
            );
            assert_eq!(json!(ma != mb), row["distinct"]);
            assert_eq!(
                json!(HighlightTag::derive("invalid", &ma).is_none()),
                row["modifiedParentRejected"]
            );
        }
    }
}

#[test]
fn large_modifier_sets_select_a_subset_without_a_power_set() {
    let modifiers: Vec<_> = (0..96)
        .map(|index| CustomTagModifier::define(format!("m{index}")))
        .collect();
    let base = HighlightTag::new("number");
    let query = modifiers.iter().rev().fold(base.clone(), |tag, modifier| {
        tag.with_custom_modifier(modifier)
    });
    assert_eq!(query.custom_modifiers(), modifiers);
    let choices = [
        base.clone()
            .with_custom_modifier(&modifiers[46])
            .with_custom_modifier(&modifiers[80]),
        base.with_custom_modifier(&modifiers[95])
            .with_custom_modifier(&modifiers[0]),
    ];
    let selectors = choices.each_ref().map(HighlightSelector::from);
    let rules = [
        foreground(&selectors[0], 0x123456),
        foreground(&selectors[1], 0x654321),
    ];
    let color = HighlightStyle::define(&rules)
        .resolve(&[query], None)
        .foreground
        .unwrap();
    assert_eq!(color.rgba32(), 0x123456ff);
}

#[test]
fn custom_wast_tags_keep_definition_identity_in_published_spans() {
    let custom = CustomTagModifier::define("hot");
    let token = HighlightTag::derive("instruction", &HighlightTag::new("number"))
        .unwrap()
        .with_custom_modifier(&custom);
    let props = style_tags([("Keyword", vec![token.clone()])]).unwrap();
    let language = wast_language().with_node_props(&[props]);
    let state = EditorState::with_extensions("(func)", language.extension()).unwrap();
    let doc = attached_language_snapshot(&state).unwrap();
    let spans = doc.highlights(1..5).unwrap();
    assert_eq!(spans.len(), 1);
    assert_eq!(spans[0].range, 1..5);
    assert_eq!(spans[0].tags, std::slice::from_ref(&token));
    assert_eq!(
        highlight_style(spans[0].tags).foreground,
        Some(COLOR.chalky)
    );
    let undeclared = CustomTagModifier::define("hot");
    let exact = HighlightSelector::from(&token);
    let exact_rules = [foreground(&exact, 0x654321)];
    assert_eq!(
        HighlightStyle::define(&exact_rules)
            .resolve(spans[0].tags, None)
            .foreground,
        Some(Color::from_rgba32(0x654321ff))
    );
    let other_number = HighlightTag::new("number").with_custom_modifier(&undeclared);
    let by_name = HighlightSelector::new("instruction");
    let by_other_modifier = HighlightSelector::from(&other_number);
    let rules = [
        foreground(&by_name, 0x123456),
        foreground(&by_other_modifier, 0x654321),
    ];
    assert_eq!(
        HighlightStyle::define(&rules)
            .resolve(spans[0].tags, None)
            .foreground,
        None
    );
}
