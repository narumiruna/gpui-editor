use std::sync::LazyLock;

use regex::Regex;

use super::{StringStream, ascii_word, eat_str, remaining};

// Keep this order: the upstream boundary is tested after each alternative.
static KEYWORD: LazyLock<Regex> = LazyLock::new(|| {
    bounded(concat!(
        "begin|break|catch|continue|data|default|do|dynamicparam|else|elseif|end|exit|",
        "filter|finally|for|foreach|from|function|if|in|param|process|return|switch|",
        "throw|trap|try|until|where|while",
    ))
});
static NUMBER: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(
    r"(?i-u:^(?:0x[0-9a-f]+|(?:[0-9]+\.[0-9]+|[0-9]\.|\.[0-9]+|[0-9]+)(?:e[+-]?[0-9]+)?)[ld]?(?:[kmgtp]b)?)"
).unwrap()
});
static OPERATOR: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(concat!(
        r"(?i-u:^(?:-(?:f|b?not|[ic]?split|join|is(?:not)?|as|[ic]?(?:eq|ne|[gl][te])|",
        r"[ic]?(?:not)?(?:like|match|contains)|[ic]?replace|b?(?:and|or|xor))\b|",
        r"[+*/%\-]=|\+\+|--|\.\.|[+*&^%:=!|/\-]|[<>]))",
    ))
    .unwrap()
});
static PUNCTUATION: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"^(?:[\[\]{},;`\\.]|@[({])").unwrap());
static IDENTIFIER: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"(?-u:^[A-Za-z_][A-Za-z0-9_\-]*\b)").unwrap());
static BUILTIN: LazyLock<Regex> = LazyLock::new(|| {
    bounded(concat!(
        r"[A-Z]:|%|\?|",
        "Add-(Computer|Content|History|Member|PSSnapin|Type)|Checkpoint-Computer|",
        "Clear-(Content|EventLog|History|Host|Item(Property)?|Variable)|Compare-Object|",
        "Complete-Transaction|Connect-PSSession|ConvertFrom-(Csv|Json|SecureString|StringData)|Convert-Path|",
        "ConvertTo-(Csv|Html|Json|SecureString|Xml)|Copy-Item(Property)?|Debug-Process|",
        "Disable-(ComputerRestore|PSBreakpoint|PSRemoting|PSSessionConfiguration)|Disconnect-PSSession|",
        "Enable-(ComputerRestore|PSBreakpoint|PSRemoting|PSSessionConfiguration)|(Enter|Exit)-PSSession|",
        "Export-(Alias|Clixml|Console|Counter|Csv|FormatData|ModuleMember|PSSession)|ForEach-Object|",
        "Format-(Custom|List|Table|Wide)|",
        "Get-(Acl|Alias|AuthenticodeSignature|ChildItem|Command|ComputerRestorePoint|Content|ControlPanelItem|Counter|Credential",
        "|Culture|Date|Event|EventLog|EventSubscriber|ExecutionPolicy|FormatData|Help|History|Host|HotFix|Item|ItemProperty|Job",
        "|Location|Member|Module|PfxCertificate|Process|PSBreakpoint|PSCallStack|PSDrive|PSProvider|PSSession|PSSessionConfiguration",
        "|PSSnapin|Random|Service|TraceSource|Transaction|TypeData|UICulture|Unique|Variable|Verb|WinEvent|WmiObject)|",
        "Group-Object|Import-(Alias|Clixml|Counter|Csv|LocalizedData|Module|PSSession)|ImportSystemModules|",
        "Invoke-(Command|Expression|History|Item|RestMethod|WebRequest|WmiMethod)|Join-Path|Limit-EventLog|",
        "Measure-(Command|Object)|Move-Item(Property)?|",
        "New-(Alias|Event|EventLog|Item(Property)?|Module|ModuleManifest|Object|PSDrive|PSSession|PSSessionConfigurationFile",
        "|PSSessionOption|PSTransportOption|Service|TimeSpan|Variable|WebServiceProxy|WinEvent)|",
        "Out-(Default|File|GridView|Host|Null|Printer|String)|Pause|(Pop|Push)-Location|Read-Host|Receive-(Job|PSSession)|",
        "Register-(EngineEvent|ObjectEvent|PSSessionConfiguration|WmiEvent)|",
        "Remove-(Computer|Event|EventLog|Item(Property)?|Job|Module|PSBreakpoint|PSDrive|PSSession|PSSnapin|TypeData|Variable|WmiObject)|",
        "Rename-(Computer|Item(Property)?)|Reset-ComputerMachinePassword|Resolve-Path|Restart-(Computer|Service)|",
        "Restore-Computer|Resume-(Job|Service)|Save-Help|Select-(Object|String|Xml)|Send-MailMessage|",
        "Set-(Acl|Alias|AuthenticodeSignature|Content|Date|ExecutionPolicy|Item(Property)?|Location|PSBreakpoint|PSDebug",
        "|PSSessionConfiguration|Service|StrictMode|TraceSource|Variable|WmiInstance)|",
        "Show-(Command|ControlPanelItem|EventLog)|Sort-Object|Split-Path|Start-(Job|Process|Service|Sleep|Transaction|Transcript)|",
        "Stop-(Computer|Job|Process|Service|Transcript)|Suspend-(Job|Service)|TabExpansion2|Tee-Object|",
        "Test-(ComputerSecureChannel|Connection|ModuleManifest|Path|PSSessionConfigurationFile)|Trace-Command|Unblock-File|",
        "Undo-Transaction|Unregister-(Event|PSSessionConfiguration)|Update-(FormatData|Help|List|TypeData)|",
        "Use-Transaction|Wait-(Event|Job|Process)|Where-Object|Write-(Debug|Error|EventLog|Host|Output|Progress|Verbose|Warning)|",
        "cd|help|mkdir|more|oss|prompt|ac|asnp|cat|cd|chdir|clc|clear|clhy|cli|clp|cls|clv|cnsn|compare|copy|cp|cpi|cpp|cvpa|dbp|del|diff|dir|dnsn|ebp|",
        "echo|epal|epcsv|epsn|erase|etsn|exsn|fc|fl|foreach|ft|fw|gal|gbp|gc|gci|gcm|gcs|gdr|ghy|gi|gjb|gl|gm|gmo|gp|gps|",
        "group|gsn|gsnp|gsv|gu|gv|gwmi|h|history|icm|iex|ihy|ii|ipal|ipcsv|ipmo|ipsn|irm|ise|iwmi|iwr|kill|lp|ls|man|md|",
        "measure|mi|mount|move|mp|mv|nal|ndr|ni|nmo|npssc|nsn|nv|ogv|oh|popd|ps|pushd|pwd|r|rbp|rcjb|rcsn|rd|rdr|ren|ri|",
        "rjb|rm|rmdir|rmo|rni|rnp|rp|rsn|rsnp|rujb|rv|rvpa|rwmi|sajb|sal|saps|sasv|sbp|sc|select|set|shcm|si|sl|sleep|sls|",
        "sort|sp|spjb|spps|spsv|start|sujb|sv|swmi|tee|trcm|type|where|wjb|write|",
        r"\$([$?^_]|Args|ConfirmPreference|ConsoleFileName|DebugPreference|Error|ErrorActionPreference|ErrorView|ExecutionContext",
        "|FormatEnumerationLimit|Home|Host|Input|MaximumAliasCount|MaximumDriveCount|MaximumErrorCount|MaximumFunctionCount",
        "|MaximumHistoryCount|MaximumVariableCount|MyInvocation|NestedPromptLevel|OutputEncoding|Pid|Profile|ProgressPreference",
        "|PSBoundParameters|PSCommandPath|PSCulture|PSDefaultParameterValues|PSEmailServer|PSHome|PSScriptRoot|PSSessionApplicationName",
        "|PSSessionConfigurationName|PSSessionOption|PSUICulture|PSVersionTable|Pwd|ShellId|StackTrace|VerbosePreference",
        "|WarningPreference|WhatIfPreference|Event|EventArgs|EventSubscriber|Sender|Matches|Ofs|ForEach|LastExitCode|PSCmdlet",
        "|PSItem|PSSenderInfo|This|true|false|null)",
    ))
});

fn bounded(alternatives: &str) -> Regex {
    // Rust regexes do not have lookahead. Capture the word but not its boundary.
    Regex::new(&format!(r"^((?i-u:{alternatives}))(?:[^A-Za-z0-9_-]|$)")).unwrap()
}

fn eat_bounded(stream: &mut StringStream<'_>, regex: &Regex) -> bool {
    let Some(captures) = stream.match_regex(regex, false) else {
        return false;
    };
    stream
        .set_pos(stream.pos() + captures.get(1).unwrap().end())
        .unwrap();
    true
}

#[derive(Clone, Debug, Default)]
pub(super) struct State {
    lexer: Lexer,
    returns: Vec<Return>,
    nesting: isize,
}

#[derive(Clone, Copy, Debug, Default)]
enum Lexer {
    #[default]
    Base,
    Comment,
    VariableBraces,
    DoubleQuote,
    Here(char),
    Interpolation(Parent),
}

#[derive(Clone, Copy, Debug)]
enum Parent {
    DoubleQuote,
    HereString,
}

#[derive(Clone, Copy, Debug)]
struct Return {
    parent: Parent,
    nesting: Option<isize>,
}

impl State {
    pub(super) fn token(&mut self, stream: &mut StringStream<'_>) -> Option<&'static str> {
        match self.lexer {
            Lexer::Base => self.base(stream),
            Lexer::Comment => Some(self.comment(stream)),
            Lexer::VariableBraces => Some(self.variable_braces(stream)),
            Lexer::DoubleQuote => Some(self.double_quote(stream)),
            Lexer::Here(quote) => Some(self.here(stream, quote)),
            Lexer::Interpolation(parent) => Some(self.interpolation(stream, parent)),
        }
    }

    fn base(&mut self, stream: &mut StringStream<'_>) -> Option<&'static str> {
        if let Some(frame) = self
            .returns
            .last()
            .filter(|frame| frame.nesting.is_none_or(|n| n == self.nesting))
        {
            let parent = frame.parent;
            self.returns.pop();
            return Some(match parent {
                Parent::DoubleQuote => {
                    self.lexer = Lexer::DoubleQuote;
                    self.double_quote(stream)
                }
                Parent::HereString => {
                    self.lexer = Lexer::Here('"');
                    self.here(stream, '"')
                }
            });
        }
        if stream.eat_space() {
            return None;
        }
        if stream.eat('(').is_some() {
            self.nesting += 1;
            return Some("punctuation");
        }
        if stream.eat(')').is_some() {
            self.nesting -= 1;
            return Some("punctuation");
        }
        if eat_bounded(stream, &KEYWORD) {
            return Some("keyword");
        }
        if stream.match_regex(&NUMBER, true).is_some() {
            return Some("number");
        }
        if !remaining(stream).starts_with("<#") && stream.match_regex(&OPERATOR, true).is_some() {
            return Some("operator");
        }
        if eat_bounded(stream, &BUILTIN) {
            return Some("builtin");
        }
        if stream.match_regex(&PUNCTUATION, true).is_some() {
            return Some("punctuation");
        }
        if stream.match_regex(&IDENTIFIER, true).is_some() {
            return Some("variable");
        }
        Some(match stream.next() {
            Some('\'') => self.single_quote(stream),
            Some('$') => self.variable(stream),
            Some('"') => self.double_quote(stream),
            Some('<') if stream.eat('#').is_some() => {
                self.lexer = Lexer::Comment;
                self.comment(stream)
            }
            Some('#') => {
                stream.skip_to_end();
                "comment"
            }
            Some('@') => {
                let quote = stream.eat_if(|ch| matches!(ch, '\'' | '"'));
                if let Some(quote) = quote.filter(|_| stream.eol()) {
                    self.lexer = Lexer::Here(quote);
                    self.here(stream, quote)
                } else if matches!(stream.peek(), Some('(' | '{')) {
                    "punctuation"
                } else if stream.peek().is_some_and(variable_char) {
                    self.variable(stream)
                } else {
                    "error"
                }
            }
            _ => "error",
        })
    }

    fn single_quote(&mut self, stream: &mut StringStream<'_>) -> &'static str {
        while let Some(ch) = stream.next() {
            if ch == '\'' && stream.eat('\'').is_none() {
                self.lexer = Lexer::Base;
                return "string";
            }
        }
        "error"
    }

    fn double_quote(&mut self, stream: &mut StringStream<'_>) -> &'static str {
        while let Some(ch) = stream.peek() {
            if ch == '$' {
                self.lexer = Lexer::Interpolation(Parent::DoubleQuote);
                return "string";
            }
            stream.next();
            if ch == '`' {
                stream.next();
            } else if ch == '"' && stream.eat('"').is_none() {
                self.lexer = Lexer::Base;
                return "string";
            }
        }
        "error"
    }

    fn interpolation(&mut self, stream: &mut StringStream<'_>, parent: Parent) -> &'static str {
        if eat_str(stream, "$(") {
            self.returns.push(Return {
                parent,
                nesting: Some(self.nesting),
            });
            self.lexer = Lexer::Base;
            self.nesting += 1;
            "punctuation"
        } else {
            stream.next();
            self.returns.push(Return {
                parent,
                nesting: None,
            });
            self.variable(stream)
        }
    }

    fn comment(&mut self, stream: &mut StringStream<'_>) -> &'static str {
        if stream.skip_to("#>") {
            eat_str(stream, "#>");
            self.lexer = Lexer::Base;
        } else {
            stream.skip_to_end();
        }
        "comment"
    }

    fn variable(&mut self, stream: &mut StringStream<'_>) -> &'static str {
        if stream.eat('{').is_some() {
            self.lexer = Lexer::VariableBraces;
            return self.variable_braces(stream);
        }
        self.lexer = Lexer::Base;
        if stream.eat_while(variable_char) {
            "variable"
        } else {
            "error"
        }
    }

    fn variable_braces(&mut self, stream: &mut StringStream<'_>) -> &'static str {
        if stream.skip_to("}") {
            stream.next();
            self.lexer = Lexer::Base;
        } else {
            stream.skip_to_end();
        }
        "variable"
    }

    fn here(&mut self, stream: &mut StringStream<'_>, quote: char) -> &'static str {
        let closing = if quote == '"' { "\"@" } else { "'@" };
        if stream.sol() && eat_str(stream, closing) {
            self.lexer = Lexer::Base;
        } else if quote == '"' {
            while let Some(ch) = stream.peek() {
                if ch == '$' {
                    self.lexer = Lexer::Interpolation(Parent::HereString);
                    break;
                }
                stream.next();
                if ch == '`' {
                    stream.next();
                }
            }
        } else {
            stream.skip_to_end();
        }
        "string"
    }
}

fn variable_char(ch: char) -> bool {
    ascii_word(ch) || matches!(ch, '-' | ':')
}
