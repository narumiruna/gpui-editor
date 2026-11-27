use std::sync::{Arc, LazyLock};

use super::{super::js_space, word};
use crate::{language::LanguageData, stream_parser::StringStream};

const ATOMS: &[&str] = &["true", "false"];
const KEYWORDS: &[&str] = &[
    "if", "then", "do", "else", "elif", "while", "until", "for", "in", "esac", "fi", "fin", "fil",
    "done", "exit", "set", "unset", "export", "function",
];
const COMMANDS: &[&str] = &[
    "ab", "awk", "bash", "beep", "cat", "cc", "cd", "chown", "chmod", "chroot", "clear", "cp",
    "curl", "cut", "diff", "echo", "find", "gawk", "gcc", "get", "git", "grep", "hg", "kill",
    "killall", "ln", "ls", "make", "mkdir", "openssl", "mv", "nc", "nl", "node", "npm", "ping",
    "ps", "restart", "rm", "rmdir", "sed", "service", "sh", "shopt", "shred", "source", "sort",
    "sleep", "ssh", "start", "stop", "su", "sudo", "svn", "tee", "telnet", "top", "touch", "vi",
    "vim", "wall", "wc", "wget", "who", "write", "yes", "zsh",
];

pub(super) fn data(data: LanguageData) -> LanguageData {
    static COMPLETIONS: LazyLock<Vec<&str>> =
        LazyLock::new(|| [ATOMS, KEYWORDS, COMMANDS].concat());
    data.with("autocomplete", COMPLETIONS.as_slice())
        .with("closeBrackets", &["(", "[", "{", "'", "\"", "`"][..])
}

#[derive(Clone, Debug, Default)]
pub(super) struct State {
    frames: Vec<Scan>,
}

#[derive(Clone, Debug)]
enum Scan {
    Quoted { open: char, style: &'static str },
    StringStart(char),
    Dollar,
    Heredoc(Arc<str>),
}

impl State {
    pub(super) fn token(&mut self, stream: &mut StringStream<'_>) -> Option<&'static str> {
        loop {
            match self.frames.last() {
                Some(&Scan::Quoted { open, style }) => {
                    if self.quoted(stream, open, style) {
                        return Some(style);
                    }
                }
                Some(&Scan::StringStart(quote)) => {
                    *self.frames.last_mut().unwrap() = Scan::Quoted {
                        open: quote,
                        style: "string",
                    };
                    stream.next();
                }
                Some(Scan::Dollar) => {
                    if self.frames.len() > 1 {
                        stream.eat('$');
                    }
                    let ch = stream.next();
                    let style = match ch {
                        Some('(') => "quote",
                        Some('{') => "def",
                        Some('\'' | '"') => "string",
                        _ => {
                            if !ch.is_some_and(|ch| ch.is_ascii_digit()) {
                                stream.eat_while(word);
                            }
                            self.frames.pop();
                            return Some("def");
                        }
                    };
                    *self.frames.last_mut().unwrap() = Scan::Quoted {
                        open: ch.unwrap(),
                        style,
                    };
                }
                Some(Scan::Heredoc(delimiter)) => {
                    if stream.sol() && stream.line() == delimiter.as_ref() {
                        self.frames.pop();
                    }
                    stream.skip_to_end();
                    return Some("string.special");
                }
                None => {
                    if stream.eat_space() {
                        return None;
                    }
                    let sol = stream.sol();
                    let ch = stream.next()?;
                    match ch {
                        '\'' | '"' | '`' => self.frames.push(Scan::Quoted {
                            open: ch,
                            style: if ch == '`' { "quote" } else { "string" },
                        }),
                        '$' => self.frames.push(Scan::Dollar),
                        '\\' => {
                            stream.next();
                            return None;
                        }
                        '#' => {
                            let style = if sol && stream.eat('!').is_some() {
                                "meta"
                            } else {
                                "comment"
                            };
                            stream.skip_to_end();
                            return Some(style);
                        }
                        '+' | '=' => return Some("operator"),
                        '-' => {
                            stream.eat('-');
                            stream.eat_while(word);
                            return Some("attribute");
                        }
                        '<' if stream.match_str("<<", true, false) => return Some("operator"),
                        '<' if stream.eat('<').is_some() => {
                            self.frames.push(Scan::Heredoc(heredoc_delimiter(stream)));
                            return Some("string.special");
                        }
                        _ => return base_word(stream, ch),
                    }
                }
            }
        }
    }

    // Return false when an inner group must continue the current token.
    fn quoted(&mut self, stream: &mut StringStream<'_>, open: char, style: &'static str) -> bool {
        let close = match open {
            '(' => ')',
            '{' => '}',
            _ => open,
        };
        let mut escaped = false;
        while let Some(ch) = stream.next() {
            if !escaped {
                if ch == close {
                    self.frames.pop();
                    break;
                }
                if ch == '$' && open != '\'' && stream.peek() != Some(close) {
                    stream.back_up(1).unwrap();
                    self.frames.push(Scan::Dollar);
                    break;
                }
                if open != close && ch == open {
                    self.frames.push(Scan::Quoted { open, style });
                    return false;
                }
                if matches!(ch, '\'' | '"') && !matches!(open, '\'' | '"') {
                    stream.back_up(1).unwrap();
                    self.frames.push(Scan::StringStart(ch));
                    break;
                }
            }
            escaped = !escaped && ch == '\\';
        }
        true
    }
}

fn base_word(stream: &mut StringStream<'_>, ch: char) -> Option<&'static str> {
    if ch.is_ascii_digit() {
        stream.eat_while(|ch| ch.is_ascii_digit());
        if !stream.peek().is_some_and(word) {
            return Some("number");
        }
    }
    stream.eat_while(|ch| word(ch) || ch == '-');
    let current = stream.current();
    if stream.peek() == Some('=') && current.chars().any(word) {
        Some("def")
    } else if ATOMS.contains(&current) {
        Some("atom")
    } else if KEYWORDS.contains(&current) {
        Some("keyword")
    } else if COMMANDS.contains(&current) {
        Some("builtin")
    } else {
        None
    }
}

fn heredoc_delimiter(stream: &mut StringStream<'_>) -> Arc<str> {
    stream.eat('-');
    stream.eat_space();
    let remaining = &stream.line()[stream.pos()..];
    let quote = ['\'', '"'];
    if remaining.starts_with(quote)
        && let Some(end) = remaining[1..].find(quote)
    {
        stream.set_pos(stream.pos() + end + 2).unwrap();
        return Arc::from(&remaining[1..end + 1]);
    }
    let start = stream.pos();
    stream.eat_while(|ch| !matches!(ch, '\'' | '"') && !js_space(ch));
    Arc::from(&stream.line()[start..stream.pos()])
}
