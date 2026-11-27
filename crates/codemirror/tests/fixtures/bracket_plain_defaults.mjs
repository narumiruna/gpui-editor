// Replay with Node and the directory of SHA-512-pinned archives.
import assert from 'node:assert/strict';
import {mkdtempSync, realpathSync, rmSync} from 'node:fs';
import {tmpdir} from 'node:os';
import {join, resolve} from 'node:path';
import {projectedOffsets} from './editor_line_offsets.mjs';
import {pinnedModules} from './pinned_modules.mjs';

const cases = [
    ['multibyte BMP fits the stock cap', 'é', 5000, '\r\n'],
    ['supplementary text reaches the cap exactly', '😀', 4999, '\r\n'],
    ['lone CR reaches a logical boundary past the cap', '😀', 5000, '\r'],
    ['a same-line match can overshoot the cap', '😀', 6000, ''],
];
const scratch = mkdtempSync(join(realpathSync(tmpdir()), 'gpui-plain-defaults.'));
try {
    const {load} = pinnedModules(resolve(process.argv[2]), scratch);
    const {bracketMatching} = await load('@codemirror/language');
    const {EditorState} = await load('@codemirror/state');
    const {ViewPlugin} = await load('@codemirror/view');
    const matching = bracketMatching();
    const plugin = matching.flat(Infinity).find(extension => extension instanceof ViewPlugin);
    assert(plugin);
    for (const [name, fill, repeat, separator] of cases) {
        const source = `(${fill.repeat(repeat)}${separator})`;
        const {units, byteAt} = projectedOffsets(source);
        const state = EditorState.create({doc: source, extensions: matching});
        const checks = [0, 1, units - 1, units].map(anchor => {
            const instance = plugin.create({state: state.update({selection: {anchor}}).state});
            const marks = [];
            for (let iter = instance.decorations.iter(); iter.value; iter.next()) {
                assert.equal(iter.value.spec.class, 'cm-matchingBracket');
                marks.push([byteAt(iter.from), byteAt(iter.to)]);
            }
            return [byteAt(anchor), marks];
        });
        console.log(JSON.stringify({name, fill, repeat, separator, checks}));
    }
} finally {
    rmSync(scratch, {recursive: true, force: true});
}
