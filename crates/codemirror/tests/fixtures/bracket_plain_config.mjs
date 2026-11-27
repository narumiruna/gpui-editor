// Run with Node and the directory of SHA-512-pinned archives.
import assert from 'node:assert/strict';
import {mkdtempSync, realpathSync, rmSync} from 'node:fs';
import {join, resolve} from 'node:path';
import {tmpdir} from 'node:os';
import {projectedOffsets} from './editor_line_offsets.mjs';
import {pinnedModules} from './pinned_modules.mjs';

const cases = [
    ['custom pairs', '<x> (p) «q»', {brackets: '<>«»'}],
    ['short limit at a line boundary', '(é\r\n)', {maxScanDistance: 1}],
    ['UTF-16 units and a logical break', '(中\r\n)', {maxScanDistance: 3}],
    ['lone CR is a logical break', '(中\r)', {maxScanDistance: 3}],
    ['limit checked between chunks', '(abcdefgh)', {maxScanDistance: 1}],
    ['custom pairs before the caret', '<x>', {brackets: '<>', afterCursor: false}],
    ['unclosed pair stops at the body edge', '<x', {brackets: '<>'}],
];
const scratch = mkdtempSync(join(realpathSync(tmpdir()), 'gpui-plain-brackets.'));
try {
    const {load} = pinnedModules(resolve(process.argv[2]), scratch);
    const {bracketMatching} = await load('@codemirror/language');
    const {EditorState} = await load('@codemirror/state');
    const {ViewPlugin} = await load('@codemirror/view');
    for (const [name, source, options] of cases) {
        const {byteAt} = projectedOffsets(source);
        let rendered;
        const matching = bracketMatching({...options, renderMatch(hit) {
            assert.equal(rendered, null);
            rendered = hit;
            return [];
        }});
        const plugin = matching.flat(Infinity).find(extension => extension instanceof ViewPlugin);
        assert(plugin);
        const state = EditorState.create({doc: source, extensions: matching});
        const range = ({from, to}) => [byteAt(from), byteAt(to)];
        let anchor = 0;
        const checks = [];
        for (const part of ['', ...source.match(/\r\n|[^]/gu)]) {
            anchor += part === '\r\n' ? 1 : part.length;
            rendered = null;
            plugin.create({state: state.update({selection: {anchor}}).state});
            checks.push([byteAt(anchor), rendered && [range(rendered.start), rendered.end ? range(rendered.end) : null, rendered.matched]]);
        }
        console.log(JSON.stringify({name, source, options, checks}));
    }
} finally {
    rmSync(scratch, {recursive: true, force: true});
}
