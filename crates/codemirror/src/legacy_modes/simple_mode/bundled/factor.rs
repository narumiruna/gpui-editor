use super::{SimpleRule, groups, next, styled};

pub(super) fn states() -> [(&'static str, Vec<SimpleRule>); 6] {
    let keyword = Some("keyword");
    let def = Some("def");
    [
        (
            "start",
            vec![
                styled(r"#?!.*", "comment"),
                next(styled(r#"""""#, "string"), "string3"),
                next(groups(r"(STRING:)(\s)", &[keyword, None]), "string2"),
                next(styled(r#"\S*?""#, "string"), "string"),
                styled(
                    r"(?:0x[\d,a-f]+)|(?:0o[0-7]+)|(?:0b[0,1]+)|(?:-?\d+.?\d*)(?=\s)",
                    "number",
                ),
                next(
                    groups(
                        r"(GENERIC|:?:)(\s+)(\S+)(\s+)(\()",
                        &[keyword, None, def, None, Some("bracket")],
                    ),
                    "stack",
                ),
                groups(
                    r"(M:)(\s+)(\S+)(\s+)(\S+)",
                    &[keyword, None, def, None, Some("tag")],
                ),
                next(styled(r"USING:", "keyword"), "vocabulary"),
                groups(
                    r"(USE:|IN:)(\s+)(\S+)(?=\s|$)",
                    &[keyword, None, Some("tag")],
                ),
                groups(r"(\S+:)(\s+)(\S+)(?=\s|$)", &[keyword, None, def]),
                styled(
                    r"(?:;|\\|t|f|if|loop|while|until|do|PRIVATE>|<PRIVATE|\.|\S*\[|\]|\S*\{|\})(?=\s|$)",
                    "keyword",
                ),
                styled(r"\S+[)>.\*?]+(?=\s|$)", "builtin"),
                styled(r"[)><]+\S+(?=\s|$)", "builtin"),
                styled(r"[+\-=/\*<>](?=\s|$)", "keyword"),
                styled(r"\S+", "variable"),
                SimpleRule::new(r"\s+|."),
            ],
        ),
        (
            "vocabulary",
            vec![
                next(styled(";", "keyword"), "start"),
                styled(r"\S+", "tag"),
                SimpleRule::new(r"\s+|."),
            ],
        ),
        (
            "string",
            vec![
                next(styled(r#"(?:[^\\]|\\.)*?""#, "string"), "start"),
                styled(".*", "string"),
            ],
        ),
        (
            "string2",
            vec![
                next(styled("^;", "keyword"), "start"),
                styled(".*", "string"),
            ],
        ),
        (
            "string3",
            vec![
                next(styled(r#"(?:[^\\]|\\.)*?""""#, "string"), "start"),
                styled(".*", "string"),
            ],
        ),
        (
            "stack",
            vec![
                next(styled(r"\)", "bracket"), "start"),
                styled("--", "bracket"),
                styled(r"\S+", "meta"),
                SimpleRule::new(r"\s+|."),
            ],
        ),
    ]
}
