// Replay with node and the directory of SHA-512-pinned archives.
import {captureQuoteInput} from './quote_input_fixture.mjs';

const cases = [
    ['inside interpreted string', 'package p\nvar s = "😀| "', '"'],
    ['inside raw string', 'package p\nvar s = `x | `', '`'],
    ['other quote inside raw string', 'package p\nvar s = `x | `', '"'],
    ['other quote inside interpreted string', 'package p\nvar s = "x | "', '`'],
    ['inside rune', "package p\nvar c = '| '", "'"],
    ['after escape', 'package p\nvar s = "\\"| "', '"'],
    ['multiline raw string and CRLF', 'package p\r\nvar s = `甲\r\n😀| `', '`'],
    ['before parsed string opener', 'package p\nvar s = |"字"', '"'],
    ['before parsed raw opener', 'package p\nvar s = |`字`', '`'],
    ['before untracked closer', 'package p\nvar s = "x|"', '"'],
    ['before untracked raw closer', 'package p\nvar s = `x|`', '`'],
    ['after a complete literal', 'package p\nvar s = "x"|;', '"'],
    ['after space outside literal', 'package p\nvar s = "x" |;', '"'],
    ['comment with a quote', 'package p\r\n// " |', '"'],
    ['unfinished string', 'package p\nvar s = "abc |;', '"'],
    ['unfinished raw string', 'package p\nvar s = `abc |;', '`'],
];
await captureQuoteInput('go', cases);
