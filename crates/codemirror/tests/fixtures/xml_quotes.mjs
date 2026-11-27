// Replay with node and the directory of SHA-512-pinned archives.
import {captureQuoteInput} from './quote_input_fixture.mjs';

await captureQuoteInput('xml', [
    ['inside double-quoted attribute', '<root name="😀 | "/>', '"'],
    ['inside single-quoted attribute', "<root name='字 | '/>", "'"],
    ['before parsed attribute opener', '<root name=|"字"/>', '"'],
    ['before single-quoted attribute opener', "<root name=|'字'/>", "'"],
    ['before untracked attribute closer', '<root name="x|"/>', '"'],
    ['after closed attribute with a space', '<root name="字" | />', '"'],
    ['character and entity references', '<root name="a&#34; &amp; | "/>', '"'],
    ['multiline attribute with CRLF', '<root name="a\r\n😀| "/>', '"'],
    ['root text is not an attribute value', '<root>"字 | </root>', '"'],
    ['comment has no string context', '<root><!-- "字 | --> </root>', '"'],
    ['processing instruction does not start a string', '<?xml version="1.0" | ?>\r\n<root/>', '"'],
    ['unfinished attribute value', '<root name="字 | />', '"'],
]);
