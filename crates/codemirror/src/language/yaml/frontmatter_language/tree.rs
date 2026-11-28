//! Outer wrapper grammar from `@codemirror/lang-yaml` 6.1.3.
//! See `docs/licenses/codemirror-lang-yaml.txt`.

use std::sync::LazyLock;

use crate::language::{
    lezer::{GrammarSpec, ParseError, Parser, SerializedTables, SyntaxTree},
    line_projection::NormalizedInput,
};

static PARSER: LazyLock<Parser> = LazyLock::new(|| {
    Parser::deserialize(GrammarSpec {
        tables: SerializedTables {
            version: 14,
            states: "!vOQOPOOO]OPO'#C_OhOPO'#C^OOOO'#Cc'#CcOpOPO'#CaQOOOOOO{OPOOOOOO'#Cb'#CbO!WOPO'#C`O!`OPO,58xOOOO-E6a-E6aOOOO-E6`-E6`OOOO'#C_'#C_OOOO1G.d1G.d",
            state_data: "!h~OXPOYROWTP~OWVXXRXYRX~OYVOXSP~OXROYROWTX~OXROYROWTP~OYVOXSX~OX[O~OXY~",
            goto: "vWPPX[beioRUOQQOR]XRXQTTOUQWQRZWSSOURYS",
            token_data: "$z~RXOYnYZ!^Z]n]^!^^}n}!O!i!O;'Sn;'S;=`!c<%lOn~qXOYnYZ!^Z]n]^!^^;'Sn;'S;=`!c<%l~n~On~~!^~!cOY~~!fP;=`<%ln~!lZOYnYZ!^Z]n]^!^^}n}!O#_!O;'Sn;'S;=`!c<%l~n~On~~!^~#bZOYnYZ!^Z]n]^!^^}n}!O$T!O;'Sn;'S;=`!c<%l~n~On~~!^~$WXOYnYZ$sZ]n]^$s^;'Sn;'S;=`!c<%l~n~On~~$s~$zOX~Y~",
        },
        node_names: "⚠ Document Frontmatter DashLine FrontmatterContent Body",
        max_term: 10,
        repeat_node_count: 2,
        tokenizers: &[0],
        top_rule: [0, 1],
        token_prec: 67,
    })
    .expect("valid tables for the pinned YAML wrapper grammar")
});

pub(super) fn parse(source: &str) -> Result<SyntaxTree, ParseError> {
    let input = NormalizedInput::new(source);
    let mut tree = match PARSER.parse(&input.text) {
        Err(ParseError::Ambiguous { .. } | ParseError::UnexpectedToken { .. }) => {
            PARSER.parse_recovering(&input.text)
        }
        result => result,
    }
    .map_err(|error| error.map_offset(|at| input.original_byte(at)))?;
    if input.text.len() != source.len() {
        tree.project_glr_ranges(|at| input.original_byte(at));
    }
    Ok(tree)
}
