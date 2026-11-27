// From the repository root, run the WAST package oracle to verify the archives:
// node crates/codemirror/tests/fixtures/wast_oracle.mjs /private/tmp/gpui-upstream-exports.UVI6VVWZ
// Then regenerate this fixture:
// node crates/app/src/editor/source_brackets/wast_brackets.mjs /private/tmp/gpui-upstream-exports.UVI6VVWZ \
//   > crates/app/src/editor/source_brackets/wast_brackets.jsonl
import assert from 'node:assert/strict';
import {readFileSync} from 'node:fs';
import {join, resolve} from 'node:path';
import {pathToFileURL} from 'node:url';

const root = resolve(process.argv[2]);
const manifest = JSON.parse(readFileSync(new URL('../../../../../docs/codemirror-upstream.json', import.meta.url)));
const pins = [...manifest.packages, ...manifest.referenceDependencies];
const dir = name => join(root, 'node_modules', name);
for (const name of ['@codemirror/lang-wast', '@codemirror/language', '@codemirror/state',
                    '@lezer/common', '@lezer/lr', '@lezer/highlight']) {
    const pkg = JSON.parse(readFileSync(join(dir(name), 'package.json')));
    assert.equal(pkg.name, name);
    assert.equal(pkg.version, pins.find(pin => pin.name === name).version);
}
const load = name => import(pathToFileURL(join(dir(name), 'dist/index.js')));
const {wast} = await load('@codemirror/lang-wast');
const {EditorState} = await load('@codemirror/state');
const {forceParsing, matchBrackets} = await load('@codemirror/language');

for (const [name, template, repeat] of [
    ['quoted closer', '(module (data (i32.const 0) ")"))'],
    ['comments, non-ASCII text, and CRLF', ';; 😀 )\r\n(module\r\n  (; ) [x] ;)\r\n  (data "字 😀 ([])" )\r\n  ;; ) [)\r\n)'],
    ['plain matching by syntax type', '(data "["\r\n  ;; ]\r\n  (; ] ;)\r\n  "]")'],
    ['different bracket types in a string', '(data "{[x])")'],
    ['unfinished application', '(module\r\n  (data "字 )")\r\n  ;; )'],
    ['stray close and adjacent pairs', ')(func)()(data $[foo} "[")'],
    ['incomplete string', '(data "[ ( )'],
    ['CRLF under the UTF-16 limit', '(data "[$]")', ['\r\n  ', 3200]],
    ['CRLF over the UTF-16 limit', '(data "[$]")', ['\r\n  ', 4000]],
    ['supplementary characters under the limit', '(data "[$]")', ['\n😀  ', 1900]],
    ['supplementary characters over the limit', '(data "[$]")', ['\n😀  ', 2100]],
    ['lone CR under the UTF-16 limit', '(data "[$]")', ['\r😀  ', 1900]],
]) {
    const source = repeat ? template.replace('$', repeat[0].repeat(repeat[1])) : template;
    const view = {state: EditorState.create({doc: source, extensions: wast()}),
                  dispatch(spec) { this.state = this.state.update(spec).state; }};
    assert.ok(forceParsing(view, view.state.doc.length, 5000));
    const {state} = view;
    const offsets = [0];
    let bytes = 0;
    for (const ch of source.match(/\r\n|[^]/gu)) {
        if (ch.length === 2 && ch !== '\r\n') offsets.push(null);
        offsets.push(bytes += Buffer.byteLength(ch));
    }
    const text = state.doc.toString();
    assert.equal(offsets.length, text.length + 1);
    const checks = [];
    for (let head = 0; head <= text.length; head++) {
        if (!/[()[\]{}]/.test(text.slice(Math.max(0, head - 1), head + 1))) continue;
        // Use the order in bracketDeco, including the brackets after the caret.
        const hit = matchBrackets(state, head, -1)
            || (head > 0 && matchBrackets(state, head - 1, 1))
            || matchBrackets(state, head, 1)
            || (head < text.length && matchBrackets(state, head + 1, -1));
        const range = ({from, to}) => [offsets[from], offsets[to]];
        checks.push([offsets[head], hit ? [range(hit.start), hit.end ? range(hit.end) : null, hit.matched] : null]);
    }
    if (name === 'quoted closer') assert.deepEqual(checks[0], [0, [[0, 1], [32, 33], true]]);
    console.log(JSON.stringify({name, source: template, repeat, checks}));
}
