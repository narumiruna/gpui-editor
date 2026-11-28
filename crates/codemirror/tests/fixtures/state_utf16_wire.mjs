// node crates/codemirror/tests/fixtures/state_utf16_wire.mjs <archive-root>
import assert from 'node:assert/strict';
import {mkdtempSync, realpathSync, rmSync} from 'node:fs';
import {tmpdir} from 'node:os';
import {join, resolve} from 'node:path';
import {pinnedModules} from './pinned_modules.mjs';

const root = realpathSync(mkdtempSync(join(tmpdir(), 'state-utf16-wire-')));
try {
    const {load} = pinnedModules(resolve(process.argv[2]), root);
    const {EditorSelection, EditorState, StateField} = await load('@codemirror/state');
    const inspect = state => ({lineBreak: state.lineBreak, selection: state.selection.toJSON()});
    const seen = StateField.define({
        create: () => { throw new Error('expected field to be initialized from JSON'); },
        update: value => value,
        fromJSON: (value, state) => [value, inspect(state)],
        toJSON: ([value, context], state) => {
            assert.deepEqual(context, inspect(state));
            return value;
        },
    });
    for (const [name, input, separator, ranges, main, multiple] of [
        ['empty', '', null, [[0, 0]], 0, false],
        ['default collapse', 'a\r\n😀b\rcé\nZ', null, [[2, 4], [8, 6]], 1, false],
        ['default multiple', 'a\r\n😀b\rcé\nZ', null, [[2, 4], [8, 6]], 0, true],
        ['configured CRLF', 'é\r\n🍵尾\r\nz', '\r\n', [[1, 0], [6, 7]], 1, true],
        ['configured long separator', 'é<:>🍵尾<:>z', '<:>', [[1, 0], [6, 7]], 1, true],
        ['literal CR before configured break', 'a\r||😀||z', '||', [[2, 1], [6, 3]], 1, true],
        ['lone CR stays content with explicit LF', 'a\ré\n🦀Z', '\n', [[0, 2], [7, 6]], 0, true],
    ]) {
        const selection = EditorSelection.create(ranges.map(([anchor, head]) =>
            EditorSelection.range(anchor, head)), main);
        const value = {doc: input, selection: selection.toJSON(), seen: name};
        const overrides = separator == null ? [] : EditorState.lineSeparator.of(separator);
        const extensions = [overrides, multiple ? EditorState.allowMultipleSelections.of(true) : []];
        const state = EditorState.fromJSON(value, {extensions}, {seen});
        const output = state.toJSON({seen});
        const lines = [...state.doc.iterLines()];
        const offsets = new Map([[0, 0]]);
        // Preserve a literal trailing CR separately from the native LF break.
        let units = 0, bytes = 0;
        let nativeText = '';
        for (let i = 0; i < lines.length; i++) {
            if (i > 0) {
                const lf = lines[i - 1].endsWith('\r') ? '\r\n' : '\n';
                nativeText += lf;
                bytes += lf.length;
                offsets.set(++units, bytes);
            }
            nativeText += lines[i];
            for (const ch of lines[i]) {
                units += ch.length;
                bytes += Buffer.byteLength(ch);
                offsets.set(units, bytes);
            }
        }
        const {selection: {ranges: selected, main: primary}} = output;
        const nativeSelection = {ranges: selected.map(({anchor, head}) => {
            assert(offsets.has(anchor) && offsets.has(head), name);
            return {anchor: offsets.get(anchor), head: offsets.get(head)};
        }), main: primary};
        console.log(JSON.stringify({name, value, separator, multiple, nativeText, nativeSelection, output}));
    }
    const doc = 'a\nb||c';
    const internalLf = EditorState.fromJSON({doc, selection: EditorSelection.single(2).toJSON()},
        {extensions: EditorState.lineSeparator.of('||')});
    assert.equal(internalLf.doc.lines, 2);
    assert.equal(internalLf.doc.line(1).text, 'a\nb');
    assert.equal(internalLf.toJSON().doc, doc);
} finally {
    rmSync(root, {recursive: true, force: true});
}
