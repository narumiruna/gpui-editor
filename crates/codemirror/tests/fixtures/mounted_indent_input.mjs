// node crates/codemirror/tests/fixtures/mounted_indent_input.mjs <archive-root>
import assert from 'node:assert/strict';
import {mkdtempSync, realpathSync, rmSync} from 'node:fs';
import {tmpdir} from 'node:os';
import {join, resolve} from 'node:path';
import {projectInput, projectedOffsets} from './editor_line_offsets.mjs';
import {pinnedModules} from './pinned_modules.mjs';

const root = realpathSync(mkdtempSync(join(tmpdir(), 'mounted-indent-input-')));
try {
    const {load} = pinnedModules(resolve(process.argv[2]), root);
    const {EditorState, StateField, Transaction} = await load('@codemirror/state');
    const {history, undo, undoDepth} = await load('@codemirror/commands');
    const {indentOnInput, indentNodeProp, indentUnit, indentService} = await load('@codemirror/language');
    const {markdown} = await load('@codemirror/lang-markdown');
    const {jsonLanguage} = await load('@codemirror/lang-json');
    const {lezerLanguage} = await load('@codemirror/lang-lezer');
    const {wastLanguage} = await load('@codemirror/lang-wast');
    for (const [name, language, label, node, body, insert] of [
        ['json', jsonLanguage, 'code', 'Object', '{\r\n  "猫😀": 1\r       ', '}'],
        ['lezer', lezerLanguage, '', 'Body', '@top Main {\r\n  "猫😀"\r       ', '}'],
        ['wast', wastLanguage, 'code', 'App', '(module\r\n  (data "猫😀")\r       ', ')'],
    ]) {
        const prefix = `Intro 😀\r\n\r\n\`\`\`${label}\r\n`;
        const suffix = '\r\n```\r\n\r\nTail.';
        const source = prefix + body + suffix;
        const candidate = prefix + body + insert + suffix;
        const caret = projectedOffsets(prefix + body).units;
        const {byteAt} = projectedOffsets(candidate);
        const edits = StateField.define({create: () => 0, update: (count, tr) => count + tr.docChanged});
        const calls = [];
        const record = (id, cx, pos, range, next) => {
            assert.equal(cx.state.doc.toString(), candidate.replace(/\r\n?/g, '\n'));
            calls.push([id, byteAt(pos), byteAt(cx.state.selection.main.head), cx.unit, cx.state.field(edits), range, next]);
        };
        const configured = language.configure({props: [indentNodeProp.add({[node]: cx => {
            const next = cx.continue();
            record(node, cx, cx.pos, [byteAt(cx.node.from), byteAt(cx.node.to)], next);
            return cx.unit + cx.state.field(edits) + (next ?? 0);
        }})]});
        const state = EditorState.create({doc: source, selection: {anchor: caret}, extensions: [
            history(), indentOnInput(), indentUnit.of('   '), edits,
            markdown({codeLanguages: info => info === 'code' ? configured : null, defaultCodeLanguage: configured}),
            indentService.of((cx, pos) => { record('service', cx, pos, null, null); }),
            name === 'wast' ? language.data.of({indentOnInput: /^\s*\)$/}) : [],
        ]});
        const typed = state.update({...state.replaceSelection(insert), userEvent: 'input.type'});
        const result = projectInput(source, typed);
        const resultHead = projectedOffsets(result).byteAt(typed.state.selection.main.head);
        assert.equal(calls.length, 2);
        assert.equal(undoDepth(typed.state), 1);
        assert.ok(undo({state: typed.state, dispatch: tr => {
            assert.equal(projectInput(result, tr), source);
            assert.equal(tr.state.selection.main.head, caret);
        }}));
        console.log(JSON.stringify({name, label, node, body, insert, source, caret: byteAt(caret), candidate,
            result, resultHead, calls}));
    }
} finally {
    rmSync(root, {recursive: true, force: true});
}
