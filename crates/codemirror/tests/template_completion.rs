use std::time::Duration;

use gpui_editor_codemirror::{
    Change, ChangeSet, EditorSelection, EditorState, Text, Transaction,
    autocomplete::session::{
        CompletionConfig, CompletionItem, CompletionSession, CompletionStatus, SourceId,
        SourceResult,
    },
    language::{
        jinja::{
            JinjaCompletionConfig, JinjaDocument, jinja_completion_source,
            jinja_completion_source_with,
        },
        liquid::{
            LiquidCompletionConfig, LiquidDocument, liquid_completion_source,
            liquid_completion_source_with,
        },
    },
};

fn custom(label: impl Into<String>) -> CompletionItem {
    let mut option = CompletionItem::new(label);
    option.kind = Some("custom".into());
    option.detail = Some("from config".into());
    option.insert = Some("inserted".into());
    option.boost = 7;
    option.commit_characters = Some(vec![';']);
    option
}

fn properties(path: &[&str], position: usize, explicit: bool) -> Vec<CompletionItem> {
    if path[0] == "empty_result" {
        return Vec::new();
    }
    vec![custom(format!("{}@{position}:{explicit}", path.join("/")))]
}

fn signature(result: SourceResult) -> (usize, usize, u32) {
    assert_eq!(result.to, None);
    assert!(result.filter);
    assert!(result.commit_characters.is_empty());
    let mut hash = 2_166_136_261u32;
    for item in result.options.iter() {
        assert!(item.display_label.is_none() && item.sort_text.is_none());
        assert!(item.info.is_none() && item.section.is_none());
        let boosts = item.boost.to_string();
        let commit: String = item.commit_characters.iter().flatten().collect();
        let fields = [
            item.label.as_str(),
            item.kind.as_deref().unwrap(),
            item.insert.as_deref().unwrap_or_default(),
            item.detail.as_deref().unwrap_or_default(),
            &boosts,
            &commit,
        ];
        for byte in fields.join("\0").bytes().chain([b'\n']) {
            hash = (hash ^ u32::from(byte)).wrapping_mul(16_777_619);
        }
    }
    (result.from, result.options.len(), hash)
}

#[test]
fn template_sources_match_pinned_upstream_options_and_ranges() {
    let tags = [custom("custom_tag"), custom("raw")];
    let variables = [custom("custom_var"), custom("empty")];
    let filters = [custom("custom_filter"), custom("abs")];
    let jinja = jinja_completion_source_with(JinjaCompletionConfig {
        tags: &tags,
        variables: &variables,
        properties: Some(&|path, _, at, explicit| properties(path, at, explicit)),
    });
    let liquid = liquid_completion_source_with(LiquidCompletionConfig {
        tags: &tags,
        variables: &variables,
        filters: &filters,
        properties: Some(&|path, _, at, explicit| properties(path, at, explicit)),
    });
    let mut failures = Vec::new();
    for case in include_str!("fixtures/template_completion.tsv").lines() {
        if case.starts_with('#') || case.is_empty() {
            continue;
        }
        let cols: Vec<_> = case.split('\t').collect();
        let [language, mode, explicit, input, from, len, hash] = cols[..] else {
            panic!("invalid oracle case: {case}");
        };
        let input = input.replace("\\n", "\n").replace("\\r", "\r");
        let position = input.find('§').unwrap();
        let text = Text::new(input.replace('§', ""));
        let explicit = explicit == "1";
        let result = match (language, mode) {
            ("jinja", "default") => {
                jinja_completion_source(&JinjaDocument::parse(text), position, explicit)
            }
            ("jinja", "custom") => jinja(&JinjaDocument::parse(text), position, explicit),
            ("liquid", "default") => {
                liquid_completion_source(&LiquidDocument::parse(text), position, explicit)
            }
            ("liquid", "custom") => liquid(&LiquidDocument::parse(text), position, explicit),
            _ => panic!("invalid oracle case: {case}"),
        };
        let expected = (from != "-").then(|| {
            (
                from.parse().unwrap(),
                len.parse().unwrap(),
                hash.parse().unwrap(),
            )
        });
        let actual = result.map(signature);
        if actual != expected {
            failures.push(format!("{case}: actual {actual:?}, expected {expected:?}"));
        }
    }
    assert!(failures.is_empty(), "{}", failures.join("\n"));
}

#[test]
fn rejects_positions_outside_utf8_boundaries() {
    let source = "{{ 😀 }}";
    let jinja = JinjaDocument::parse(Text::new(source));
    let liquid = LiquidDocument::parse(Text::new(source));
    for position in [4, 5, 6, source.len() + 1] {
        assert!(jinja_completion_source(&jinja, position, true).is_none());
        assert!(liquid_completion_source(&liquid, position, true).is_none());
    }
}

#[test]
fn completion_session_reuses_results_for_words_only() {
    let tags = [custom("é😀_tag")];
    for jinja in [true, false] {
        let text = Text::new("{% ");
        let mut state = EditorState::new(text.as_str());
        let mut at = text.len();
        state
            .update(Transaction::default().with_selection(EditorSelection::single(at, at)))
            .unwrap();
        let mut session =
            CompletionSession::new(&state, CompletionConfig::default(), [SourceId(1)]);
        session.start(true, Duration::ZERO);
        let now = session.deadline().unwrap();
        let [query] = session.take_queries(now).try_into().unwrap();
        let result = if jinja {
            jinja_completion_source_with(JinjaCompletionConfig {
                tags: &tags,
                ..Default::default()
            })(&JinjaDocument::parse(text), at, true)
        } else {
            liquid_completion_source_with(LiquidCompletionConfig {
                tags: &tags,
                ..Default::default()
            })(&LiquidDocument::parse(text), at, true)
        };
        assert!(session.finish_query(&query, result, now).unwrap());
        assert_eq!(session.status(), CompletionStatus::Active);

        for (insert, expected) in [
            ("é😀_", CompletionStatus::Active),
            ("-", CompletionStatus::Pending),
        ] {
            let end = at + insert.len();
            let changes = ChangeSet::single(Change::new(at..at, insert)).unwrap();
            let desc = changes.desc(state.text().len()).unwrap();
            state
                .update(
                    Transaction::new(changes)
                        .with_selection(EditorSelection::single(end, end))
                        .with_user_event("input.type"),
                )
                .unwrap();
            session
                .update(&state, &desc, true, Some("input.type"), now)
                .unwrap();
            assert_eq!(session.status(), expected, "after inserting {insert:?}");
            at = end;
        }
    }
}
