use super::{StringStream, word};

#[derive(Clone, Debug, Default)]
pub(super) struct State {
    block_comment: bool,
}

impl State {
    pub(super) fn token(
        &mut self,
        stream: &mut StringStream<'_>,
        arm: bool,
    ) -> Option<&'static str> {
        if self.block_comment {
            return Some(self.comment(stream));
        }
        if stream.eat_space() {
            return None;
        }

        let ch = stream.next()?;
        if ch == if arm { '@' } else { '#' } {
            stream.skip_to_end();
            return Some("comment");
        }
        match ch {
            '/' if stream.eat('*').is_some() => Some(self.comment(stream)),
            '#' if arm => {
                stream.eat_while(word);
                Some("number")
            }
            '"' => {
                let mut escaped = false;
                for ch in stream {
                    if ch == '"' && !escaped {
                        break;
                    }
                    escaped = !escaped && ch == '\\';
                }
                Some("string")
            }
            '.' => {
                stream.eat_while(word);
                let directive = stream.current().to_ascii_lowercase();
                DIRECTIVES
                    .binary_search(&directive.as_str())
                    .is_ok()
                    .then_some("builtin")
            }
            '=' => {
                stream.eat_while(word);
                Some("tag")
            }
            '{' | '}' => Some("bracket"),
            '0'..='9' => {
                if ch == '0' && stream.eat('x').is_some() {
                    stream.eat_while(|ch| ch.is_ascii_hexdigit());
                } else {
                    stream.eat_while(|ch| ch.is_ascii_digit());
                }
                Some("number")
            }
            _ if word(ch) => {
                stream.eat_while(word);
                if stream.eat(':').is_some() {
                    return Some("tag");
                }
                register(stream.current(), arm)
            }
            _ => None,
        }
    }

    fn comment(&mut self, stream: &mut StringStream<'_>) -> &'static str {
        if stream.skip_to("*/") {
            stream.match_str("*/", true, false);
            self.block_comment = false;
        } else {
            stream.skip_to_end();
            self.block_comment = true;
        }
        "comment"
    }
}

fn register(text: &str, arm: bool) -> Option<&'static str> {
    let text = text.to_ascii_lowercase();
    if arm {
        match text.as_str() {
            "sp" | "lr" | "pc" | "r13" | "r14" | "r15" => Some("variableName.special"),
            "r0" | "r1" | "r2" | "r3" | "r4" | "r5" | "r6" | "r7" | "r8" | "r9" | "r10" | "r11"
            | "r12" => Some("variable"),
            _ => None,
        }
    } else {
        match text.as_str() {
            "al" | "ah" | "ax" | "bl" | "bh" | "bx" | "cl" | "ch" | "cx" | "dl" | "dh" | "dx"
            | "si" | "di" | "sp" | "bp" | "ip" => Some("variable"),
            "eax" | "rax" | "ebx" | "rbx" | "ecx" | "rcx" | "edx" | "rdx" | "esi" | "rsi"
            | "edi" | "rdi" | "esp" | "rsp" | "ebp" | "rbp" | "eip" | "rip" => {
                Some("variableName.special")
            }
            "cs" | "ds" | "ss" | "es" | "fs" | "gs" => Some("keyword"),
            _ => None,
        }
    }
}

const DIRECTIVES: &[&str] = &[
    ".abort",
    ".align",
    ".altmacro",
    ".ascii",
    ".asciz",
    ".balign",
    ".balignl",
    ".balignw",
    ".bundle_align_mode",
    ".bundle_lock",
    ".bundle_unlock",
    ".byte",
    ".cfi_startproc",
    ".comm",
    ".data",
    ".def",
    ".desc",
    ".dim",
    ".double",
    ".eject",
    ".else",
    ".elseif",
    ".end",
    ".endef",
    ".endfunc",
    ".endif",
    ".equ",
    ".equiv",
    ".eqv",
    ".err",
    ".error",
    ".exitm",
    ".extern",
    ".fail",
    ".file",
    ".fill",
    ".float",
    ".func",
    ".global",
    ".gnu_attribute",
    ".hidden",
    ".hword",
    ".ident",
    ".if",
    ".incbin",
    ".include",
    ".int",
    ".internal",
    ".irp",
    ".irpc",
    ".lcomm",
    ".lflags",
    ".line",
    ".linkonce",
    ".list",
    ".ln",
    ".loc",
    ".loc_mark_labels",
    ".local",
    ".long",
    ".macro",
    ".mri",
    ".noaltmacro",
    ".nolist",
    ".octa",
    ".offset",
    ".org",
    ".p2align",
    ".popsection",
    ".previous",
    ".print",
    ".protected",
    ".psize",
    ".purgem",
    ".pushsection",
    ".quad",
    ".reloc",
    ".rept",
    ".sbttl",
    ".scl",
    ".section",
    ".set",
    ".short",
    ".single",
    ".size",
    ".skip",
    ".sleb128",
    ".space",
    ".stab",
    ".string",
    ".struct",
    ".subsection",
    ".symver",
    ".tag",
    ".text",
    ".title",
    ".type",
    ".uleb128",
    ".val",
    ".version",
    ".vtable_entry",
    ".vtable_inherit",
    ".warning",
    ".weak",
    ".weakref",
    ".word",
];
