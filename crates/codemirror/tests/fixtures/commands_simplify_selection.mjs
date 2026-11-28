// Run with Node and a directory of SHA-512-pinned npm archives.
import assert from 'node:assert/strict';
import {mkdtempSync, realpathSync, rmSync} from 'node:fs';
import {tmpdir} from 'node:os';
import {join, resolve} from 'node:path';
import {pinnedModules} from './pinned_modules.mjs';
import {projectedOffsets} from './editor_line_offsets.mjs';

assert.equal(process.argv.length, 3, 'usage: node commands_simplify_selection.mjs <archive directory>');
const scratch = mkdtempSync(join(realpathSync(tmpdir()), 'gpui-simplify-selection.'));
try {
    const {load} = pinnedModules(resolve(process.argv[2]), scratch);
    const {EditorState, EditorSelection, Transaction} = await load('@codemirror/state');
    const {simplifySelection} = await load('@codemirror/commands');
    const doc = 'é 😀 abcdef';
    const {byteAt} = projectedOffsets(doc);
    const snapshot = ({ranges, mainIndex}) => ({main: mainIndex, ranges: ranges.map(r => [byteAt(r.anchor), byteAt(r.head), r.assoc,
        r.bidiLevel ?? null, r.goalColumn ?? null, r.undirectional])});
    for (const [name, ranges, main] of [
        ['keep cursor metadata', [EditorSelection.cursor(0), EditorSelection.cursor(6, -1, 3, 20)], 1],
        ['keep reversed range metadata', [EditorSelection.cursor(0), EditorSelection.range(8, 6, 9, 2, -1)], 1],
        ['keep undirectional range', [EditorSelection.undirectionalRange(1, 4), EditorSelection.cursor(6)], 0],
        ['collapse single range', [EditorSelection.range(8, 6, 9, 2, -1)], 0],
        ['decline single cursor', [EditorSelection.cursor(6, 1, 3, 20)], 0],
    ]) {
        const state = EditorState.create({doc, selection: EditorSelection.create(ranges, main),
            extensions: EditorState.allowMultipleSelections.of(true)});
        let tr;
        const handled = simplifySelection({state, dispatch(value) { tr = value; }});
        console.log(JSON.stringify({name, doc, before: snapshot(state.selection), handled,
            after: snapshot(tr ? tr.newSelection : state.selection), event: tr?.annotation(Transaction.userEvent), scroll: tr?.scrollIntoView}));
    }
} finally {
    rmSync(scratch, {recursive: true, force: true});
}
