//! Reference output from @codemirror/lang-vue 0.1.3 with the pinned dependencies.

use std::ops::Range;

use gpui_editor_codemirror::{
    Text,
    language::{
        IndentOptions, StreamSettings, TagModifier,
        html::{HtmlEmbeddedDocument, HtmlEmbeddedLanguage, HtmlNodeKind},
        vue::{VueDocument, VueExpressionKind},
    },
};

fn highlights(doc: &VueDocument, range: Range<usize>) -> Vec<(&str, String)> {
    doc.highlights(range)
        .unwrap()
        .map(|span| {
            (
                doc.text().slice(span.range).unwrap(),
                span.tags
                    .iter()
                    .map(|tag| tag.name())
                    .collect::<Vec<_>>()
                    .join(" "),
            )
        })
        .collect()
}

#[test]
fn highlights_vue_template_and_routes_expressions() {
    let source = "<template>\r\n  🦀 <Comp v-if=\"é\" /> {{ É }}\r\n</template>";
    let doc = VueDocument::parse(Text::new(source));
    assert_eq!(doc.text().as_str(), source);
    assert_eq!(
        highlights(&doc, 0..source.len()),
        [
            ("<", "angleBracket"),
            ("template", "tagName"),
            (">", "angleBracket"),
            ("\r\n  🦀 ", "content"),
            ("<", "angleBracket"),
            ("Comp", "tagName"),
            ("v-if", "keyword"),
            ("=", "definitionOperator"),
            ("\"", "attributeValue"),
            ("é", "variableName"),
            ("\"", "attributeValue"),
            ("/>", "angleBracket"),
            (" ", "content"),
            ("É", "variableName"),
            ("}}", "brace"),
            ("\r\n", "content"),
            ("</", "angleBracket"),
            ("template", "tagName"),
            (">", "angleBracket"),
        ]
        .map(|(text, tag)| (text, tag.to_owned()))
    );
    assert_eq!(
        doc.expressions()
            .iter()
            .map(|mount| (
                mount.kind(),
                mount.range(),
                mount.document().text().as_str()
            ))
            .collect::<Vec<_>>(),
        [
            (VueExpressionKind::Directive, 31..33, "é"),
            (VueExpressionKind::Interpolation, 40..44, " É "),
        ]
    );
    assert_eq!(
        highlights(&doc, 30..33),
        [
            ("\"", "attributeValue".into()),
            ("é", "variableName".into())
        ]
    );
    assert!(doc.highlights(32..33).is_none());
    assert!(doc.highlights(33..source.len() + 1).is_none());
    assert!(highlights(&doc, 31..31).is_empty());
    assert!(doc.host().nodes().any(|node| {
        node.kind() == HtmlNodeKind::SelfClosingTag
            && doc.text().slice(node.range()) == Some("<Comp v-if=\"é\" />")
    }));
}

#[test]
fn routes_blocks_to_html_native_children() {
    let source = "<template>\n  <Panel />\n</template>\n<script setup lang=\"ts\">\nconst x: number = 2\n</script>\n<style scoped>\n.p {\ncolor: red;\n}\n</style>";
    let doc = VueDocument::parse(Text::new(source));
    let mounts = doc.host().mounts();
    assert_eq!(mounts.len(), 2);
    assert_eq!(mounts[0].language(), HtmlEmbeddedLanguage::TypeScript);
    let HtmlEmbeddedDocument::Javascript(script) = mounts[0].document() else {
        panic!("expected the JavaScript child tree");
    };
    assert_eq!(script.text().as_str(), "\nconst x: number = 2\n");
    assert!(!script.is_recovered());
    assert_eq!(mounts[1].language(), HtmlEmbeddedLanguage::Css);
    let HtmlEmbeddedDocument::Css(style) = mounts[1].document() else {
        panic!("expected the CSS child tree");
    };
    assert_eq!(style.text().as_str(), "\n.p {\ncolor: red;\n}\n");
    let spans = highlights(&doc, 0..source.len());
    assert!(spans.contains(&("number", "typeName".into())));
    assert!(spans.contains(&("color", "propertyName".into())));
    for (open, close) in [
        ("<template>", "</template>"),
        ("<script setup lang=\"ts\">", "</script>"),
        ("<style scoped>", "</style>"),
    ] {
        let start = source.find(open).unwrap();
        let end = start + open.len();
        assert_eq!(
            doc.foldable(start, end),
            Some(end..source.find(close).unwrap())
        );
    }
    let css = source.find(".p {").unwrap();
    let from = css + ".p {".len();
    assert_eq!(
        doc.foldable(css, from),
        Some(from..source.find('}').unwrap())
    );
    let settings = StreamSettings::default();
    let indent = |line| {
        doc.indentation(
            source.find(line).unwrap(),
            settings,
            IndentOptions::default(),
        )
    };
    assert_eq!(indent("  <Panel />"), Some(2));
    assert_eq!(indent("const x"), Some(2));
    assert_eq!(indent(".p {"), Some(2));
    assert_eq!(indent("color: red;"), Some(2));
    assert_eq!(indent("}"), Some(0));
    assert_eq!(indent("</style>"), Some(0));
}

#[test]
fn parses_directives_as_javascript_single_expressions() {
    let source = r#"<Widget v-model.trim="model" v-bind:key='row.id' :style="{color: color}" @click="x++; y()" v-for="(x, i) in list" v-if=ok :[slot]="x" #[name]="y" v-slot="{ foo }" />"#;
    let doc = VueDocument::parse(Text::new(source));
    assert_eq!(
        doc.expressions()
            .iter()
            .map(|mount| mount.document().text().as_str())
            .collect::<Vec<_>>(),
        [
            "model",
            "row.id",
            "{color: color}",
            "x++; y()",
            "(x, i) in list",
            "x",
            "{ foo }",
        ]
    );
    let expr = &doc.expressions()[2];
    let js = expr.document();
    assert!(!js.is_recovered());
    assert_eq!(js.root().kind().name(), "SingleExpression");
    assert_eq!(
        js.root().children().next().unwrap().kind().name(),
        "ObjectExpression"
    );
    let span = doc
        .highlights(expr.range())
        .unwrap()
        .find(|span| &source[span.range.clone()] == "color")
        .unwrap();
    assert_eq!(span.tags[0].name(), "propertyName");
    assert_eq!(span.tags[0].modifiers(), &[TagModifier::Definition]);
    let spans = highlights(&doc, 0..source.len());
    for (token, tag) in [
        ("v-model", "keyword"),
        ("trim", "variableName"),
        ("@", "punctuation"),
        ("click", "variableName"),
        ("in", "operatorKeyword"),
        ("ok", "attributeName"),
        ("slot", "variableName"),
        ("#[name]", "attributeName"),
    ] {
        assert!(spans.contains(&(token, tag.to_owned())));
    }
}

#[test]
fn folds_and_indents_through_vue_expressions() {
    let source = "<template>\n<Comp\n:prop=\"fn(\na,\nb\n)\"\n/>\n<div>\n{{ foo({\nx: 1\n}) }}\n</div>\n</template>\n<script lang=\"ts\">\nfunction f(): number {\nreturn 1\n}\n</script>";
    let doc = VueDocument::parse(Text::new(source));
    let starts = std::iter::once(0).chain(source.match_indices('\n').map(|(at, _)| at + 1));
    assert_eq!(
        starts
            .map(|at| doc.indentation(at, StreamSettings::default(), IndentOptions::default()))
            .collect::<Vec<_>>(),
        [0, 2, 2, 2, 2, 0, 2, 2, 2, 2, 0, 0, 0, 0, 2, 2, 0, 0].map(Some)
    );
    let expr_start = source.find("{{ foo({").unwrap();
    let expr_end = expr_start + "{{ foo({".len();
    assert_eq!(
        doc.foldable(expr_start, expr_end),
        Some(expr_end..source.find("}) }}").unwrap())
    );
    let js_start = source.find("function f").unwrap();
    let js_end = js_start + "function f(): number {".len();
    assert_eq!(
        doc.foldable(js_start, js_end),
        Some(js_end..source.find("}\n</script>").unwrap())
    );
}

#[test]
fn preserves_html_mount_boundaries_and_recovers_incomplete_vue_fragments() {
    let source = "<template>😀 {{ x &amp; y }} - {{ a < b }} - {{}} - {{ foo\n}} {{ broken</template><br>{{ good }}";
    let doc = VueDocument::parse(Text::new(source));
    assert_eq!(
        doc.expressions()
            .iter()
            .map(|expr| expr.document().text().as_str())
            .collect::<Vec<_>>(),
        [" x ", " a ", " foo\n", " broken", " good "]
    );
    let spans = highlights(&doc, 0..source.len());
    assert!(spans.contains(&("&amp;", "character".into())));
    assert!(!spans.iter().any(|&(token, _)| token == "{{"));
    assert_eq!(
        spans
            .iter()
            .filter(|(token, tag)| *token == "}}" && tag == "brace")
            .count(),
        3
    );

    let malformed = [
        "",
        "<",
        "<<a v-未知='😀",
        "{{🔥",
        "{{ odd }",
        "{{{{{{",
        "<x :é=火>foo</",
        "<x :é=\"🔥\r\n更多",
        "<C :[bad]=\"unclosed",
        r#"<C :x='' @="" />"#,
    ];
    for source in malformed.into_iter().map(str::to_owned).chain([
        "{{x}} ".repeat(4_096) + "{{unterminated 🦀",
        "<C :[x.foo]=\"v\" />".repeat(4_096),
    ]) {
        let doc = VueDocument::parse(Text::new(&source));
        let mut end = 0;
        for span in doc.highlights(0..source.len()).unwrap() {
            assert!(span.range.start >= end && !span.range.is_empty());
            assert!(source.get(span.range.clone()).is_some());
            end = span.range.end;
        }
        for mount in doc.expressions() {
            assert_eq!(
                source.get(mount.range()),
                Some(mount.document().text().as_str())
            );
        }
    }
}
