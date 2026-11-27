// node crates/codemirror/tests/fixtures/wast_host_commands.mjs <archive-root>
import assert from 'node:assert/strict';
import {mkdtempSync, realpathSync, rmSync} from 'node:fs';
import {tmpdir} from 'node:os';
import {join, resolve} from 'node:path';
import {projectedOffsets, projectInput} from './editor_line_offsets.mjs';
import {pinnedModules} from './pinned_modules.mjs';

const root = realpathSync(mkdtempSync(join(tmpdir(), 'wast-host-commands-')));
try {
    const {load} = pinnedModules(resolve(process.argv[2]), root);
    const {EditorState} = await load('@codemirror/state');
    const {foldNodeProp, foldable} = await load('@codemirror/language');
    const {wastLanguage} = await load('@codemirror/lang-wast');
    const {markdown} = await load('@codemirror/lang-markdown');
    const {toggleComment} = await load('@codemirror/commands');
    const prefix = 'Outside 🧭\r\n\r\n```wat\r\n';
    const body = '(module\r  (func $a)\r\n)';
    const source = `${prefix}${body}\r\n\x60\x60\x60\r\nTail.`;
    const {byteAt} = projectedOffsets(source);
    const installed = (language, extra) => EditorState.create({doc: source,
        selection: {anchor: prefix.replace(/\r\n?/g, '\n').length},
        extensions: [markdown({codeLanguages: name => name === 'wat' ? language : null}), extra]});
    const calls = [];
    const fold = (node, state) => {
        assert.equal(state.sliceDoc(node.from, node.from + 1), '(');
        assert.ok(state.doc.toString().startsWith('Outside 🧭'));
        calls.push([byteAt(node.from), byteAt(node.to), byteAt(state.selection.main.head)]);
        return {from: node.from + 2, to: node.to - 1};
    };
    for (const [name, callback] of [['fold', fold], ['disabled fold', null]]) {
        const language = wastLanguage.configure({props: [foldNodeProp.add(type => type.name === 'App' ? callback : undefined)]});
        const state = installed(language, []);
        const line = state.doc.lineAt(state.selection.main.head);
        const value = foldable(state, line.from, line.to);
        console.log(JSON.stringify({name, source, body, line: [byteAt(line.from), byteAt(line.to)],
            selection: byteAt(state.selection.main.head), calls: calls.splice(0),
            range: value ? [byteAt(value.from), byteAt(value.to)] : null}));
    }
    for (const [name, tokens] of [['comment override', {line: ';; host'}], ['disabled comment', {}]]) {
        const state = installed(wastLanguage, wastLanguage.data.of({commentTokens: tokens}));
        const func = state.doc.toString().indexOf('(func');
        const selected = state.update({selection: {anchor: func}}).state;
        let transaction;
        const handled = toggleComment({state: selected, dispatch(tr) { transaction = tr; }});
        console.log(JSON.stringify({name, source, body, selection: byteAt(func),
            text: handled ? projectInput(source, transaction) : null}));
    }
} finally {
    rmSync(root, {recursive: true, force: true});
}
