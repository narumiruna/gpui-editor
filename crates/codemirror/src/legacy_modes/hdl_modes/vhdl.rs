use std::{borrow::Cow, collections::HashSet, sync::LazyLock};

use super::{Contexts, Kind, bracket_close, identifier, word};
use crate::{legacy_modes::js_space, stream_parser::StringStream};

const KEYWORDS: &str = concat!(
    "abs,access,after,alias,all,and,architecture,array,assert,attribute,begin,block,",
    "body,buffer,bus,case,component,configuration,constant,disconnect,downto,else,elsif,end,end block,end case,",
    "end component,end for,end generate,end if,end loop,end process,end record,end units,entity,exit,file,for,",
    "function,generate,generic,generic map,group,guarded,if,impure,in,inertial,inout,is,label,library,linkage,",
    "literal,loop,map,mod,nand,new,next,nor,null,of,on,open,or,others,out,package,package body,port,port map,",
    "postponed,procedure,process,pure,range,record,register,reject,rem,report,return,rol,ror,select,severity,signal,",
    "sla,sll,sra,srl,subtype,then,to,transport,type,unaffected,units,until,use,variable,wait,when,while,with,xnor,xor",
);
const BLOCKS: &[&str] = &[
    "architecture",
    "entity",
    "begin",
    "case",
    "port",
    "else",
    "elsif",
    "end",
    "for",
    "function",
    "if",
];
const OPERATOR: &str = "&|~><!)(*#%@+/=?:;}{,.^-[]";

static KEYWORD_SET: LazyLock<HashSet<&str>> = LazyLock::new(|| KEYWORDS.split(',').collect());

#[derive(Clone, Debug)]
pub(super) struct State(Contexts);

impl State {
    pub(super) fn new(unit: usize) -> Self {
        Self(Contexts::new(unit))
    }

    pub(super) fn token(&mut self, stream: &mut StringStream<'_>) -> Option<&'static str> {
        let contexts = &mut self.0;
        contexts.start_line(stream);
        if stream.eat_space() {
            return None;
        }
        let ch = stream.next()?;
        let mut punctuation = None;
        let mut new_statement = false;
        let style = match ch {
            '`' | '$' => {
                stream.eat_while(identifier);
                return Some("meta");
            }
            '\'' | '"' => {
                // Upstream compares each individual character to "--", so neither
                // a backslash nor a dash escapes a quote. The string ends at EOL.
                stream.find(|&next| next == ch);
                Some(if ch == '"' {
                    "string.special"
                } else {
                    "string"
                })
            }
            '[' | ']' | '{' | '}' | '(' | ')' | ',' | ';' | ':' | '.' => {
                punctuation = Some(ch);
                None
            }
            '0'..='9' => {
                stream.eat_while(|ch| word(ch) || matches!(ch, '.' | '\''));
                Some("number")
            }
            '-' if stream.eat('-').is_some() => {
                stream.skip_to_end();
                return Some("comment");
            }
            _ if OPERATOR.contains(ch) => {
                stream.eat_while(|ch| OPERATOR.contains(ch));
                Some("operator")
            }
            _ => {
                stream.eat_while(identifier);
                let text = stream.current();
                let lower = if text.bytes().any(|ch| ch.is_ascii_uppercase()) {
                    Cow::Owned(text.to_ascii_lowercase())
                } else {
                    Cow::Borrowed(text)
                };
                if KEYWORD_SET.contains(lower.as_ref()) {
                    // Keyword highlighting ignores case. Contexts accept only
                    // lowercase, uppercase, or an uppercase first letter.
                    new_statement = BLOCKS.contains(&lower.as_ref())
                        && (text[1..] == lower[1..]
                            || text.bytes().all(|ch| ch.is_ascii_uppercase()));
                    Some("keyword")
                } else {
                    Some("variable")
                }
            }
        };
        contexts.set_alignment(true);
        let kind = contexts.current().kind;
        match punctuation {
            Some(';' | ':') if kind == Kind::Statement => contexts.pop(),
            Some(ch) if let Some(close) = bracket_close(ch) => {
                contexts.push(Kind::Bracket(close), stream)
            }
            Some('}') => {
                contexts.pop_statements();
                if contexts.current().kind == Kind::Bracket('}') {
                    contexts.pop();
                }
                contexts.pop_statements();
            }
            Some(ch) if kind == Kind::Bracket(ch) => contexts.pop(),
            _ if matches!(kind, Kind::Top | Kind::Bracket('}'))
                || (kind == Kind::Statement && new_statement) =>
            {
                contexts.push(Kind::Statement, stream)
            }
            _ => {}
        }
        style
    }

    pub(super) fn indent(&self, text_after: &str, unit: usize) -> usize {
        let ctx = self.0.current();
        let closing = matches!(ctx.kind, Kind::Bracket(ch) if text_after.starts_with(ch));
        if ctx.kind == Kind::Statement {
            ctx.indented
                .saturating_add(if text_after.starts_with('{') { 0 } else { unit })
        } else if ctx.align == Some(true) {
            ctx.column.saturating_add(usize::from(!closing))
        } else {
            self.0.indent_from(ctx, if closing { 0 } else { unit })
        }
    }
}

pub(super) fn indent_on_input(text: &str) -> bool {
    matches!(text.trim_start_matches(js_space), "{" | "}")
}
