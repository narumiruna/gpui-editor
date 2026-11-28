// Run with Node and the directory of SHA-512-pinned npm archives.
import assert from 'node:assert/strict';
import {mkdtempSync, realpathSync, rmSync} from 'node:fs';
import {tmpdir} from 'node:os';
import {join, resolve} from 'node:path';
import {projectedOffsets} from './editor_line_offsets.mjs';
import {pinnedModules} from './pinned_modules.mjs';

assert.equal(process.argv.length, 3, 'usage: node lint_gutter_mapping.mjs <archive directory>');
const root = mkdtempSync(join(realpathSync(tmpdir()), 'lint-gutter-mapping.'));
try {
    const {load} = pinnedModules(resolve(process.argv[2]), root);
    const {Compartment, EditorState} = await load('@codemirror/state');
    const {lintGutter, linter, setDiagnostics} = await load('@codemirror/lint');
    const source = '😀\r\na\r\nb';
    const baseline = [[6, 7, 'warning', 'a'], [6, 7, 'error', 'a again'], [9, 10, 'info', 'b']];
    const steps = [
        ['publish', baseline], ['configure', 'all'], ['publish', baseline],
        ['configure', 'without errors'], ['publish', baseline],
        ['edit', [0, 0, 'é']], ['edit', [8, 8, '\r\n']],
        ['edit', [13, 14, '']], ['edit', [6, 8, '']],
        ['configure', 'off'], ['configure', 'all'],
        ['publish', [[6, 6, 'hint', 'point']]], ['publish', []],
    ];
    const slot = new Compartment();
    const init = linter(null, {markerFilter: () => []});
    let state = EditorState.create({doc: source, extensions: [init, slot.of([])]});
    let raw = source, field = null;
    console.log(JSON.stringify({source}));
    for (const [op, value] of steps) {
        const unit = byte => projectedOffsets(Buffer.from(raw).subarray(0, byte).toString()).units;
        if (op === 'configure') {
            const extension = value === 'off' ? [] : lintGutter(value === 'without errors'
                ? {markerFilter: ds => ds.filter(d => d.severity !== 'error')} : {});
            field = extension[1];
            state = state.update({effects: slot.reconfigure(extension)}).state;
        } else if (op === 'publish') {
            state = state.update(setDiagnostics(state, value.map(([from, to, severity, message]) => ({
                from: unit(from), to: unit(to), severity, message,
            })))).state;
        } else {
            const [from, to, insert] = value;
            state = state.update({changes: {from: unit(from), to: unit(to), insert}}).state;
            const bytes = Buffer.from(raw);
            raw = Buffer.concat([bytes.subarray(0, from), Buffer.from(insert), bytes.subarray(to)]).toString();
        }
        assert.equal(state.doc.toString(), raw.replace(/\r\n?/g, '\n'));
        const {byteAt} = projectedOffsets(raw);
        const markers = field ? [] : null;
        if (field) {
            for (const it = state.field(field).iter(); it.value; it.next()) {
                markers.push([byteAt(it.from), it.value.severity, it.value.diagnostics.map(d => d.message)]);
            }
        }
        console.log(JSON.stringify({op, value, markers, raw}));
    }
} finally {
    rmSync(root, {recursive: true, force: true});
}
