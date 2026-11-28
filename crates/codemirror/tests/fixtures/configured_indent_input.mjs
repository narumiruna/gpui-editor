// node crates/codemirror/tests/fixtures/configured_indent_input.mjs <archive-root> > crates/codemirror/tests/fixtures/configured_indent_input.jsonl
import assert from 'node:assert/strict';
import {mkdtempSync, realpathSync, rmSync} from 'node:fs';
import {tmpdir} from 'node:os';
import {join, resolve} from 'node:path';
import {projectInput, projectedOffsets} from './editor_line_offsets.mjs';
import {pinnedModules} from './pinned_modules.mjs';

const root = realpathSync(mkdtempSync(join(tmpdir(), 'configured-indent-input-')));
try {
    const {load} = pinnedModules(resolve(process.argv[2]), root);
    const {Compartment, EditorState, Prec, StateField, Transaction} = await load('@codemirror/state');
    const {history, undo, undoDepth} = await load('@codemirror/commands');
    const {indentNodeProp, indentOnInput, indentService, indentUnit} = await load('@codemirror/language');
    const {jsonLanguage} = await load('@codemirror/lang-json');
    const {lezerLanguage} = await load('@codemirror/lang-lezer');
    const edits = StateField.define({create: () => 0, update: (n, tr) => n + Number(tr.docChanged)});
    for (const [language, row, source] of [
        [jsonLanguage, 'Object', '{\r\n  "猫😀": 1\r       \r\n'],
        [lezerLanguage, 'Body', '@top Main {\r\n  "猫😀"\r       \r\n'],
    ]) {
        let raw = source, candidate = '', seen = [];
        const observe = (name, cx, pos) => {
            assert.equal(cx.state.doc.toString(), candidate.replace(/\r\n?/g, '\n'));
            const {byteAt} = projectedOffsets(candidate);
            seen.push([name, byteAt(pos), byteAt(cx.state.selection.main.head), cx.unit,
                cx.state.field(edits), candidate]);
        };
        const slot = new Compartment();
        const config = (mode, unit) => {
            const parser = mode.startsWith('stock-') ? language : language.configure({props: [indentNodeProp.add(type => {
                if (type.name === row) return cx => {
                    observe(row, cx, cx.pos);
                    return mode === 'stop' ? null : cx.continue() + cx.state.field(edits) + cx.unit;
                };
                if (type.isTop) return cx => { observe('root', cx, cx.pos); return 1; };
            })]});
            return [parser, indentUnit.of(unit),
                indentService.of((cx, pos) => {
                    observe('middle', cx, pos);
                    return mode === 'stock-blocked' ? null : mode === 'stock-service' ? 5 : undefined;
                }),
                Prec.high(indentService.of((cx, pos) => { observe('high', cx, pos); })),
                Prec.low(indentService.of((cx, pos) => { observe('low', cx, pos); }))];
        };
        const caret = source.slice(0, -2).replace(/\r\n?/g, '\n').length;
        let state = EditorState.create({doc: source, selection: {anchor: caret},
            extensions: [indentOnInput(), history(), edits, slot.of(config('continue', '\t'))]});
        const steps = [];
        const record = (step, tr) => {
            raw = projectInput(raw, tr);
            state = tr.state;
            const byteAt = projectedOffsets(raw).byteAt;
            steps.push({...step, result: raw, head: byteAt(state.selection.main.head),
                undoDepth: undoDepth(state), seen});
            seen = [];
        };
        let time = 0;
        const type = (insert, mode, unit, event = 'input.type') => {
            const head = projectedOffsets(raw).byteAt(state.selection.main.head);
            const bytes = Buffer.from(raw);
            candidate = bytes.subarray(0, head).toString() + insert + bytes.subarray(head).toString();
            record({insert, mode, unit, event}, state.update({...state.replaceSelection(insert),
                effects: mode ? slot.reconfigure(config(mode, unit)) : [],
                annotations: [Transaction.userEvent.of(event), Transaction.time.of(time += 1000)]}));
        };
        const undoOnce = () => assert.ok(undo({state, dispatch: tr => record({undo: true}, tr)}));
        type('}');
        undoOnce();
        type('x');
        undoOnce();
        const from = state.doc.toString().indexOf('猫');
        const byteAt = projectedOffsets(raw).byteAt;
        record({edit: [byteAt(from), byteAt(from + 1), '🌱a']}, state.update({
            changes: {from, to: from + 1, insert: '🌱a'}, annotations: Transaction.time.of(time += 1000),
        }));
        type('}', 'stop', '   ', 'input.complete');
        undoOnce();
        const reconfigure = (mode, unit) => record({mode, unit}, state.update({effects: slot.reconfigure(config(mode, unit))}));
        reconfigure('stock-blocked', '   ');
        type('}');
        undoOnce();
        reconfigure('stock-service', '   ');
        type('}');
        undoOnce();
        type('}', 'continue', '\t');
        undoOnce();
        console.log(JSON.stringify({language: language.name, source, caret: projectedOffsets(source).byteAt(caret), steps}));
    }
} finally {
    rmSync(root, {recursive: true, force: true});
}
