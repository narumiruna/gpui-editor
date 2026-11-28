// Run with Node and the directory of SHA-512-pinned npm archives.
import assert from 'node:assert/strict';
import {mkdtempSync, realpathSync, rmSync} from 'node:fs';
import {tmpdir} from 'node:os';
import {join, resolve} from 'node:path';
import {projectedOffsets} from './editor_line_offsets.mjs';
import {pinnedModules} from './pinned_modules.mjs';

assert.equal(process.argv.length, 3, 'usage: node lint_editor_lines.mjs <archive directory>');
const root = mkdtempSync(join(realpathSync(tmpdir()), 'lint-editor-lines.'));
try {
    const {load} = pinnedModules(resolve(process.argv[2]), root);
    const {EditorState} = await load('@codemirror/state');
    const {forEachDiagnostic, setDiagnostics} = await load('@codemirror/lint');
    const cases = [
        ['lone-cr', 'A\rZ', 1, 2],
        ['lf', 'A\nZ', 1, 2],
        ['crlf', '🦀\r\nZ', 2, 3],
        ['mixed', '🦀\r\r\n\nZ', 2, 5],
        ['ends-with-content', 'A\rZ', 1, 3],
        ['only-cr', '\r', 0, 1],
        ['at-eof', 'A\r', 1, 2],
        ['nine-cr', 'A' + '\r'.repeat(9) + 'Z', 1, 10],
        ['ten-cr', 'A' + '\r'.repeat(10) + 'Z', 1, 11],
    ];
    for (const [id, source, from, to] of cases) {
        const {byteAt} = projectedOffsets(source);
        let state = EditorState.create({doc: source});
        state = state.update(setDiagnostics(state, [{from, to, severity: 'warning', message: id}])).state;
        const markers = [];
        forEachDiagnostic(state, (_d, from, to) => markers.push([byteAt(from), byteAt(to)]));
        console.log(JSON.stringify({id, source, range: [byteAt(from), byteAt(to)], markers}));
    }
} finally {
    rmSync(root, {recursive: true, force: true});
}
