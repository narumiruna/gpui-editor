// Published Lezer v14 grammar from @codemirror/lang-vue 0.1.3, dist/index.js.
// The local token data is decoded to u16 arrays. See docs/licenses/codemirror-lang-vue.txt.

use std::sync::LazyLock;

use super::super::lezer::{
    ExternalOptions, ExternalToken, ExternalTokenizer, GrammarSpec, ParseError, Parser,
    SerializedTables, TokenInput, Tokenizer,
};

const STATES: &str = r#"%pOVOWOOObQPOOOpOSO'#C_OOOO'#Cp'#CpQVOWOOQxQPOOO!TQQOOQ!YQPOOOOOO,58y,58yO!_OSO,58yOOOO-E6n-E6nO!dQQO'#CqQ{QPOOO!iQPOOQ{QPOOO!qQPOOOOOO1G.e1G.eOOQO,59],59]OOQO-E6o-E6oO!yOpO'#CiO#RO`O'#CiQOQPOOO#ZO#tO'#CmO#fO!bO'#CmOOQO,59T,59TO#qOpO,59TO#vO`O,59TOOOO'#Cr'#CrO#{O#tO,59XOOQO,59X,59XOOOO'#Cs'#CsO$WO!bO,59XOOQO1G.o1G.oOOOO-E6p-E6pOOQO1G.s1G.sOOOO-E6q-E6q"#;
const STATE_DATA: &str = r#"$g~OjOS~OQROUROkQO~OWTOXUOZUO`VO~OSXOTWO~OXUO[]OlZO~OY^O~O[_O~OT`O~OYaO~OmcOodO~OmfOogO~O^iOnhO~O_jOphO~ObkOqkOrmO~OcnOsnOtmO~OnpO~OppO~ObkOqkOrrO~OcnOsnOtrO~OWX`~"#;
const GOTO: &str = r#"!^hPPPiPPPPPPPPPmPPPpPPsy!Q!WTROSRe]Re_QSORYSS[T^Rb[QlfRqlQogRso"#;
const NODE_NAMES: &str = r#"⚠ Content Text Interpolation InterpolationContent }} Entity Attribute VueAttributeName : Identifier @ Is ScriptAttributeValue AttributeScript AttributeScript AttributeName AttributeValue Entity Entity"#;
const TOKEN_DATA: &str = r#"'y~RdXY!aYZ!a]^!apq!ars!rwx!w}!O!|!O!P#t!Q![#y![!]$s!_!`%g!b!c%l!c!}#y#R#S#y#T#j#y#j#k%q#k#o#y%W;'S#y;'S;:j$m<%lO#y~!fSj~XY!aYZ!a]^!apq!a~!wOm~~!|Oo~!b#RX`!b}!O!|!Q![!|![!]!|!c!}!|#R#S!|#T#o!|%W;'S!|;'S;:j#n<%lO!|!b#qP;=`<%l!|~#yOl~%W$QXY#t`!b}!O!|!Q![#y![!]!|!c!}#y#R#S#y#T#o#y%W;'S#y;'S;:j$m<%lO#y%W$pP;=`<%l#y~$zXX~`!b}!O!|!Q![!|![!]!|!c!}!|#R#S!|#T#o!|%W;'S!|;'S;:j#n<%lO!|~%lO[~~%qOZ~%W%xXY#t`!b}!O&e!Q![#y![!]!|!c!}#y#R#S#y#T#o#y%W;'S#y;'S;:j$m<%lO#y!b&jX`!b}!O!|!Q![!|![!]!|!c!}'V#R#S!|#T#o'V%W;'S!|;'S;:j#n<%lO!|!b'^XW!b`!b}!O!|!Q![!|![!]!|!c!}'V#R#S!|#T#o'V%W;'S!|;'S;:j#n<%lO!|"#;

const LOCAL_GROUPS: [(&[u16], usize, u16); 6] = [
    (
        &[
            65535, 3, 1, 125, 126, 6, 65535, 9, 1, 125, 126, 12, 65535, 17, 0, 5, 65535, 65535,
        ],
        17,
        4,
    ),
    (
        &[
            65535, 3, 2, 38, 39, 9, 123, 124, 61, 65535, 14, 5, 6, 65535, 0, 32, 29, 33, 59, 29,
            60, 55296, 29, 55296, 56320, 55, 57344, 0, 29, 65535, 32, 6, 0, 32, 29, 33, 59, 29, 59,
            60, 50, 60, 55296, 29, 55296, 56320, 55, 57344, 0, 29, 65535, 55, 0, 6, 65535, 65535,
            58, 1, 56320, 57344, 29, 65535, 64, 1, 123, 124, 67, 65535, 72, 0, 27, 65535, 65535,
        ],
        72,
        2,
    ),
    (
        &[65535, 3, 1, 39, 40, 6, 65535, 11, 0, 32, 65535, 65535],
        11,
        15,
    ),
    (
        &[65535, 3, 1, 34, 35, 6, 65535, 11, 0, 30, 65535, 65535],
        11,
        14,
    ),
    (
        &[
            65535, 3, 2, 38, 39, 9, 39, 40, 61, 65535, 14, 5, 19, 65535, 0, 32, 29, 33, 59, 29, 60,
            55296, 29, 55296, 56320, 55, 57344, 0, 29, 65535, 32, 6, 0, 32, 29, 33, 59, 29, 59, 60,
            50, 60, 55296, 29, 55296, 56320, 55, 57344, 0, 29, 65535, 55, 0, 19, 65535, 65535, 58,
            1, 56320, 57344, 29, 65535, 66, 0, 36, 65535, 65535,
        ],
        66,
        35,
    ),
    (
        &[
            65535, 3, 2, 34, 35, 9, 38, 39, 14, 65535, 14, 0, 34, 65535, 65535, 19, 5, 18, 65535,
            0, 32, 34, 33, 59, 34, 60, 55296, 34, 55296, 56320, 60, 57344, 0, 34, 65535, 37, 6, 0,
            32, 34, 33, 59, 34, 59, 60, 55, 60, 55296, 34, 55296, 56320, 60, 57344, 0, 34, 65535,
            60, 0, 18, 65535, 65535, 63, 1, 56320, 57344, 34, 65535,
        ],
        66,
        33,
    ),
];

pub(super) fn content() -> &'static Parser {
    static PARSER: LazyLock<Parser> = LazyLock::new(|| parser([0, 1]));
    &PARSER
}

pub(super) fn attribute() -> &'static Parser {
    static PARSER: LazyLock<Parser> = LazyLock::new(|| parser([1, 7]));
    &PARSER
}

fn parser(top_rule: [u16; 2]) -> Parser {
    let mut tokenizers = Vec::from([Tokenizer::Internal(6), Tokenizer::Internal(7)]);
    tokenizers.extend(LOCAL_GROUPS.map(|(data, prec, otherwise)| {
        // None of these groups has a precedence list. Prefer the longest match.
        assert_eq!(data[prec], u16::MAX);
        Tokenizer::External(ExternalTokenizer::new(
            ExternalOptions::default(),
            move |input, _| local_scan(input, &data[..prec], otherwise),
        ))
    }));
    Parser::deserialize_with_hooks(
        GrammarSpec {
            tables: SerializedTables {
                version: 14,
                states: STATES,
                state_data: STATE_DATA,
                goto: GOTO,
                token_data: TOKEN_DATA,
            },
            node_names: NODE_NAMES,
            max_term: 36,
            repeat_node_count: 4,
            tokenizers: &tokenizers,
            top_rule,
            token_prec: 157,
        },
        &[],
    )
    .expect("valid tables for the pinned Vue grammar")
}

// A local group returns either a token at the current position or one `@else`
// token for the text before the next match. Use the runtime's metered input.
fn local_scan(
    input: &mut TokenInput<'_>,
    data: &[u16],
    otherwise: u16,
) -> Result<Option<ExternalToken>, ParseError> {
    let start = input.position();
    while input.peek(0)?.is_some() {
        if let Some(token) = local_match(input, data)? {
            return Ok(Some(if input.position() == start {
                token
            } else {
                input.token(otherwise)
            }));
        }
        input.advance(1)?;
    }
    Ok((input.position() > start).then(|| input.token(otherwise)))
}

fn local_match(input: &TokenInput<'_>, data: &[u16]) -> Result<Option<ExternalToken>, ParseError> {
    let mut state = 0;
    let mut length = 0;
    let mut token = None;
    loop {
        let accept_end = usize::from(data[state + 1]);
        if state + 3 < accept_end {
            token = Some(ExternalToken {
                term: data[state + 3],
                end: input.position() + length as usize,
            });
        }
        let edges = usize::from(data[state + 2]);
        let Some(unit) = input.peek(length)? else {
            break;
        };
        let Some(edge) = data[accept_end..accept_end + 3 * edges]
            .chunks_exact(3)
            .find(|edge| edge[0] <= unit && (edge[1] == 0 || unit < edge[1]))
        else {
            break;
        };
        state = usize::from(edge[2]);
        length += 1;
    }
    Ok(token)
}

#[cfg(test)]
mod tests {
    use super::{attribute, content};

    #[test]
    fn parses_both_pinned_grammar_entry_points() {
        let source = "🦀 {{ é }} &amp;";
        let tree = content().parse(source).unwrap();
        let root = tree.root();
        assert_eq!(root.name(), "Content");
        assert_eq!(
            root.children()
                .map(|node| (node.name(), &source[node.range()]))
                .collect::<Vec<_>>(),
            [
                ("Text", "🦀 "),
                ("Interpolation", "{{ é }}"),
                ("Text", " "),
                ("Entity", "&amp;")
            ]
        );

        let source = r#"v-bind:prop.camel='é'"#;
        let tree = attribute().parse(source).unwrap();
        let root = tree.root();
        assert_eq!(root.name(), "Attribute");
        assert_eq!(
            root.children()
                .map(|node| (node.name(), &source[node.range()]))
                .collect::<Vec<_>>(),
            [
                ("VueAttributeName", "v-bind"),
                (":", ":"),
                ("Identifier", "prop"),
                ("Identifier", "camel"),
                ("Is", "="),
                ("ScriptAttributeValue", "'é'")
            ]
        );
    }
}
