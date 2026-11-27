use gpui_editor_codemirror::{
    Change, ChangeSet, Text,
    autocomplete::session::SourceResult,
    language::{
        IndentOptions, StreamSettings,
        css::{CssDocument, css_completion_source},
    },
};
use serde_json::{Value, json};

// Captured with @codemirror/lang-css 6.3.1, @lezer/css 1.3.9 and
// @codemirror/autocomplete 6.20.4 in Chromium 159.0.8154.6. The Styles
// cases use cssLanguage.configure({top: "Styles"}) and css().support.
// The state uses LF as its separator so CRLF stays in the input. The cursor
// marker is not part of the document. All positions are UTF-8 byte offsets.
#[test]
fn recovery_matches_pinned_tree_and_editor_services() {
    let cases: Vec<Value> =
        serde_json::from_str(include_str!("fixtures/css_recovery.json")).unwrap();
    for case in cases {
        let input = case["input"].as_str().unwrap();
        let (before, after) = input.split_once('§').unwrap();
        let source = [before, after].concat();
        let doc = match case["mode"].as_str().unwrap() {
            "styles" => CssDocument::parse_styles(Text::new(&source)),
            "css" => CssDocument::parse(Text::new(&source)),
            mode => panic!("unknown CSS parse mode: {mode}"),
        };
        assert_eq!(doc.text().as_str(), source);
        let nodes: Vec<_> = doc
            .nodes()
            .map(|node| {
                let depth = std::iter::successors(node.parent(), |node| node.parent()).count();
                let range = node.range();
                json!([node.kind().name(), range.start, range.end, depth])
            })
            .collect();
        assert_eq!(json!(nodes), case["nodes"], "tree for {input:?}");

        let highlights: Vec<_> = doc
            .highlights(0..source.len())
            .unwrap()
            .map(|span| {
                let [tag] = span.tags else {
                    panic!("expected one CSS tag")
                };
                assert!(tag.modifiers().is_empty());
                json!([span.range.start, span.range.end, tag.name()])
            })
            .collect();
        assert_eq!(json!(highlights), case["highlights"], "highlight {input:?}");

        let mut start = 0;
        let lines: Vec<_> = source
            .split('\n')
            .map(|line| {
                let end = start + line.len();
                let indent =
                    doc.indentation(start, StreamSettings::default(), IndentOptions::default());
                let fold = doc
                    .foldable(start, end)
                    .map(|range| [range.start, range.end]);
                start = end + 1;
                json!([indent, fold])
            })
            .collect();
        assert_eq!(json!(lines), case["lines"], "line services for {input:?}");

        let completions = [false, true].map(|explicit| {
            css_completion_source(&doc, before.len(), explicit).map(completion_signature)
        });
        assert_eq!(
            json!(completions),
            case["completions"],
            "completion {input:?}"
        );
    }
}

fn completion_signature(result: SourceResult) -> Value {
    assert_eq!(result.to, None);
    assert!(result.filter);
    // FNV-1a over the full ordered sets of labels, types, and apply text.
    let mut hash = 2_166_136_261u32;
    for item in result.options.iter() {
        for byte in item
            .label
            .bytes()
            .chain([0])
            .chain(item.kind.as_deref().unwrap().bytes())
            .chain([0])
            .chain(item.insert.as_deref().unwrap_or_default().bytes())
            .chain([b'\n'])
        {
            hash = (hash ^ u32::from(byte)).wrapping_mul(16_777_619);
        }
    }
    let kind = result
        .options
        .first()
        .map_or("empty", |item| item.kind.as_deref().unwrap());
    json!([result.from, kind, result.options.len(), hash])
}

#[test]
fn removing_a_call_closer_does_not_take_the_next_style() {
    let source = "--tone: tomato;\r\ncolor: var( /*😀*/ );\r\nwidth: 4px;";
    let doc = CssDocument::parse_styles(Text::new(source));
    let closer = source.find(')').unwrap();
    let remove = ChangeSet::single(Change::new(closer..closer + 1, "")).unwrap();
    let recovered = doc.reparse(&remove).unwrap();
    assert_eq!(
        recovered.text().as_str(),
        "--tone: tomato;\r\ncolor: var( /*😀*/ ;\r\nwidth: 4px;"
    );
    let result = css_completion_source(&recovered, closer, true).unwrap();
    assert_eq!(result.from, closer);
    assert_eq!(result.options.len(), 1);
    assert_eq!(result.options[0].label, "--tone");
    let width = recovered.text().as_str().find("width").unwrap();
    let result = css_completion_source(&recovered, width + 2, false).unwrap();
    assert_eq!(result.from, width);
    assert!(result.options.iter().any(|item| item.label == "width"));

    let restore = ChangeSet::single(Change::new(closer..closer, ")")).unwrap();
    let restored = recovered.reparse(&restore).unwrap();
    assert_eq!(restored.text().as_str(), source);
    assert!(!restored.nodes().any(|node| node.kind().name() == "⚠"));
}
