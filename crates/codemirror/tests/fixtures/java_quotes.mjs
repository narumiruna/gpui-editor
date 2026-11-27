// Replay with node and the directory of SHA-512-pinned archives.
import {captureQuoteInput} from './quote_input_fixture.mjs';

const cases = [
    ['inside-double', 'class A { String s = "😀| "; }', '"'],
    ['escape', 'class A { String s = "\\"| "; }', '"'],
    ['inside-block', 'class A {\r\n String s = """\r\n  甲😀| \r\n """;\r\n}', '"'],
    ['inside-character', "class A { char a = '| '; }", "'"],
    ['before-open', 'class A { String s = |"字"; }', '"'],
    ['inside-other-quote', 'class A { String s = "x | "; }', "'"],
    ['before-untracked-close', 'class A { String s = "x|"; }', '"'],
    ['after-close', 'class A { String s = "x"|; }', '"'],
    ['after-space', 'class A { String s = "x" |; }', '"'],
    ['line-comment', 'class A { // " |\r\n }', '"'],
    ['block-comment', 'class A { /* " | */ }', '"'],
    ['new-character', 'class A { char x = |; }', "'"],
    ['quoted-character', "class A { char x = '\\'|'; }", "'"],
    ['missing-close', 'class A { String s = "x |; }', '"'],
    ['after-word', 'class A { Object o = test|; }', '"'],
];
await captureQuoteInput('java', cases);
