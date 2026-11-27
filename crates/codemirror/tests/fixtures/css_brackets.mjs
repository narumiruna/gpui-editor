// Replay with node and the directory of SHA-512-pinned archives.
import assert from 'node:assert/strict';
import {mkdtempSync, realpathSync, rmSync} from 'node:fs';
import {join, resolve} from 'node:path';
import {tmpdir} from 'node:os';
import {projectedOffsets} from './editor_line_offsets.mjs';
import {pinnedModules} from './pinned_modules.mjs';

const cases = [
    ['stylesheet', 'a { --v: f(")"); content: "😀}"; }'],
    ['stylesheet', 'a { color: rgb(/* ) */ 0, 0, 0); }'],
    ['stylesheet', 'a[data-kind="]"]:is(.x, .y) { width: 4px; }'],
    ['stylesheet', '@supports (color: rgb(0 0 0)) { a:has(:is(.x, .y)) { --a: [0]; } }'],
    ['stylesheet', 'a { background: url(https://example.test/[(]); }'],
    ['stylesheet', 'a { content: "[😀 )"; --other: "]"; } /* [ ] */'],
    ['stylesheet', '/* [ */\r\n/* 甲 ] */\r\na { content: "[x]"; }'],
    ['stylesheet', '/* 😀 ( */\r/* ] */\r\na { color: blue; }'],
    ['stylesheet', 'a { color: var( --tone'],
    ['stylesheet', 'a { color: rgb(1, 2, 3);'],
    ['styles', '--v: f(")"); color: rgb(0, 0, 0)'],
    ['styles', 'content: "[😀]"; --other: [1, 2];'],
    ['styles', '/* ( */\r\n--v: f( /* ) */ 1);'],
    ['styles', '--v: f( /* ) */ 1;'],
];
const scratch = mkdtempSync(join(realpathSync(tmpdir()), 'gpui-css-brackets.'));
try {
    const {load} = pinnedModules(resolve(process.argv[2]), scratch);
    const {css, cssLanguage} = await load('@codemirror/lang-css');
    const {bracketMatching, forceParsing} = await load('@codemirror/language');
    const {EditorState} = await load('@codemirror/state');
    const {ViewPlugin} = await load('@codemirror/view');
    let rendered;
    const matching = bracketMatching({renderMatch(hit) {
        assert.equal(rendered, null);
        rendered = hit;
        return [];
    }});
    const plugin = matching.flat(Infinity).find(extension => extension instanceof ViewPlugin);
    assert(plugin);
    const inline = cssLanguage.configure({top: 'Styles'});
    for (const [mode, source] of cases) {
        const {units, byteAt} = projectedOffsets(source);
        const language = mode === 'styles' ? inline : css();
        const view = {state: EditorState.create({doc: source, extensions: [language, matching]}), dispatch(spec) {
            this.state = this.state.update(spec).state;
        }};
        assert(forceParsing(view, units, 5000));
        const range = ({from, to}) => [byteAt(from), byteAt(to)];
        const checks = [];
        let anchor = 0;
        for (const part of ['', ...source.match(/\r\n|[^]/gu)]) {
            anchor += part === '\r\n' ? 1 : part.length;
            rendered = null;
            plugin.create({state: view.state.update({selection: {anchor}}).state});
            checks.push([byteAt(anchor), rendered && [range(rendered.start), rendered.end ? range(rendered.end) : null, rendered.matched]]);
        }
        console.log(JSON.stringify({mode, source, checks}));
    }
} finally {
    rmSync(scratch, {recursive: true, force: true});
}
