// node crates/codemirror/tests/fixtures/setup_projected_keys.mjs <archive-root>
import assert from 'node:assert/strict';
import {mkdtempSync, realpathSync, rmSync} from 'node:fs';
import {tmpdir} from 'node:os';
import {join, resolve} from 'node:path';
import {projectedOffsets} from './editor_line_offsets.mjs';
import {pinnedModules} from './pinned_modules.mjs';

const root = realpathSync(mkdtempSync(join(tmpdir(), 'setup-projected-keys-')));
try {
    const {load} = pinnedModules(resolve(process.argv[2]), root);
    const commands = await load('@codemirror/commands');
    const {basicSetup} = await load('codemirror');
    const {EditorSelection, EditorState, Transaction} = await load('@codemirror/state');
    const mixed = '甲\r😀§x\r\né\n終';
    const wrap = '§abcde\rabc\nz';
    const rows = [0, 3, 6, 10];
    const normalize = text => text.replace(/\r\n?/g, '\n');
    const cases = [
        ['cursorLineBoundaryBackward', 'Home', mixed],
        ['cursorLineBoundaryForward', 'End', mixed],
        ['selectLine', 'Alt-l', mixed],
        ['cursorLineDown', 'ArrowDown', mixed],
        ['deleteLine', 'Ctrl-Shift-k', mixed, '甲\r\né\n終'],
        ['indentLess', 'Ctrl-[', 'p\r  §猫\r\nq', 'p\r猫\r\nq'],
        ['indentMore', 'Ctrl-]', 'p\r§猫\r\nq', 'p\r  猫\r\nq'],
        ['deleteCharBackward', 'Backspace', 'p\r    §猫\r\nq', 'p\r  猫\r\nq'],
        ['indentLess', 'Ctrl-[', 'p\r  §\nq', null],
        ['cursorLineDown', 'ArrowDown', wrap, undefined, rows],
        ['addCursorBelow', 'Ctrl-Alt-ArrowDown', wrap, undefined, rows],
    ];
    for (const [command, key, marked, expected, measured] of cases) {
        const at = marked.indexOf('§');
        const source = marked.replace('§', '');
        const position = normalize(source.slice(0, at)).length;
        const state = EditorState.create({doc: source, selection: EditorSelection.single(position), extensions: basicSetup});
        const view = {
            state,
            lineWrapping: false,
            lineBlockAt(pos) { return this.state.doc.lineAt(pos); },
            moveToLineBoundary(cursor, forward) {
                const line = this.lineBlockAt(cursor.head);
                return EditorSelection.cursor(forward ? line.to : line.from);
            },
            moveVertically(cursor, forward) {
                if (measured) {
                    const index = measured.indexOf(cursor.head) + (forward ? 1 : -1);
                    return EditorSelection.cursor(measured[index] ?? cursor.head);
                }
                const line = this.state.doc.lineAt(cursor.head);
                const index = line.number + (forward ? 1 : -1);
                if (index < 1 || index > this.state.doc.lines) return cursor;
                const next = this.state.doc.line(index);
                return EditorSelection.cursor(Math.min(next.to, next.from + cursor.head - line.from));
            },
            dispatch(...spec) {
                const transaction = spec[0] instanceof Transaction ? spec[0] : this.state.update(...spec);
                this.state = transaction.state;
            },
        };
        view.dispatch = view.dispatch.bind(view);
        assert.ok(commands[command](view), command);
        const output = expected === undefined ? source : expected;
        if (output !== null) assert.equal(view.state.doc.toString(), normalize(output), command);
        const select = output === null ? null : view.state.selection.ranges.map(range =>
            [range.anchor, range.head].map(projectedOffsets(output).byteAt));
        console.log(JSON.stringify({command, key, source, at: projectedOffsets(source).byteAt(position),
            output, select, rows: measured ?? null, main: view.state.selection.mainIndex}));
    }
} finally {
    rmSync(root, {recursive: true, force: true});
}
