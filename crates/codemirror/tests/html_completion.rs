use std::time::Duration;

use gpui_editor_codemirror::{
    Change, ChangeSet, EditorSelection, EditorState, Text, Transaction,
    autocomplete::session::{
        CompletionConfig, CompletionSession, CompletionStatus, SourceId, SourceResult,
    },
    language::html::{
        HtmlAttributeSpec, HtmlCompletionConfig, HtmlDocument, HtmlOptions, HtmlTagSpec,
        html_completion_source, html_completion_source_with,
    },
};

// Captured in scratch Node from the SHA-512-pinned @codemirror/lang-html
// 6.4.12 tarball, with @lezer/html 1.3.13 and CompletionContext 6.20.4.
// Context fixtures select the HTML language data with side -1. Offsets were
// converted from UTF-16 to bytes; the fixture line separator was set to LF.
type Signature = (usize, usize, usize, u32);
const FNV_OFFSET: u32 = 2_166_136_261;
const TAGS: u32 = 713_011_268;
const START_TAGS: u32 = 3_918_049_366;
const GLOBALS: u32 = 794_691_960;
const DIR_VALUES: u32 = 1_456_009_987;

fn fnv(hash: u32, data: &str) -> u32 {
    data.bytes().fold(hash, |hash, byte| {
        (hash ^ u32::from(byte)).wrapping_mul(16_777_619)
    })
}

fn signature(result: &SourceResult) -> Signature {
    assert!(result.filter && result.commit_characters.is_empty());
    let hash = result.options.iter().fold(FNV_OFFSET, |hash, item| {
        assert!(item.display_label.is_none() && item.sort_text.is_none());
        assert!(item.info.is_none() && item.detail.is_none());
        assert!(item.section.is_none() && item.commit_characters.is_none());
        fnv(
            hash,
            &format!(
                "{}\0{}\0{}\0{}\n",
                item.label,
                item.kind.as_deref().unwrap(),
                item.insert.as_deref().unwrap_or_default(),
                item.boost,
            ),
        )
    });
    (result.from, result.to.unwrap(), result.options.len(), hash)
}

fn document_at(input: &str, options: HtmlOptions) -> (HtmlDocument, usize) {
    let (prefix, suffix) = input.split_once('§').unwrap();
    (
        HtmlDocument::parse_with_options(Text::new(format!("{prefix}{suffix}")), options),
        prefix.len(),
    )
}

fn complete(input: &str) -> SourceResult {
    let (doc, pos) = document_at(input, HtmlOptions::default());
    html_completion_source(&doc, pos, false).unwrap_or_else(|| panic!("missing result for {input}"))
}

#[test]
fn matches_every_ordered_default_tag_attribute_and_value() {
    let all_tags = complete("<§");
    let globals = complete("<custom-element §>");
    assert_eq!(signature(&all_tags), (1, 1, 112, TAGS));
    assert_eq!(signature(&globals), (16, 16, 83, GLOBALS));

    let (mut total, mut digest) = (0, FNV_OFFSET);
    let mut record = |result: &SourceResult| {
        let (from, to, count, hash) = signature(result);
        digest = fnv(digest, &format!("{from},{to},{count},{hash}\n"));
        total += 1;
    };
    record(&all_tags);
    record(&globals);
    for tag in &*all_tags.options {
        let attrs = complete(&format!("<{} §>", tag.label));
        record(&attrs);
        for attr in &attrs.options[..attrs.options.len() - globals.options.len()] {
            record(&complete(&format!("<{} {}=§>", tag.label, attr.label)));
        }
    }
    for attr in &*globals.options {
        record(&complete(&format!("<custom-element {}=§>", attr.label)));
    }
    assert_eq!((total, digest), (420, 1_256_786_315));
}

#[test]
fn matches_pinned_html_query_contexts() {
    for (input, explicit_only, expected) in [
        ("§", true, Some((0, 0, 112, START_TAGS))),
        ("a§bc", true, Some((1, 1, 112, START_TAGS))),
        ("<§", false, Some((1, 1, 112, TAGS))),
        ("<§div>", false, Some((1, 1, 112, TAGS))),
        ("<di§v>", false, Some((1, 3, 112, TAGS))),
        ("<div §", false, Some((5, 5, 83, GLOBALS))),
        ("<div cl§ass=\"x\">", false, Some((5, 7, 83, GLOBALS))),
        ("<div class= §>", false, None),
        ("<div class= §", true, Some((12, 12, 112, START_TAGS))),
        ("<div dir=§", false, Some((9, 9, 3, DIR_VALUES))),
        ("<div dir=l§tr>", false, Some((9, 10, 3, DIR_VALUES))),
        ("<div dir='l§tr'>", false, Some((10, 11, 3, 624_104_204))),
        (
            "<div dir=\"lt§r\">",
            false,
            Some((10, 12, 3, 2_368_095_257)),
        ),
        ("<div dir=\"ltr§\">", false, Some((10, 13, 3, 730_045_385))),
        (
            "<div dir=\"ltr\"§>",
            false,
            Some((10, 14, 3, 2_368_095_257)),
        ),
        ("<div dir=\"ltr§", false, Some((10, 13, 3, 2_368_095_257))),
        ("<div dir=\"l&am§p;\">", false, None),
        ("<div DIR=§", false, Some((9, 9, 0, FNV_OFFSET))),
        ("<INPUT type=§", false, Some((12, 12, 0, FNV_OFFSET))),
        ("<script type=§>", false, Some((13, 13, 1, 532_126_421))),
        ("<div>§", true, Some((5, 5, 113, 187_266_404))),
        ("<div>x§</div>", true, Some((6, 6, 112, START_TAGS))),
        ("<div></div>§", true, Some((11, 11, 112, START_TAGS))),
        ("<div><§", false, Some((6, 6, 113, 981_861_798))),
        ("<div></di§v>", false, Some((7, 9, 0, FNV_OFFSET))),
        ("<div></d§", false, Some((7, 8, 1, 3_168_721_071))),
        ("<div></a§    >", false, Some((7, 8, 1, 3_462_241_803))),
        ("<div></a§     >", false, Some((7, 8, 1, 3_168_721_071))),
        ("<div></a§😀😀>", false, Some((7, 8, 1, 3_462_241_803))),
        ("<div><span></sp§", false, Some((13, 15, 2, 2_986_194_746))),
        (
            "<div><span><§</div>",
            false,
            Some((12, 12, 113, 2_704_161_228)),
        ),
        ("<div><span></span>§", true, Some((18, 18, 112, START_TAGS))),
        (
            "<div><div><SPAN></§",
            false,
            Some((18, 18, 2, 3_301_121_594)),
        ),
        ("<DIV><div></§", false, Some((12, 12, 2, 11_332_304))),
        ("<head><§", false, Some((7, 7, 9, 490_363_384))),
        ("<head><m§", false, Some((7, 8, 9, 490_363_384))),
        ("<head> §</head>", true, Some((7, 7, 8, 2_711_457_706))),
        ("<HEAD><§", false, Some((7, 7, 113, 3_714_069_400))),
        ("<ul><§li>", false, Some((5, 5, 6, 1_971_519_073))),
        (
            "<ul><l§i>one<li>two</ul>",
            false,
            Some((5, 6, 5, 4_250_483_241)),
        ),
        ("<ol><§", false, Some((5, 5, 6, 3_668_625_489))),
        ("<div/><§", false, Some((7, 7, 113, 981_861_798))),
        ("<br>§", true, Some((4, 4, 113, 4_165_328_583))),
        ("<img §", false, Some((5, 5, 90, 4_229_500_370))),
        (
            "<svg><circle/><§",
            false,
            Some((15, 15, 113, 2_757_463_366)),
        ),
        ("<d\r\n§", false, Some((4, 4, 83, GLOBALS))),
        ("😀<div dir=§", false, Some((13, 13, 3, DIR_VALUES))),
        (
            "😀<élément><em></§",
            false,
            Some((21, 21, 2, 3_853_104_624)),
        ),
        ("<a\n  <b\n §", false, Some((9, 9, 83, GLOBALS))),
        ("<a><!-- § -->", false, None),
        ("<div>&amp§;", false, None),
        ("<style>§", false, None),
        ("<style>p§ {}</style>", false, None),
        ("<script>§test</script>", false, None),
        ("<script>x§</script>", false, None),
        (
            "<script type=\"custom\">foo§</script>",
            true,
            Some((25, 25, 112, START_TAGS)),
        ),
        (
            "<textarea>x§</textarea>",
            true,
            Some((11, 11, 112, START_TAGS)),
        ),
        ("<div style=\"§col\">", false, None),
        ("<div style=\"col\"§>", false, Some((11, 16, 0, FNV_OFFSET))),
        ("<div style=§color>", false, Some((11, 11, 0, FNV_OFFSET))),
        ("<div style=co§lor>", false, None),
        ("<div onclick=\"do§Thing()\">", false, None),
        ("<div><😀§>", false, Some((6, 10, 113, 1_799_991_120))),
    ] {
        let (doc, pos) = document_at(input, HtmlOptions::default());
        for explicit in [false, true] {
            let actual = html_completion_source(&doc, pos, explicit);
            let want = if !explicit && explicit_only {
                None
            } else {
                expected
            };
            assert_eq!(
                actual.as_ref().map(signature),
                want,
                "{input:?}, explicit={explicit}"
            );
        }
    }
}

#[test]
fn uses_the_documents_html_dialect() {
    for (input, match_closing_tags, self_closing_tags, expected) in [
        ("<div/><§", true, true, (7, 7, 112, TAGS)),
        ("<div></p>§", true, false, (9, 9, 113, 187_266_404)),
        ("<div></p>§", false, false, (9, 9, 112, START_TAGS)),
        ("<a/><b></§", false, true, (9, 9, 1, 2_844_044_669)),
    ] {
        let (doc, pos) = document_at(
            input,
            HtmlOptions {
                match_closing_tags,
                self_closing_tags,
            },
        );
        assert_eq!(
            html_completion_source(&doc, pos, true)
                .as_ref()
                .map(signature),
            Some(expected)
        );
    }
}

#[test]
fn custom_schema_matches_order_overrides_and_value_precedence() {
    use HtmlAttributeSpec as Attr;
    let config = HtmlCompletionConfig {
        extra_tags: &[
            HtmlTagSpec {
                attributes: &[
                    Attr::new("tone", Some(&["quiet", "loud"])),
                    Attr::new("shared", Some(&["local"])),
                    Attr::new("empty", Some(&["local"])),
                    Attr::new("dir", Some(&["custom"])),
                ],
                children: Some(&["x-leaf", "not-in-schema"]),
                global_attrs: false,
                ..HtmlTagSpec::new("x-panel")
            },
            HtmlTagSpec {
                attributes: &[
                    Attr::new("shared", Some(&["local"])),
                    Attr::new("empty", Some(&["local"])),
                ],
                children: Some(&[]),
                ..HtmlTagSpec::new("x-leaf")
            },
            HtmlTagSpec {
                attributes: &[Attr::new("mode", Some(&["a", "b"]))],
                ..HtmlTagSpec::new("input")
            },
        ],
        extra_global_attributes: &[
            Attr::new("shared", Some(&["global"])),
            Attr::new("empty", Some(&[])),
            Attr::new("dir", None),
            Attr::new("theme", Some(&["ocean", "forest"])),
        ],
    };
    let source = html_completion_source_with(config);
    for (input, expected) in [
        ("<§", (1, 1, 114, 115_857_834)),
        ("<x-panel><§", (10, 10, 3, 1_914_333_029)),
        ("<x-leaf><§", (9, 9, 1, 3_552_339_789)),
        ("<x-panel §>", (9, 9, 4, 1_750_051_222)),
        ("<x-leaf §>", (8, 8, 88, 3_484_714_358)),
        ("<input §>", (7, 7, 87, 2_215_891_776)),
        ("<x-panel tone=§>", (14, 14, 2, 3_347_317_769)),
        ("<x-panel shared=§>", (16, 16, 1, 844_375_675)),
        ("<x-panel empty=§>", (15, 15, 0, FNV_OFFSET)),
        ("<x-panel dir=§>", (13, 13, 1, 2_567_498_431)),
        ("<x-panel theme=§>", (15, 15, 2, 1_939_124_479)),
        ("<foo theme=\"o§\">", (12, 13, 2, 197_444_457)),
        ("<input type=§>", (12, 12, 0, FNV_OFFSET)),
    ] {
        let (doc, pos) = document_at(input, HtmlOptions::default());
        assert_eq!(
            source(&doc, pos, false).as_ref().map(signature),
            Some(expected),
            "{input:?}"
        );
    }
}

#[test]
fn rejects_non_byte_boundaries() {
    let doc = HtmlDocument::parse(Text::new("😀<"));
    assert!(html_completion_source(&doc, 2, true).is_none());
    assert!(html_completion_source(&doc, doc.text().len() + 1, true).is_none());
}

#[test]
fn session_inserts_with_quotes_and_reuses_a_valid_prefix() {
    let (doc, pos) = document_at("😀<div dir='l§'>", HtmlOptions::default());
    let mut state = EditorState::new(doc.text().as_str());
    state
        .update(
            Transaction::new(ChangeSet::default())
                .with_selection(EditorSelection::single(pos, pos)),
        )
        .unwrap();
    let mut session = CompletionSession::new(&state, CompletionConfig::default(), [SourceId(1)]);
    session.start(false, Duration::ZERO);
    let query_time = session.deadline().unwrap();
    let query = session.take_queries(query_time).pop().unwrap();
    assert!(
        session
            .finish_query(&query, html_completion_source(&doc, pos, false), query_time)
            .unwrap()
    );
    let id = session
        .options()
        .unwrap()
        .iter()
        .find(|opt| opt.item.label == "ltr")
        .unwrap()
        .id;
    let accepted = session
        .accept(id, Duration::from_millis(200))
        .unwrap()
        .unwrap();
    let mut applied = state.clone();
    applied.update(accepted).unwrap();
    assert_eq!(applied.text().as_str(), "😀<div dir='ltr'>");

    for (offset, input, status) in [
        (0, "t", CompletionStatus::Active),
        (1, "'", CompletionStatus::Pending),
    ] {
        let cursor = pos + offset;
        let changes = ChangeSet::single(Change::new(cursor..cursor, input)).unwrap();
        let desc = changes.desc(state.text().len()).unwrap();
        let end = cursor + input.len();
        state
            .update(
                Transaction::new(changes)
                    .with_selection(EditorSelection::single(end, end))
                    .with_user_event("input.type"),
            )
            .unwrap();
        session
            .update(
                &state,
                &desc,
                true,
                Some("input.type"),
                Duration::from_millis(220 + offset as u64),
            )
            .unwrap();
        assert_eq!(session.status(), status);
    }
}
