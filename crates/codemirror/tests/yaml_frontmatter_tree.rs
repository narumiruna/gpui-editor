use std::sync::Arc;

use gpui_editor_codemirror::{
    Change, ChangeSet, EditorState, Extension, Text, Transaction,
    language::{lezer::SyntaxNode, yaml::frontmatter_language::YamlFrontmatterLanguage},
    language_data::{NativeLanguage, NativeSyntax},
};
use serde_json::{Value, json};

fn nodes(root: SyntaxNode<'_>) -> Value {
    let mut output = Vec::new();
    let mut pending = vec![(0, root)];
    while let Some((depth, node)) = pending.pop() {
        let range = node.range();
        output.push(json!([depth, node.name(), range.start, range.end]));
        pending.extend(node.children().rev().map(|child| (depth + 1, child)));
    }
    json!(output)
}

#[test]
fn outer_tree_and_mount_boundaries_match_pinned_wrapper() {
    let language = YamlFrontmatterLanguage::new(NativeLanguage::Json);
    for line in include_str!("fixtures/yaml_frontmatter_tree.jsonl").lines() {
        let row: Value = serde_json::from_str(line).unwrap();
        let source = row["source"].as_str().unwrap();
        let doc = language.parse(Text::new(source)).unwrap();
        let tree = doc
            .wrapper_tree()
            .unwrap_or_else(|error| panic!("{source:?}: {error:?}"));
        assert_eq!(nodes(tree.root()), row["nodes"], "{source:?}");
        assert!(std::ptr::eq(tree, doc.wrapper_tree().unwrap()));

        let mut mounts = Vec::new();
        if let Some(front) = doc.frontmatter() {
            let root = front.yaml().root();
            let range = root.range();
            mounts.push(json!([
                "FrontmatterContent",
                range.start,
                range.end,
                root.kind().name()
            ]));
        }
        let NativeSyntax::Json(body) = doc.body() else {
            panic!("expected the JSON body");
        };
        let range = doc.body_range();
        mounts.push(json!([
            "Body",
            range.start,
            range.end,
            body.root().kind().name()
        ]));
        assert_eq!(json!(mounts), row["mounts"], "{source:?}");
    }
}

#[test]
fn outer_tree_follows_edits_and_keeps_retained_snapshots() {
    let language = YamlFrontmatterLanguage::new(NativeLanguage::Json);
    let field = language.state_field(Extension::default());
    let source = "---\r\ntitle: 前😀\r[]";
    let mut state = EditorState::with_extensions(source, field.extension()).unwrap();
    let snapshot = |state: &EditorState| state.context().field(&field).unwrap().unwrap();
    let retained = snapshot(&state);
    let original = retained.as_ref().as_ref().unwrap().wrapper_tree().unwrap();
    assert!(
        original
            .root()
            .children()
            .next()
            .unwrap()
            .children()
            .any(SyntaxNode::is_error)
    );
    let opening = source.find('[').unwrap();
    let edit = ChangeSet::single(Change::new(opening..opening, "---\r\n")).unwrap();
    state.update(Transaction::new(edit.clone())).unwrap();
    let updated = snapshot(&state);
    assert!(!Arc::ptr_eq(&retained, &updated));
    let updated = updated.as_ref().as_ref().unwrap();
    let fixed = updated.wrapper_tree().unwrap();
    assert!(
        !fixed
            .root()
            .children()
            .next()
            .unwrap()
            .children()
            .any(SyntaxNode::is_error)
    );
    assert_eq!(updated.body_range().start, opening + 5);
    assert!(
        original
            .root()
            .children()
            .next()
            .unwrap()
            .children()
            .any(SyntaxNode::is_error)
    );

    let mut direct = language.parse(Text::new(source)).unwrap();
    let cached = std::ptr::from_ref(direct.wrapper_tree().unwrap());
    let split = source.find('😀').unwrap() + 1;
    let invalid = ChangeSet::single(Change::new(split..split, "x")).unwrap();
    assert!(direct.apply_changes(&invalid).is_err());
    assert_eq!(std::ptr::from_ref(direct.wrapper_tree().unwrap()), cached);
    assert_eq!(
        nodes(direct.wrapper_tree().unwrap().root()),
        nodes(original.root())
    );
    direct.apply_changes(&edit).unwrap();
    assert_eq!(
        nodes(direct.wrapper_tree().unwrap().root()),
        nodes(fixed.root())
    );
}
