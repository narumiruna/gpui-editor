// Run with Node and a directory of SHA-512-pinned npm archives.
import assert from 'node:assert/strict';
import {mkdtempSync, realpathSync, rmSync} from 'node:fs';
import {tmpdir} from 'node:os';
import {join, resolve} from 'node:path';
import {pinnedModules} from './pinned_modules.mjs';
import {projectedOffsets} from './editor_line_offsets.mjs';

assert.equal(process.argv.length, 3, 'usage: node commands_blank_line.mjs <archive directory>');
const scratch = mkdtempSync(join(realpathSync(tmpdir()), 'gpui-blank-line.'));
try {
    const {load} = pinnedModules(resolve(process.argv[2]), scratch);
    const {EditorState, EditorSelection, Transaction} = await load('@codemirror/state');
    const {indentService} = await load('@codemirror/language');
    const {insertBlankLine} = await load('@codemirror/commands');
    for (const [name, corpus, direction, span] of [
        ['cross LF, forward', 'x\n ', false, [0, 2]],
        ['cross LF, reversed', 'x\n ', true, [0, 2]],
        ['select through the whitespace', 'x\n ', false, [0, 3]],
        ['single line, trim whitespace', 'x\n ', false, [2, 2]],
        ['leave next code line intact', 'x\na', false, [0, 2]],
        ['cross CRLF with Unicode', '猫😀\r\n  ', true, [0, 4]],
        ['trim same CRLF line', '猫😀\r\n  ', false, [4, 4]],
    ]) {
        const ending = corpus.includes('\r\n') ? '\r\n' : '\n';
        const {byteAt} = projectedOffsets(corpus);
        const queries = [];
        const selection = direction ? span.slice().reverse() : span;
        const state = EditorState.create({doc: corpus, selection: EditorSelection.single(...selection),
            extensions: [EditorState.lineSeparator.of(ending), indentService.of((_, pos) => {
                queries.push([byteAt(pos)]);
                return null;
            })]});
        let tr;
        const handled = insertBlankLine({state, dispatch(value) { tr = value; }});
        assert.ok(handled);
        const text = tr.state.sliceDoc();
        const before = [selection.map(byteAt)];
        const after = [[tr.newSelection.main.anchor, tr.newSelection.main.head].map(projectedOffsets(text).byteAt)];
        const edits = [];
        tr.changes.iterChanges((from, to, _newFrom, _newTo, insert) => edits.push([byteAt(from), byteAt(to), insert.sliceString(0, insert.length, ending)]));
        console.log(JSON.stringify({name, source: corpus, before, beforeMain: 0, mode: 'fallback', opts: {},
            command: 'blank', queries, handled, events: [[tr.annotation(Transaction.userEvent), tr.scrollIntoView, edits]],
            text, after, afterMain: 0}));
    }
} finally {
    rmSync(scratch, {recursive: true, force: true});
}
