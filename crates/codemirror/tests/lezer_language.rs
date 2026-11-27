//! References from the SRI-verified `@codemirror/lang-lezer` 6.0.2 and
//! `@lezer/lezer` 1.1.3 tarballs. The oracle used `@codemirror/language` 6.13.1,
//! `@codemirror/state` 6.7.6, `@lezer/common` 1.5.7, `@lezer/lr` 1.4.13, and
//! `@lezer/highlight` 1.2.5. Tree, highlight, indentation, and fold results
//! use original UTF-8 bytes instead of the editor's UTF-16 offsets. Change
//! fixtures compare JavaScript `EditorState.update` against a fresh state.

use std::ops::Range;

use gpui_editor_codemirror::{
    Change, ChangeSet, EditError, Text,
    commands::CommentTokens,
    language::{
        IndentOptions, StreamSettings, TagModifier,
        lezer::ReuseStats,
        lezer_grammar::{
            BLOCK_COMMENT, LINE_COMMENT, LezerDocument, LezerNode, LezerNodeKind, NAME,
            indent_on_input, language_data,
        },
    },
};

fn parse(source: &str) -> LezerDocument {
    LezerDocument::parse(Text::new(source))
}

fn tree(node: LezerNode<'_>) -> String {
    let name = node.kind().name();
    let name = if name.starts_with('@')
        || (name.len() == 1 && !name.as_bytes()[0].is_ascii_alphabetic())
    {
        format!("{name:?}")
    } else {
        name.to_owned()
    };
    if node.children().len() == 0 {
        name
    } else {
        format!(
            "{name}({})",
            node.children().map(tree).collect::<Vec<_>>().join(",")
        )
    }
}

fn check_lines(doc: &LezerDocument, expected: &[(usize, Option<Range<usize>>)]) {
    assert_eq!(doc.text().line_count(), expected.len());
    for (i, (indent, fold)) in expected.iter().enumerate() {
        let line = doc.text().line(i + 1).unwrap();
        assert_eq!(
            doc.indentation(
                line.range.start,
                StreamSettings::default(),
                IndentOptions::default()
            ),
            Some(*indent),
            "indentation on line {}: {:?}",
            i + 1,
            line.text,
        );
        assert_eq!(
            doc.foldable(line.range.start, line.range.end),
            *fold,
            "fold on line {}",
            i + 1
        );
    }
}

fn style_records(doc: &LezerDocument) -> Vec<(Range<usize>, &str)> {
    doc.highlights(0..doc.text().len())
        .unwrap()
        .map(|span| {
            let [tag] = span.tags else { panic!("{span:?}") };
            let label = if tag.modifiers() == [TagModifier::Definition] {
                assert_eq!(tag.name(), "variableName");
                "variableName.definition"
            } else {
                assert!(tag.modifiers().is_empty());
                tag.name()
            };
            (span.range, label)
        })
        .collect()
}

fn node_records(doc: &LezerDocument) -> Vec<(&'static str, Range<usize>)> {
    doc.nodes()
        .map(|node| (node.kind().name(), node.range()))
        .collect()
}

fn check_fresh(doc: &LezerDocument) {
    let fresh = LezerDocument::parse(doc.text().clone());
    assert_eq!(node_records(doc), node_records(&fresh));
    assert_eq!(tree(doc.root()), tree(fresh.root()));
    assert_eq!(style_records(doc), style_records(&fresh));
    for i in 1..=doc.text().line_count() {
        let line = doc.text().line(i).unwrap().range;
        let indent = |doc: &LezerDocument| {
            doc.indentation(
                line.start,
                StreamSettings::default(),
                IndentOptions::default(),
            )
        };
        assert_eq!(indent(doc), indent(&fresh));
        assert_eq!(
            doc.foldable(line.start, line.end),
            fresh.foldable(line.start, line.end)
        );
    }
}

fn change(doc: &LezerDocument, range: Range<usize>, insert: &str) -> LezerDocument {
    doc.reparse(&ChangeSet::single(Change::new(range, insert)).unwrap())
        .unwrap()
}

#[test]
fn matches_expression_tree_and_highlights() {
    let source = concat!(
        "// 描述 😀\n",
        "@top Expr[@name=Root, custom={expr}.x] <P> { (foo.bar<P, \"x\"> | )+ !left ~amb\n",
        "  $[a\\]b] ![x] _ @digit @extend[@name=Alias]<Foo, \"é\"> Embedded {\n",
        "    /* body */ P? | [@name=Other] { P* }\n",
        "  }\n}",
    );
    let doc = parse(source);
    assert_eq!(doc.root().range(), 0..206);
    assert_eq!(doc.text().as_str(), source);
    assert_eq!(
        tree(doc.root()),
        concat!(
            r#"Grammar(LineComment,RuleDeclaration("@top",RuleName,Props("[",Prop(AtName,"=",Name),"#,
            r#"Prop(Name,"=",PropEsc("{",RuleName,"}"),".",Name),"]"),ParamList(Name),Body("{",Sequence("#,
            r#"Repeat1(ParenExpression("(",Choice(Call(ScopedName(RuleName,".",RuleName),ArgList(RuleName,Literal)),"|"),")"),"+"),"#,
            r#"PrecedenceMarker("!",PrecedenceName),AmbiguityMarker("~",Name),CharSet,InvertedCharSet,AnyChar,CharClass,"#,
            r#"Specialization("@extend",Props("[",Prop(AtName,"=",Name),"]"),ArgList(RuleName,Literal)),"#,
            r#"InlineRule(RuleName,Body("{",BlockComment,Choice(Optional(RuleName,"?"),"|","#,
            r#"InlineRule(Props("[",Prop(AtName,"=",Name),"]"),Body("{",Repeat(RuleName,"*"),"}"))),"}"))),"}")))"#,
        )
    );
    let expected = [
        (0..14, "lineComment"),
        (15..19, "modifier"),
        (20..24, "variableName.definition"),
        (24..25, "squareBracket"),
        (25..30, "keyword"),
        (31..35, "name"),
        (37..43, "name"),
        (44..45, "brace"),
        (45..49, "variableName"),
        (49..50, "brace"),
        (51..52, "name"),
        (52..53, "squareBracket"),
        (55..56, "name"),
        (58..59, "brace"),
        (60..61, "paren"),
        (61..64, "variableName"),
        (65..68, "variableName"),
        (69..70, "variableName"),
        (72..75, "string"),
        (77..78, "operator"),
        (79..80, "paren"),
        (80..81, "operator"),
        (82..83, "operator"),
        (83..87, "labelName"),
        (88..89, "operator"),
        (89..92, "name"),
        (95..102, "regexp"),
        (103..107, "regexp"),
        (108..109, "character"),
        (110..116, "atom"),
        (117..124, "operatorKeyword"),
        (124..125, "squareBracket"),
        (125..130, "keyword"),
        (131..136, "name"),
        (136..137, "squareBracket"),
        (138..141, "variableName"),
        (143..147, "string"),
        (149..157, "variableName.definition"),
        (158..159, "brace"),
        (164..174, "blockComment"),
        (175..176, "variableName"),
        (176..177, "operator"),
        (178..179, "operator"),
        (180..181, "squareBracket"),
        (181..186, "keyword"),
        (187..192, "name"),
        (192..193, "squareBracket"),
        (194..195, "brace"),
        (196..197, "variableName"),
        (197..198, "operator"),
        (199..200, "brace"),
        (203..204, "brace"),
        (205..206, "brace"),
    ];
    assert_eq!(style_records(&doc), expected);
    check_lines(
        &doc,
        &[
            (0, None),
            (0, Some(59..205)),
            (45, Some(159..203)),
            (4, None),
            (2, None),
            (43, None),
        ],
    );

    let emoji = source.find('😀').unwrap();
    let mut spans = doc.highlights(emoji..emoji + 4).unwrap();
    let span = spans.next().unwrap();
    assert_eq!(span.range, emoji..emoji + 4);
    assert_eq!(span.tags[0].name(), "lineComment");
    assert!(spans.next().is_none());
    assert!(doc.highlights(emoji + 1..emoji + 4).is_none());
    assert!(doc.highlights(emoji..emoji).unwrap().next().is_none());
    assert!(doc.highlights(Range { start: 2, end: 1 }).is_none());
    assert!(doc.foldable(0, emoji + 1).is_none());
    assert!(
        doc.indentation(
            emoji + 1,
            StreamSettings::default(),
            IndentOptions::default()
        )
        .is_none()
    );
}

#[test]
fn matches_declarations_and_folds() {
    let doc = parse(concat!(
        "@precedence {\n  primary @left, unary @right, assign @cut\n}\n",
        "@tokens {\n  word { @asciiLetter+ }\n  \"?\"[@name=Question]\n",
        "  @precedence {\n    word, \"?\"\n  }\n  @conflict { word, \"?\" }\n}\n",
        "@local tokens {\n  @else content[@name=Content]\n}\n",
        "@external tokens tokenizer from \"./tokens\" {\n  open[@dialect=base], close,\n",
        "  @conflict { token.call<a>, close }\n}\n",
        "@external prop metadata as info from \"./props\"\n",
        "@external propSource styling from \"./styling\"\n",
        "@external specialize { word } keywords from \"./keywords\" { Kw }\n",
        "@external extend { word } extensions from \"./extensions\" { New }\n",
        "@context track from \"./tracker\"\n@dialects { base, extra }\n",
        "@skip { ws }\n@skip {} {\n  @top Scoped { (word) }\n}\n@detectDelim",
    ));
    assert_eq!(
        tree(doc.root()),
        concat!(
            r#"Grammar(PrecedenceDeclaration("@precedence",PrecedenceBody("{",Precedence(PrecedenceName,"@left"),Precedence(PrecedenceName,"@right"),Precedence(PrecedenceName,"@cut"),"}")),"#,
            r#"TokensDeclaration("@tokens",TokensBody("{",RuleDeclaration(RuleName,Body("{",Repeat1(CharClass,"+"),"}")),"#,
            r#"LiteralTokenDeclaration(Literal,Props("[",Prop(AtName,"=",Name),"]")),TokenPrecedenceDeclaration("@precedence",PrecedenceBody("{",RuleName,Literal,"}")),"#,
            r#"TokenConflictDeclaration("@conflict",ConflictBody("{",RuleName,Literal,"}")),"}")),"#,
            r#"LocalTokensDeclaration("@local",tokens,TokensBody("{",ElseToken("@else",RuleName,Props("[",Prop(AtName,"=",Name),"]")),"}")),"#,
            r#"ExternalTokensDeclaration("@external",tokens,Name,from,Literal,TokensBody("{",Token(RuleName,Props("[",Prop(AtName,"=",Name),"]")),"#,
            r#"Token(RuleName),ExternalConflictDeclaration("@conflict",ConflictBody("{",Call(ScopedName(RuleName,".",RuleName),ArgList(RuleName)),RuleName,"}")),"}")),"#,
            r#"ExternalPropDeclaration("@external",prop,Name,as,Name,from,Literal),ExternalPropSourceDeclaration("@external",propSource,Name,from,Literal),"#,
            r#"ExternalSpecializeDeclaration("@external",specialize,Body("{",RuleName,"}"),Name,from,Literal,TokensBody("{",Token(RuleName),"}")),"#,
            r#"ExternalSpecializeDeclaration("@external",extend,Body("{",RuleName,"}"),Name,from,Literal,TokensBody("{",Token(RuleName),"}")),"#,
            r#"ContextDeclaration("@context",Name,from,Literal),DialectsDeclaration("@dialects",DialectBody("{",Name,Name,"}")),"#,
            r#"TopSkipDeclaration("@skip",Body("{",RuleName,"}")),SkipScope("@skip",Body("{","}"),SkipBody("{",RuleDeclaration("@top",RuleName,Body("{",ParenExpression("(",RuleName,")"),"}")),"}")),"#,
            r#"DetectDelimDeclaration("@detectDelim"))"#,
        )
    );
    check_lines(
        &doc,
        &[
            (0, Some(13..57)),
            (2, None),
            (0, None),
            (0, Some(68..176)),
            (2, None),
            (2, None),
            (2, Some(131..148)),
            (4, None),
            (2, None),
            (2, None),
            (0, None),
            (0, Some(193..225)),
            (2, None),
            (0, None),
            (0, Some(271..339)),
            (2, None),
            (2, None),
            (0, None),
            (0, None),
            (0, None),
            (0, None),
            (0, None),
            (0, None),
            (0, None),
            (0, None),
            (0, Some(644..670)),
            (2, None),
            (0, None),
            (0, None),
        ],
    );
    let highlights = |range| {
        doc.highlights(range)
            .unwrap()
            .map(|span| span.tags[0].name())
            .collect::<Vec<_>>()
    };
    assert_eq!(highlights(178..201), ["keyword", "brace"]); // @local and @else have no style.
    assert!(highlights(398..408).is_empty()); // propSource has no style.
    assert_eq!(highlights(672..684), ["definitionKeyword"]);
}

#[test]
fn matches_detected_delimiter_indentation_and_nested_folds() {
    let doc = parse(
        "  Rule { (foo\n\tbar\n  )\n    Inline[ann=Yes,\n           @name=Else] {\n    thing\n }\n }",
    );
    check_lines(
        &doc,
        &[
            (0, Some(8..82)),
            (10, None),
            (9, None),
            (9, None),
            (11, Some(67..79)),
            (6, None),
            (4, None),
            (7, None),
        ],
    );
    let pos = doc.text().line(3).unwrap().range.start;
    let override_indentation = |from| (from == 0).then_some(6);
    let indent = |options| doc.indentation(pos, StreamSettings::default(), options);
    assert_eq!(
        indent(IndentOptions {
            override_indentation: Some(&override_indentation),
            ..Default::default()
        }),
        Some(13)
    );
    let options = IndentOptions {
        simulate_break: Some(pos),
        simulate_double_break: true,
        ..Default::default()
    };
    assert_eq!(indent(options), Some(10));
    assert_eq!(
        indent(IndentOptions {
            override_indentation: Some(&override_indentation),
            ..options
        }),
        Some(14)
    );
    let nested = parse("R { Inner {\r\n    foo\r\n  }\r\n}\r\n");
    check_lines(
        &nested,
        &[(0, Some(3..27)), (2, None), (0, None), (2, None), (0, None)],
    );
}

#[test]
fn exposes_upstream_language_data() {
    assert_eq!(NAME, "lezer");
    let data = language_data();
    let comments = data.get::<CommentTokens>("commentTokens").unwrap();
    assert_eq!(comments.line, Some("//"));
    assert_eq!(comments.block, Some(("/*", "*/")));
    assert_eq!(comments.line, Some(LINE_COMMENT));
    assert_eq!(comments.block, Some(BLOCK_COMMENT));
    let trigger = data.get::<fn(&str) -> bool>("indentOnInput").unwrap();
    for text in ["}", "  \t}", "\u{feff}}"] {
        assert!(trigger(text) && indent_on_input(text));
    }
    for text in [" )", "  ]", "} ", "}\n", "\u{85}}", "foo}"] {
        assert!(!trigger(text) && !indent_on_input(text));
    }
}

#[test]
fn recovers_missing_syntax_and_keeps_crlf_byte_offsets() {
    let source = "First { (foo\r\n}\r\nNext {\r\n  // incomplete 😀\r\n";
    let doc = parse(source);
    assert_eq!(
        node_records(&doc),
        [
            ("Grammar", 0..47),
            ("RuleDeclaration", 0..15),
            ("RuleName", 0..5),
            ("Body", 6..15),
            ("{", 6..7),
            ("ParenExpression", 8..14),
            ("(", 8..9),
            ("RuleName", 9..12),
            ("⚠", 14..14),
            ("}", 14..15),
            ("RuleDeclaration", 17..47),
            ("RuleName", 17..21),
            ("Body", 22..47),
            ("{", 22..23),
            ("LineComment", 27..45),
            ("⚠", 47..47),
        ],
    );
    check_lines(
        &doc,
        &[
            (0, Some(7..14)),
            (9, None),
            (0, Some(23..47)),
            (2, None),
            (2, None),
        ],
    );
    let comment = doc.highlights(27..47).unwrap().next().unwrap();
    assert_eq!(comment.range, 27..45);
    assert_eq!(comment.tags[0].name(), "lineComment");

    assert_eq!(
        tree(parse("Rule { \"open\n }").root()),
        r#"Grammar(RuleDeclaration(RuleName,Body("{",Literal,"}")))"#
    );
    assert_eq!(
        tree(parse("R { foo /* note */ bar /* final */ }").root()),
        r#"Grammar(RuleDeclaration(RuleName,Body("{",Sequence(RuleName,BlockComment,RuleName),BlockComment,"}")))"#
    );
}

#[test]
fn matches_recovery_with_a_deleted_token_and_specialized_declarations() {
    let doc = parse(concat!(
        "@tokens {\r\n  ; // unexpected 😀\r\n  word { @digit+ }\r\n}\r\n",
        "@external tokens scan from \"x\" {\r\n  Word\r\n}\r\n",
    ));
    assert_eq!(
        tree(doc.root()),
        concat!(
            r#"Grammar(TokensDeclaration("@tokens",TokensBody("{",⚠,LineComment,RuleDeclaration("#,
            r#"RuleName,Body("{",Repeat1(CharClass,"+"),"}")),"}")),ExternalTokensDeclaration("#,
            r#""@external",tokens,Name,from,Literal,TokensBody("{",Token(RuleName),"}")))"#,
        )
    );
    let nodes = node_records(&doc);
    assert!(nodes.contains(&("⚠", 13..14)));
    assert_eq!(
        style_records(&doc),
        [
            (0..7, "definitionKeyword"),
            (8..9, "brace"),
            (15..33, "lineComment"),
            (37..41, "variableName.definition"),
            (42..43, "brace"),
            (44..50, "atom"),
            (50..51, "operator"),
            (52..53, "brace"),
            (55..56, "brace"),
            (58..67, "modifier"),
            (68..74, "keyword"),
            (75..79, "name"),
            (80..84, "keyword"),
            (85..88, "string"),
            (89..90, "brace"),
            (94..98, "variableName"),
            (100..101, "brace"),
        ]
    );
    check_lines(
        &doc,
        &[
            (0, Some(9..55)),
            (2, None),
            (2, None),
            (0, None),
            (0, Some(90..100)),
            (2, None),
            (0, None),
            (0, None),
        ],
    );
    assert_eq!(doc.reuse_stats(), ReuseStats::default());
}

#[test]
fn lezer_grammar_reuses_suffix_scans_after_crlf_normalization() {
    let rules: String = (0..70)
        .map(|i| format!("symbol{i} {{ \"a\" @digit }}\r\n"))
        .collect();
    let source = format!("@top Main {{ symbol0+ }}\r\n{rules}target {{ \"🦀\" }}\r\n{rules}");
    let doc = parse(&source);
    let at = source.find('🦀').unwrap();
    let cr = at + source[at..].find('\r').unwrap();
    let changes = ChangeSet::new(vec![
        Change::new(at..at + 4, "東京"),
        Change::new(cr..cr + 1, ""),
    ])
    .unwrap();
    let updated = doc.reparse(&changes).unwrap();
    let fresh = parse(updated.text().as_str());
    let records = |doc: &LezerDocument| {
        doc.nodes()
            .map(|node| (node.kind(), node.range(), node.children().len()))
            .collect::<Vec<_>>()
    };
    assert_eq!(records(&updated), records(&fresh));
    assert_eq!(style_records(&updated), style_records(&fresh));
    assert!(updated.reuse_stats().nodes_reused > 100);
    assert!(updated.reused_suffix_actions() > 100);
}

#[test]
fn reuses_specialized_prefix_and_matches_pinned_edits_through_recovery() {
    let prefix = "// α😀\r\n@top Head { @digit+ }\r\n".repeat(12);
    let cut = prefix.len();
    let source = format!("{prefix}@tokens {{\r\n  word {{ @digit+ }}\r\n}}\r\n");
    let doc = parse(&source);
    assert_eq!(doc.reuse_stats(), ReuseStats::default());
    let start = source.rfind("@digit").unwrap();
    let updated = change(&doc, start..start + 6, "@asciiLetter");
    let reuse = updated.reuse_stats();
    assert!(reuse.nodes_reused > 50, "{reuse:?}");
    assert!((64..start).contains(&reuse.resumed_at), "{reuse:?}");
    assert_eq!(doc.text().as_str(), source);

    let lf = source.replace("\r\n", "\n");
    let lf_start = lf.rfind("@digit").unwrap();
    let lf_updated = change(&parse(&lf), lf_start..lf_start + 6, "@asciiLetter");
    let normalized = source[..reuse.resumed_at].replace("\r\n", "\n");
    assert_eq!(lf_updated.reuse_stats().resumed_at, normalized.len());
    assert_eq!(lf_updated.reuse_stats().nodes_reused, reuse.nodes_reused);
    assert!(reuse.resumed_at > normalized.len());

    let broken = change(&updated, cut + 37..cut + 38, "");
    let fixed = change(&broken, cut + 37..cut + 37, "}");
    assert_eq!(broken.reuse_stats(), ReuseStats::default());
    assert_eq!(broken.reused_suffix_actions(), 0);
    assert!(fixed.reuse_stats().nodes_reused > 50);
    assert!(fixed.reuse_stats().resumed_at < cut + 37);
    assert_eq!(fixed.reused_suffix_actions(), 0);
    assert_eq!(updated.text(), fixed.text());
    for (doc, closed) in [(&updated, true), (&broken, false), (&fixed, true)] {
        check_fresh(doc);
        let declaration = doc.root().children().next_back().unwrap();
        let end = if closed { 38 } else { 39 };
        assert_eq!(declaration.range(), cut..cut + end);
        let last = if closed { r#""}""# } else { "⚠" };
        assert_eq!(
            tree(declaration),
            format!(
                concat!(
                    r#"TokensDeclaration("@tokens",TokensBody("{{",RuleDeclaration(RuleName,"#,
                    r#"Body("{{",Repeat1(CharClass,"+"),"}}")),{}))"#,
                ),
                last
            )
        );
        let tail_spans: Vec<_> = style_records(doc)
            .into_iter()
            .filter(|(range, _)| range.start >= cut)
            .map(|(range, tag)| (range.start - cut..range.end - cut, tag))
            .collect();
        let mut expected = vec![
            (0..7, "definitionKeyword"),
            (8..9, "brace"),
            (13..17, "variableName.definition"),
            (18..19, "brace"),
            (20..32, "atom"),
            (32..33, "operator"),
            (34..35, "brace"),
        ];
        if closed {
            expected.push((37..38, "brace"));
        }
        assert_eq!(tail_spans, expected);
        let line = doc.text().line(25).unwrap();
        let fold_end = if closed { 37 } else { 39 };
        assert_eq!(
            doc.foldable(line.range.start, line.range.end),
            Some(cut + 9..cut + fold_end)
        );
        let trailing_indent = if closed { 0 } else { 2 };
        for (index, indent) in [0, 2, trailing_indent, trailing_indent]
            .into_iter()
            .enumerate()
        {
            let line = doc.text().line(25 + index).unwrap();
            assert_eq!(
                doc.indentation(
                    line.range.start,
                    StreamSettings::default(),
                    IndentOptions::default()
                ),
                Some(indent)
            );
        }
    }
}

#[test]
fn reuses_before_a_crlf_split_and_join_and_rejects_invalid_byte_edits() {
    let prefix = "// α😀\r\n@top Head { @digit+ }\r\n".repeat(12);
    let source = format!("{prefix}Rule {{\r\n  // note 😀\r\n  @digit\r\n}}\r\n");
    let doc = parse(&source);
    let inside_emoji = source.find('😀').unwrap() + 1;
    let invalid = ChangeSet::single(Change::new(inside_emoji..inside_emoji, "x")).unwrap();
    assert_eq!(doc.reparse(&invalid).unwrap_err(), EditError::InvalidRange);
    assert_eq!(doc.text().as_str(), source);

    let lf = source.rfind("😀\r\n").unwrap() + "😀\r".len();
    let split = change(&doc, lf..lf, "x");
    let joined = change(&split, lf..lf + 1, "");
    assert_eq!(joined.text().as_str(), source);
    for edited in [&split, &joined] {
        let reuse = edited.reuse_stats();
        assert!(reuse.nodes_reused > 50, "{reuse:?}");
        assert!((64..lf).contains(&reuse.resumed_at), "{reuse:?}");
        check_fresh(edited);
        let last_comment = edited
            .nodes()
            .filter(|node| node.kind() == LezerNodeKind::LineComment)
            .last()
            .unwrap();
        assert_eq!(
            edited.text().slice(last_comment.range()),
            Some("// note 😀")
        );
    }
    assert!(
        split
            .nodes()
            .any(|node| node.kind() == LezerNodeKind::RuleName
                && split.text().slice(node.range()) == Some("x"))
    );
}

#[test]
fn limits_recovery_nesting_and_keeps_later_declarations() {
    let depth = 16_384;
    let source = format!(
        "@unknown\nR {{ {}x{} }}\nS {{ y }}",
        "(".repeat(depth),
        ")".repeat(depth)
    );
    let doc = parse(&source);
    assert!(
        doc.nodes()
            .any(|node| node.kind() == LezerNodeKind::Error && node.range().len() > depth)
    );
    assert_eq!(
        tree(doc.root().children().next_back().unwrap()),
        r#"RuleDeclaration(RuleName,Body("{",RuleName,"}"))"#
    );
    for node in doc.nodes() {
        let range = node.range();
        assert!(source.get(range.clone()).is_some());
        let mut end = range.start;
        for child in node.children() {
            let child_range = child.range();
            assert_eq!(child.parent().unwrap().range(), range);
            assert!(end <= child_range.start && child_range.end <= range.end);
            end = child_range.end;
        }
    }
}
