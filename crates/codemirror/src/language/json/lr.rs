//! Generated parser tables from @lezer/json 1.0.3.
//! See docs/licenses/codemirror-lang-json.txt.

use std::sync::LazyLock;

use super::{JsonNodeKind, Node};
use crate::language::lezer::{
    CLOSED_BY, FOLD, GrammarSpec, INDENT, ISOLATE, Isolation, LANGUAGE_DATA, LanguageFacet,
    NodeTypes, OPENED_BY, ParseError, Parser, STYLE_RULES, SerializedTables, StyleRules,
    SyntaxTree, indent::stock_json_indent, stock_fold,
};

pub(super) static PARSER: LazyLock<Parser> =
    LazyLock::new(|| Parser::deserialize(JSON_GRAMMAR).expect("verified @lezer/json tables"));

/// Parse original JSON source without editor line normalization. Ranges
/// use UTF-8 bytes. Strict mode stops when no branch can advance. Both modes
/// are bounded and may fail without producing a tree. This is not JSON lint.
pub fn parse_raw(source: &str, strict: bool) -> Result<SyntaxTree, ParseError> {
    if strict {
        PARSER.parse_editor_strict(source)
    } else {
        PARSER.parse_incremental_json_recovering(source, &[])
    }
}

const JSON: SerializedTables<'_> = SerializedTables {
    version: 14,
    states: "$bOVQPOOOOQO'#Cb'#CbOnQPO'#CeOvQPO'#ClOOQO'#Cr'#CrQOQPOOOOQO'#Cg'#CgO}QPO'#CfO!SQPO'#CtOOQO,59P,59PO![QPO,59PO!aQPO'#CuOOQO,59W,59WO!iQPO,59WOVQPO,59QOqQPO'#CmO!nQPO,59`OOQO1G.k1G.kOVQPO'#CnO!vQPO,59aOOQO1G.r1G.rOOQO1G.l1G.lOOQO,59X,59XOOQO-E6k-E6kOOQO,59Y,59YOOQO-E6l-E6l",
    state_data: "#O~OeOS~OQSORSOSSOTSOWQO_ROgPO~OVXOgUO~O^[O~PVO[^O~O]_OVhX~OVaO~O]bO^iX~O^dO~O]_OVha~O]bO^ia~O",
    goto: "!kjPPPPPPkPPkqwPPPPk{!RPPP!XP!e!hXSOR^bQWQRf_TVQ_Q`WRg`QcZRicQTOQZRQe^RhbRYQR]R",
    token_data: "(|~RaXY!WYZ!W]^!Wpq!Wrs!]|}$u}!O$z!Q!R%T!R![&c![!]&t!}#O&y#P#Q'O#Y#Z'T#b#c'r#h#i(Z#o#p(r#q#r(w~!]Oe~~!`Wpq!]qr!]rs!xs#O!]#O#P!}#P;'S!];'S;=`$o<%lO!]~!}Og~~#QXrs!]!P!Q!]#O#P!]#U#V!]#Y#Z!]#b#c!]#f#g!]#h#i!]#i#j#m~#pR!Q![#y!c!i#y#T#Z#y~#|R!Q![$V!c!i$V#T#Z$V~$YR!Q![$c!c!i$c#T#Z$c~$fR!Q![!]!c!i!]#T#Z!]~$rP;=`<%l!]~$zO]~~$}Q!Q!R%T!R![&c~%YRT~!O!P%c!g!h%w#X#Y%w~%fP!Q![%i~%nRT~!Q![%i!g!h%w#X#Y%w~%zR{|&T}!O&T!Q![&Z~&WP!Q![&Z~&`PT~!Q![&Z~&hST~!O!P%c!Q![&c!g!h%w#X#Y%w~&yO[~~'OO_~~'TO^~~'WP#T#U'Z~'^P#`#a'a~'dP#g#h'g~'jP#X#Y'm~'rOR~~'uP#i#j'x~'{P#`#a(O~(RP#`#a(U~(ZOS~~(^P#f#g(a~(dP#i#j(g~(jP#X#Y(m~(rOQ~~(wOW~~(|OV~",
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

const KINDS: [JsonNodeKind; 17] = {
    use JsonNodeKind::*;
    [
        Error,
        JsonText,
        True,
        False,
        Null,
        Number,
        String,
        CloseBrace,
        OpenBrace,
        Object,
        Property,
        PropertyName,
        Colon,
        Comma,
        CloseBracket,
        OpenBracket,
        Array,
    ]
};

pub(crate) fn stock_data_facet() -> &'static LanguageFacet {
    static FACET: LazyLock<LanguageFacet> =
        LazyLock::new(|| LanguageFacet::new(Some(super::editor_language_data())));
    &FACET
}

pub(crate) fn stock_node_types() -> &'static NodeTypes {
    static TYPES: LazyLock<NodeTypes> = LazyLock::new(|| {
        let top = JSON_GRAMMAR.top_rule[1];
        let [object, array] = stock_json_indent();
        NodeTypes::new(
            JSON_GRAMMAR.node_names,
            JSON_GRAMMAR.repeat_node_count,
            top,
            &[0],
            |term, _, props| {
                match term {
                    6 | 11 => props.insert(&ISOLATE, Some(Isolation::Auto)),
                    7 => props.insert(&OPENED_BY, vec!["{".into()]),
                    8 => props.insert(&CLOSED_BY, vec!["}".into()]),
                    9 | 16 => {
                        props.insert(&FOLD, Some(stock_fold()));
                        props.insert(
                            &INDENT,
                            Some(if term == 9 {
                                object.clone()
                            } else {
                                array.clone()
                            }),
                        );
                    }
                    14 => props.insert(&OPENED_BY, vec!["[".into()]),
                    15 => props.insert(&CLOSED_BY, vec!["]".into()]),
                    _ if term == top => {
                        props.insert(&LANGUAGE_DATA, Some(stock_data_facet().clone()))
                    }
                    _ => (),
                }
                if let Some(tags) = KINDS
                    .get(usize::from(term))
                    .and_then(|kind| kind.highlight())
                {
                    props.insert(&STYLE_RULES, StyleRules::normal(tags));
                }
            },
        )
    });
    &TYPES
}

pub(super) fn term(kind: JsonNodeKind) -> u16 {
    KINDS
        .iter()
        .position(|&row| row == kind)
        .expect("JSON kind has a grammar term") as u16
}

pub(super) fn nodes(tree: &SyntaxTree) -> Vec<Node> {
    let mut nodes: Vec<Node> = Vec::new();
    let mut pending = vec![(None::<usize>, tree.root())];
    while let Some((parent, node)) = pending.pop() {
        let id = nodes.len();
        if let Some(parent) = parent {
            nodes[parent].children.push(id);
        }
        nodes.push(Node {
            kind: KINDS[usize::from(node.term())],
            range: node.range(),
            parent,
            children: Vec::with_capacity(node.children().len()),
        });
        pending.extend(node.children().rev().map(|child| (Some(id), child)));
    }
    nodes
}
