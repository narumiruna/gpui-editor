// Replay with node and the directory of SHA-512-pinned archives.
import assert from 'node:assert/strict';
import {mkdtempSync, realpathSync, rmSync} from 'node:fs';
import {join, resolve} from 'node:path';
import {tmpdir} from 'node:os';
import {projectedOffsets} from './editor_line_offsets.mjs';
import {pinnedModules} from './pinned_modules.mjs';

const cases = [
    ['script and host boundaries', '<script>|log(|"|)"|)|</script>'],
    ['comments and Unicode', '<script>// 甲 😀 ]\r\nlog(|/* ) */ 1|);\r\n</script>'],
    ['independent scripts', '<script>f(|</script>\r\n<script>|)</script>'],
    ['TypeScript script', '<script type="text/typescript">let x: number = a[|0]; log(|"|)");</script>'],
    ['event expression', `<p onclick='log(|"|)")|' title='(not code)'>字</p>`],
    ['JSON script expression', '<script type="application/json">{|"a": "|}", "b": [|1]}</script>'],
    ['style and host boundaries', '<style>|a { --v: f(|"|)"|); }</style>'],
    ['inline style', `<p style='--v: f(|"|)")|;' title='(untouched)'>字</p>`],
];
const scratch = mkdtempSync(join(realpathSync(tmpdir()), 'gpui-html-mounted-brackets.'));
try {
    const {load} = pinnedModules(resolve(process.argv[2]), scratch);
    const {html} = await load('@codemirror/lang-html');
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
    for (const [name, marked] of cases) {
        const parts = marked.split('|');
        const source = parts.join('');
        const {units, byteAt} = projectedOffsets(source);
        const view = {state: EditorState.create({doc: source, extensions: [html(), matching]}), dispatch(spec) {
            this.state = this.state.update(spec).state;
        }};
        assert(forceParsing(view, units, 5000));
        const range = ({from, to}) => [byteAt(from), byteAt(to)];
        const checks = [];
        let anchor = 0;
        for (const part of parts.slice(0, -1)) {
            anchor += part.replace(/\r\n?/g, '\n').length;
            rendered = null;
            plugin.create({state: view.state.update({selection: {anchor}}).state});
            checks.push([byteAt(anchor), rendered && [range(rendered.start), rendered.end ? range(rendered.end) : null, rendered.matched]]);
        }
        console.log(JSON.stringify({name, source, checks}));
    }
} finally {
    rmSync(scratch, {recursive: true, force: true});
}
