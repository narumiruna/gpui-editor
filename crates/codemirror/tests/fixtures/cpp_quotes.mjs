// Replay with Node and the directory of SHA-512-pinned archives.
import {captureQuoteInput} from './quote_input_fixture.mjs';

await captureQuoteInput('cpp', [
    ['inside a string', 'const char *s = "😀 | ";', '"'],
    ['inside a wide string', 'const auto s = L"字 | ";', '"'],
    ['inside a UTF-8 string', 'const auto s = u8"字 | ";', '"'],
    ['inside a raw string', 'const auto s = R"tag(字 | )tag";', '"'],
    ['inside a prefixed raw string', 'const auto s = u8R"tag(字 | )tag";', '"'],
    ['raw literal across CRLF', 'const auto s = R"tag(x\r\n😀| )tag";', '"'],
    ['before an unprefixed string opener', 'const auto s = |"字";', '"'],
    ['before an untracked closer', 'const auto s = "x|";', '"'],
    ['inside an escape context', 'const auto s = "\\"| ";', '"'],
    ['paired quote after a complete prefix', 'const auto s = u8R|;', '"'],
    ['wide character literal interior', "auto c = L'字 | ';", "'"],
    ['quote in comment', '// " |\r\nint x;', '"'],
], {}, 'cpp');

await captureQuoteInput('cpp', [
    ['C include angle is not a string opener', '#include |<stdio.h>', '"'],
    ['C wide string interior', 'const wchar_t *s = L"字 | ";', '"'],
    ['C unprefixed character opener', "char c = |'q';", "'"],
], {}, 'c');
