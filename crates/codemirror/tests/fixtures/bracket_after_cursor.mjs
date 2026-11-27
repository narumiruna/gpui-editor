// Run with Node and the directory of SHA-512-pinned archives.
import assert from 'node:assert/strict';
import {mkdtempSync, realpathSync, rmSync} from 'node:fs';
import {join, resolve} from 'node:path';
import {tmpdir} from 'node:os';
import {projectedOffsets} from './editor_line_offsets.mjs';
import {pinnedModules} from './pinned_modules.mjs';

const cases = [
    ['text', '(a) [b]'],
    ['rust', 'fn f() { log("😀)"); }'],
    ['js', 'const s = `前${(1)} ${[0]}`;'],
    ['html', '<a>外</a>\r\n<script>log(")");</script>'],
];
const scratch = mkdtempSync(join(realpathSync(tmpdir()), 'gpui-before-brackets.'));
try {
    const {load} = pinnedModules(resolve(process.argv[2]), scratch);
    const {rust} = await load('@codemirror/lang-rust');
    const {javascript} = await load('@codemirror/lang-javascript');
    const {html} = await load('@codemirror/lang-html');
    const {bracketMatching, forceParsing} = await load('@codemirror/language');
    const {EditorState} = await load('@codemirror/state');
    const {ViewPlugin} = await load('@codemirror/view');
    const languages = {text: () => [], rust, js: javascript, html};
    for (const [language, source] of cases) {
        const {units, byteAt} = projectedOffsets(source);
        const checks = [];
        for (const afterCursor of [false, true]) {
            let rendered;
            const matching = bracketMatching({afterCursor, renderMatch(hit) {
                assert.equal(rendered, null);
                rendered = hit;
                return [];
            }});
            const plugin = matching.flat(Infinity).find(extension => extension instanceof ViewPlugin);
            assert(plugin);
            const view = {state: EditorState.create({doc: source, extensions: [languages[language](), matching]}), dispatch(spec) {
                this.state = this.state.update(spec).state;
            }};
            if (language !== 'text') assert(forceParsing(view, units, 5000));
            const range = ({from, to}) => [byteAt(from), byteAt(to)];
            const hits = [];
            let anchor = 0;
            for (const part of ['', ...source.match(/\r\n|[^]/gu)]) {
                anchor += part === '\r\n' ? 1 : part.length;
                rendered = null;
                plugin.create({state: view.state.update({selection: {anchor}}).state});
                hits.push([byteAt(anchor), rendered && [range(rendered.start), rendered.end ? range(rendered.end) : null, rendered.matched]]);
            }
            checks.push(hits);
        }
        console.log(JSON.stringify({language, source, beforeOnly: checks[0], both: checks[1]}));
    }
} finally {
    rmSync(scratch, {recursive: true, force: true});
}
