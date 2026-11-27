// Replay with node and the directory of SHA-512-pinned archives.
import {captureQuoteInput} from './quote_input_fixture.mjs';

const cases = [
    ['inside double quotes', 'fn f() { let s = "😀| "; }', '"'],
    ['inside byte string', 'fn f() { let s = b"x | "; }', '"'],
    ['inside raw string', 'fn f() { let s = r"x | "; }', '"'],
    ['inside raw byte string', 'fn f() { let s = br"x | "; }', '"'],
    ['raw hash delimiter', 'fn f() { let s = r#"甲 | "#; }', '"'],
    ['after an escape', 'fn f() { let s = "\\"| "; }', '"'],
    ['CRLF and Unicode', 'fn f() {\r\n let s = "\\\r\n 😀| ";\r\n}', '"'],
    ['before a parsed opener', 'fn f() { let s = |"字"; }', '"'],
    ['before an untracked closer', 'fn f() { let s = "x|"; }', '"'],
    ['immediately after the literal', 'fn f() { let s = "x"|; }', '"'],
    ['after space outside the literal', 'fn f() { let s = "x" |; }', '"'],
    ['inside a character', "fn f() { let c = '| '; }", "'"],
    ['after a lifetime', "fn f<'a|>() {}", "'"],
    ['before a byte prefix literal', 'fn f() { let s = |b"x"; }', '"'],
    ['after a byte prefix', 'fn f() { let s = b|; }', '"'],
    ['line comment with a quote', 'fn f() { // " |\r\n}', '"'],
    ['unfinished string', 'fn f() { let s = "abc |;', '"'],
];
await captureQuoteInput('rust', cases);
