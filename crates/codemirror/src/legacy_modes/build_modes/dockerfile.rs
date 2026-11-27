use std::{collections::VecDeque, sync::LazyLock};

use regex::Regex;

use crate::stream_parser::StringStream;

type Style = Option<&'static str>;
const PLAIN: Style = None;
const COMMENT: Style = Some("comment");
const KEYWORD: Style = Some("keyword");
const ERROR: Style = Some("error");
const STRING: Style = Some("string");
const NUMBER: Style = Some("number");

#[derive(Clone, Debug, Default)]
pub(super) struct State {
    scan: Scan,
    pending: VecDeque<(usize, Style)>,
}

#[derive(Clone, Copy, Debug, Default)]
enum Scan {
    #[default]
    Start,
    From,
    Single,
    Double,
    Array,
    Expose,
    Arguments,
}

impl State {
    pub(super) fn token(&mut self, stream: &mut StringStream<'_>) -> Style {
        if let Some((len, style)) = self.pending.pop_front() {
            stream.set_pos(stream.pos() + len).unwrap();
            return style;
        }
        if matches!(self.scan, Scan::Start) && !stream.sol() {
            stream.next();
            return None;
        }
        for rule in self.scan.rules() {
            if rule.sol && !stream.sol() {
                continue;
            }
            let Some(matched) = stream.match_regex(&rule.pattern, false) else {
                continue;
            };
            if let Some(scan) = rule.next {
                self.scan = scan;
            }
            let first_len = if rule.styles.len() > 1 {
                // As in simple-mode, the captured groups are distinct tokens.
                for (group, &style) in rule.styles.iter().enumerate().skip(1) {
                    if let Some(text) = matched.get(group + 1)
                        && !text.is_empty()
                    {
                        self.pending.push_back((text.len(), style));
                    }
                }
                matched.get(1).map_or(0, |text| text.len())
            } else {
                matched.get(0).unwrap().len()
            };
            stream.set_pos(stream.pos() + first_len).unwrap();
            return rule.styles[0];
        }
        match self.scan {
            Scan::From | Scan::Expose | Scan::Arguments => self.scan = Scan::Start,
            _ => {
                stream.next();
            }
        }
        None
    }
}

struct Rule {
    pattern: Regex,
    styles: &'static [Style],
    next: Option<Scan>,
    sol: bool,
}

impl Rule {
    fn new(pattern: &str, styles: &'static [Style], next: Option<Scan>) -> Self {
        const SPACE: &str = r"\t-\r \u{a0}\u{1680}\u{2000}-\u{200a}\u{2028}\u{2029}\u{202f}\u{205f}\u{3000}\u{feff}";
        let pattern = pattern
            .replace(r"\s", &format!("[{SPACE}]"))
            .replace(r"\S", &format!("[^{SPACE}]"));
        Self {
            pattern: Regex::new(&pattern).unwrap(),
            styles,
            next,
            sol: false,
        }
    }

    fn sol(mut self) -> Self {
        self.sol = true;
        self
    }
}

impl Scan {
    fn rules(self) -> &'static [Rule] {
        static RULES: LazyLock<[Vec<Rule>; 7]> = LazyLock::new(|| {
            use Scan::{Arguments, Array, Double, Expose, From, Single, Start};
            let rule = Rule::new;
            const INSTRUCTIONS: &str = "from|expose|run|cmd|entrypoint|shell|arg|maintainer|label|env|add|copy|volume|user|workdir|onbuild|stopsignal|healthcheck";
            const COMMENT_LINE: &str = r"^\s*#[^\r\n\u{2028}\u{2029}]*$";
            [
                vec![
                    rule(COMMENT_LINE, &[COMMENT], None),
                    rule(
                        r"^(\s*)(?-u:\b)((?i-u:from))(?-u:\b)",
                        &[PLAIN, KEYWORD],
                        Some(From),
                    ),
                    rule(
                        &format!(
                            r"^(\s*)((?i-u:{INSTRUCTIONS}))(\s*)(#[^\r\n\u{{2028}}\u{{2029}}]*)?$"
                        ),
                        &[PLAIN, KEYWORD, PLAIN, ERROR],
                        None,
                    ),
                    rule(
                        r"^(\s*)((?i-u:run|cmd|entrypoint|shell))(\s+\[)",
                        &[PLAIN, KEYWORD, PLAIN],
                        Some(Array),
                    ),
                    rule(
                        r"^(\s*)((?i-u:expose))(\s+)",
                        &[PLAIN, KEYWORD, PLAIN],
                        Some(Expose),
                    ),
                    rule(
                        &format!(r"^(\s*)((?i-u:{INSTRUCTIONS}))(\s+)"),
                        &[PLAIN, KEYWORD, PLAIN],
                        Some(Arguments),
                    ),
                ],
                vec![
                    rule(r"^\s*$", &[PLAIN], Some(Start)),
                    rule(
                        r"^(\s*)(#[^\r\n\u{2028}\u{2029}]*)$",
                        &[PLAIN, ERROR],
                        Some(Start),
                    ),
                    rule(r"^(\s*\S+\s+)((?i-u:as))", &[PLAIN, KEYWORD], Some(Start)),
                ],
                vec![
                    rule(r"^(?:[^\\']|\\[^\r\n\u{2028}\u{2029}])", &[STRING], None),
                    rule(r"^'", &[STRING], Some(Arguments)),
                ],
                vec![
                    rule(r#"^(?:[^\\"]|\\[^\r\n\u{2028}\u{2029}])"#, &[STRING], None),
                    rule(r#"^""#, &[STRING], Some(Arguments)),
                ],
                vec![
                    rule(r"^\]", &[PLAIN], Some(Start)),
                    rule(
                        r#"^"(?:[^\\"]|\\[^\r\n\u{2028}\u{2029}])*"?"#,
                        &[STRING],
                        None,
                    ),
                ],
                vec![
                    rule(r"^[0-9]+$", &[NUMBER], Some(Start)),
                    rule(r"^[^0-9]+$", &[PLAIN], Some(Start)),
                    rule(r"^[0-9]+", &[NUMBER], None),
                    rule(r"^[^0-9]+", &[PLAIN], None),
                ],
                vec![
                    rule(COMMENT_LINE, &[COMMENT], None).sol(),
                    rule(
                        r#"^"(?:[^\\"]|\\[^\r\n\u{2028}\u{2029}])*"?$"#,
                        &[STRING],
                        Some(Start),
                    ),
                    rule(r#"^""#, &[STRING], Some(Double)),
                    rule(
                        r"^'(?:[^\\']|\\[^\r\n\u{2028}\u{2029}])*'?$",
                        &[STRING],
                        Some(Start),
                    ),
                    rule(r"^'", &[STRING], Some(Single)),
                    rule(r#"^[^#"']+[\\`]$"#, &[PLAIN], None),
                    rule(r#"^[^#"']+$"#, &[PLAIN], Some(Start)),
                    rule(r#"^[^#"']+"#, &[PLAIN], None),
                ],
            ]
        });
        &RULES[self as usize]
    }
}
