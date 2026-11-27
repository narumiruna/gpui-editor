//! Recover one tag at a time when the strict Lezer runtime cannot finish.

use super::{
    LiquidNodeKind::{self, *},
    parser::{Arena, parse_tree},
};

pub(super) fn parse(source: &str) -> Arena {
    let mut arena = Arena::default();
    arena.add(Template, 0..source.len(), None);
    let mut open: Vec<(usize, &str)> = Vec::new();
    let mut at = 0;
    while at < source.len() {
        let (parent, raw) = open.last().map_or((0, None), |&(id, word)| {
            let raw = match word {
                "endraw" => Some(RawText),
                "endcomment" => Some(CommentText),
                _ => None,
            };
            (id, raw)
        });
        let close_raw = open.last().map(|entry| entry.1).filter(|_| raw.is_some());
        let next = find_start(source, at, close_raw);
        if at < next {
            arena.leaf(raw.unwrap_or(Text), at..next, parent);
        }
        at = next;
        if at == source.len() {
            break;
        }
        let interpolation = source[at..].starts_with("{{");
        let delimiter = if interpolation { "}}" } else { "%}" };
        let prefix = if interpolation {
            2
        } else {
            tag_prefix(&source[at..])
        };
        let start = at + prefix;
        let (end, closed) = tag_end(source, start, delimiter);
        let input = &source[at..end];
        let word = if interpolation {
            ""
        } else {
            command(&input[prefix..])
        };
        let part = TagRegion {
            at,
            input,
            prefix,
            closed,
            word,
        };
        if !interpolation && word.starts_with("end") {
            if let Some(depth) = open
                .iter()
                .rev()
                .take(64)
                .position(|&(_, expected)| expected == word)
            {
                for _ in 0..depth {
                    finish(&mut arena, open.pop().unwrap().0, at);
                }
                let (id, _) = open.pop().unwrap();
                part.parse(&mut arena, EndTag, None, id);
                arena.0[id].range.end = end;
            } else {
                let id = arena.leaf(EndTag, at..start, parent);
                arena.leaf(TagStart, at..start, id);
                arena.error(start, id);
                at = start;
                continue;
            }
        } else {
            let opening = directive(word);
            let parent = if let Some((kind, expected)) = opening {
                let id = arena.leaf(kind, at..end, parent);
                open.push((id, expected));
                id
            } else {
                parent
            };
            let kind = if interpolation { Interpolation } else { Tag };
            let close = opening.map(|(_, close)| close);
            let surrounding = open.last().map(|&(_, word)| word);
            part.parse(
                &mut arena,
                kind,
                close.or(surrounding.filter(|_| matches!(word, "elsif" | "else" | "when"))),
                parent,
            );
        }
        at = end;
    }
    while let Some((id, _)) = open.pop() {
        finish(&mut arena, id, source.len());
    }
    arena
}

fn finish(arena: &mut Arena, id: usize, end: usize) {
    arena.0[id].range.end = end;
    arena.error(end, id);
}

fn find_start(source: &str, at: usize, raw_close: Option<&str>) -> usize {
    source[at..]
        .match_indices('{')
        .find_map(|(relative, _)| {
            let start = at + relative;
            let next = &source[start + 1..];
            if let Some(expected) = raw_close {
                let rest = next.strip_prefix('%')?;
                let rest = rest
                    .strip_prefix('-')
                    .unwrap_or(rest)
                    .trim_start_matches([' ', '\n']);
                let word = rest.trim_start_matches(|ch: char| ch.is_ascii_alphabetic());
                return (&rest[..rest.len() - word.len()] == expected).then_some(start);
            }
            next.starts_with(['{', '%']).then_some(start)
        })
        .unwrap_or(source.len())
}

fn tag_prefix(source: &str) -> usize {
    let mut at = 2;
    loop {
        match source.as_bytes().get(at) {
            Some(b' ' | b'\n') => at += 1,
            Some(b'-') => return at + 1,
            _ => return 2,
        }
    }
}

fn tag_end(source: &str, start: usize, delimiter: &str) -> (usize, bool) {
    let interpolation = delimiter == "}}";
    let mut quote = None;
    let mut comment = false;
    if interpolation {
        let rest = source[start..].trim_start();
        if let Some(after) = rest.strip_prefix('-')
            && !after.starts_with(|ch: char| ch.is_ascii_digit())
        {
            return (source.len() - rest.len(), false);
        }
    }
    for (i, ch) in source[start..].char_indices() {
        let at = start + i;
        if let Some(mark) = quote {
            if ch == mark {
                quote = None;
            }
            continue;
        }
        if source[at..].starts_with(delimiter) {
            if interpolation && !comment && at > start && source.as_bytes()[at - 1] == b'-' {
                return (at - 1, false);
            }
            return (at + 2, true);
        }
        if comment {
            comment = ch != '\n';
        } else {
            if interpolation && ch == '.' && source[start..at].trim_end().ends_with('|') {
                return (at, false);
            }
            match ch {
                '"' | '\'' => quote = Some(ch),
                '#' => comment = true,
                '{' if matches!(source.as_bytes().get(at + 1), Some(b'{' | b'%')) => {
                    return (at, false);
                }
                _ => {}
            }
        }
    }
    (source.len(), false)
}

fn command(source: &str) -> &str {
    let mut rest = source.trim_start();
    while rest.starts_with('#') {
        let Some((_, next)) = rest.split_once('\n') else {
            return "";
        };
        rest = next.trim_start();
    }
    let end = rest
        .find(|ch: char| !(ch.is_alphanumeric() || matches!(ch, '_' | '-')))
        .unwrap_or(rest.len());
    &rest[..end]
}

fn directive(word: &str) -> Option<(LiquidNodeKind, &'static str)> {
    Some(match word {
        "if" => (IfDirective, "endif"),
        "unless" => (UnlessDirective, "endunless"),
        "case" => (CaseDirective, "endcase"),
        "for" => (ForDirective, "endfor"),
        "tablerow" => (TableDirective, "endtablerow"),
        "capture" => (CaptureDirective, "endcapture"),
        "raw" => (RawDirective, "endraw"),
        "comment" => (Comment, "endcomment"),
        _ => return None,
    })
}

fn closing_context(word: &str) -> Option<&'static str> {
    let opening = match word.strip_prefix("end").unwrap_or(word) {
        "elsif" | "else" => "if",
        "when" => "case",
        name => name,
    };
    Some(match opening {
        "if" => "{% if x %}",
        "unless" => "{% unless x %}",
        "case" => "{% case x %}",
        "for" => "{% for x in xs %}",
        "tablerow" => "{% tablerow x in xs %}",
        "capture" => "{% capture x %}",
        "raw" => "{% raw %}",
        "comment" => "{% comment %}",
        _ => return None,
    })
}

struct TagRegion<'a> {
    at: usize,
    input: &'a str,
    prefix: usize,
    closed: bool,
    word: &'a str,
}

impl TagRegion<'_> {
    fn parse(&self, arena: &mut Arena, kind: LiquidNodeKind, close: Option<&str>, parent: usize) {
        let Self {
            at,
            input,
            prefix,
            closed,
            word,
        } = *self;
        let end = at + input.len();
        let branch = matches!(word, "when" | "elsif" | "else");
        let before = if kind == EndTag {
            closing_context(word).unwrap_or("")
        } else if branch {
            closing_context(close.unwrap_or(word)).unwrap_or("")
        } else {
            ""
        };
        let close = close.or(match word {
            "when" => Some("endcase"),
            "elsif" | "else" => Some("endif"),
            _ => None,
        });
        let (open_kind, close_kind, delimiter) = if kind == Interpolation {
            (InterpolationStart, InterpolationEnd, "}}")
        } else {
            (TagStart, TagEnd, "%}")
        };
        let content_end = if closed {
            input.len()
                - if kind != Interpolation && input.len() >= prefix + 3 && input.ends_with("-%}") {
                    3
                } else {
                    2
                }
        } else {
            input.len()
        };
        let content = &input[prefix..content_end];
        let trailing = content.trim_end().ends_with(['.', '|', ':', ',', '=']);
        // At most two parses of one small region. An inserted token marks a
        // missing operand, property, or filter at its original zero-width range.
        if input.len() <= 8192 {
            for marker in [Some(""), trailing.then_some(" missing")]
                .into_iter()
                .flatten()
            {
                let mut parsed = String::from(before);
                parsed.push_str(&input[..content_end]);
                parsed.push_str(marker);
                parsed.push_str(&input[content_end..]);
                if !closed {
                    parsed.push_str(delimiter);
                }
                if let Some(close) = close {
                    parsed.push_str("{% ");
                    parsed.push_str(close);
                    parsed.push_str(" %}");
                }
                let Ok(tree) = parse_tree(&parsed) else {
                    continue;
                };
                let start = before.len();
                let root = tree.root();
                let matches = |node: &super::super::lezer::SyntaxNode<'_>| {
                    node.name() == kind.name() && node.range().start == start
                };
                let child = root
                    .children()
                    .find(matches)
                    .or_else(|| root.children().next()?.children().find(matches));
                if let Some(child) = child {
                    let edit = start + content_end;
                    let suffix = edit + marker.len();
                    let limit = start + input.len() + marker.len();
                    let project = |pos: usize| {
                        at + pos.saturating_sub(start + pos.saturating_sub(edit).min(marker.len()))
                    };
                    let id = arena
                        .copy(child, Some(parent), |range| {
                            (range.start < limit).then(|| {
                                let missing = !marker.is_empty()
                                    && range.start >= edit
                                    && range.end <= suffix;
                                (project(range.start)..project(range.end.min(limit)), missing)
                            })
                        })
                        .expect("selected tag starts in the source");
                    if !closed {
                        arena.0[id].range.end = end;
                        arena.error(end, id);
                    }
                    return;
                }
            }
        }
        let id = arena.leaf(kind, at..end, parent);
        arena.leaf(open_kind, at..at + prefix, id);
        let mut cursor = prefix;
        if !word.is_empty()
            && let Some(offset) = content.find(word)
        {
            let start = prefix + offset;
            let end = start + word.len();
            arena.leaf(keyword(word), at + start..at + end, id);
            cursor = end;
        }
        let rest = &input[cursor..content_end];
        let from = cursor + rest.len() - rest.trim_start().len();
        let to = cursor + rest.trim_end().len();
        if from < to {
            arena.leaf(Error, at + from..at + to, id);
        } else {
            arena.error(at + content_end, id);
        }
        if closed {
            arena.leaf(close_kind, at + content_end..end, id);
        }
    }
}

fn keyword(word: &str) -> LiquidNodeKind {
    match word {
        "if" => If,
        "elsif" => Elsif,
        "else" => Else,
        "endif" => EndIf,
        "unless" => Unless,
        "endunless" => EndUnless,
        "case" => Case,
        "when" => When,
        "endcase" => EndCase,
        "for" => For,
        "endfor" => EndFor,
        "tablerow" => TableRow,
        "endtablerow" => EndTableRow,
        "capture" => Capture,
        "endcapture" => EndCapture,
        "comment" => CommentKeyword,
        "endcomment" => EndComment,
        "raw" => Raw,
        "endraw" => EndRaw,
        "liquid" => Liquid,
        "echo" => Echo,
        "assign" => Assign,
        "cycle" => Cycle,
        "continue" => Continue,
        "break" => Break,
        "increment" => Increment,
        "decrement" => Decrement,
        "render" => Render,
        "include" => Include,
        _ => TagName,
    }
}
