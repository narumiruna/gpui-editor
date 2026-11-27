// Replay with node and the directory of SHA-512-pinned archives.
import {captureQuoteInput} from './quote_input_fixture.mjs';

await captureQuoteInput('javascript', [
    ['inside a quoted literal', 'const x = "😀 | ";', '"'],
    ['after a quote escape', 'const x = "\\"| ";', '"'],
    ['a different quote inside a literal', 'const x = "字 | ";', "'"],
    ['before the parsed opening quote', 'const x = |"字";', '"'],
    ['before an untracked closing quote', 'const x = "字|";', '"'],
    ['immediately after the string', 'const x = "x"|;', '"'],
    ['after a space outside the string', 'const x = "x" |;', '"'],
    ['inside a template', 'const x = `字 | `;', '`'],
    ['before the template opener', 'const x = |`字`;', '`'],
    ['inside a template substitution', 'const x = `中 ${ | }`;', '`'],
    ['inside a nested string', "const x = `x ${call('字 | ')}`;", "'"],
    ['inside a tagged multiline template', 'tag`字\r\n😀| `;', '`'],
    ['after an escaped line break', 'const x = "\\\r\n字 | ";', '"'],
    ['after a quote in a comment', '// " |\r\nlet x;', '"'],
    ['after slash in a regular expression', 'const x = /" | /;', '"'],
    ['unfinished quote', 'const x = "字 |;', '"'],
], {}, 'js');

await captureQuoteInput('javascript', [
    ['type literal content', 'type A = "字 | ";', '"'],
    ['first quote after the type', 'type A = "字" |;', '"'],
    ['template literal type', 'type A = `get${Capitalize<"字 | ">}`;', '"'],
], {typescript: true}, 'ts');

await captureQuoteInput('javascript', [
    ['attribute quote content', 'const el = <p title="字 | "/>;', '"'],
    ['before the attribute opener', 'const el = <p title=|"字"/>;', '"'],
    ['after an untracked attribute closer', 'const el = <p title="字"| />;', '"'],
    ['quote inside JSX text', 'const el = <p>甲 | 乙</p>;', '"'],
    ['literal inside an attribute expression', 'const el = <p title={"甲 | 乙"}/>;', '"'],
], {jsx: true}, 'jsx');

await captureQuoteInput('javascript', [
    ['typed arrow function return literal', 'const f = <T,>(x: T) => <p>{`字 | `}</p>;', '`'],
    ['before a nested attribute opener', 'const x: JSX.Element = <p title=|"字">😀</p>;', '"'],
], {typescript: true, jsx: true}, 'tsx');

await captureQuoteInput('html', [
    ['script string interior', '<script>const x = "😀 | ";</script>', '"'],
    ['script string opener', '<script>const x = |"字";</script>', '"'],
    ['script template with CRLF', '<script>const x = `字\r\n😀| `;</script>', '`'],
    ['only selected script receives input', '<script>call("abc ");</script><script>call("中 | ");</script>', '"'],
    ['event attribute string interior', `<button onclick="call('😀 | ')">OK</button>`, "'"],
    ['event attribute string opener', `<button onclick="call(|'字')">OK</button>`, "'"],
    ['ordinary attribute before string opener', '<p title=|"字">ok</p>', '"'],
    ['ordinary attribute string interior', '<p title="😀 | ">ok</p>', '"'],
    ['ordinary attribute untracked closer', '<p title="😀 |">ok</p>', '"'],
    ['ordinary attribute after a closed value', '<p title="字" | >ok</p>', '"'],
], {}, 'html');
