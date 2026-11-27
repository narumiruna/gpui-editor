// Run: node crates/codemirror/tests/fixtures/wast_indent_props.mjs <pinned archive directory>
// The loader verifies SHA-512 pins in docs/codemirror-upstream.json.
import assert from 'node:assert/strict';
import {mkdtempSync, realpathSync, rmSync, symlinkSync} from 'node:fs';
import {tmpdir} from 'node:os';
import {join, resolve} from 'node:path';
import {pinnedModules} from './pinned_modules.mjs';

assert.equal(process.argv.length, 3, 'provide the directory of local npm archives');
const root = mkdtempSync(join(realpathSync(tmpdir()), 'gpui-wast-indent-props.'));
const path = process.env.PATH;
try {
    // Keep this run offline, even when an archive is missing.
    symlinkSync('/usr/bin/tar', join(root, 'tar'));
    process.env.PATH = root;
    const {load} = pinnedModules(resolve(process.argv[2]), root);
    const {wastLanguage: stock} = await load('@codemirror/lang-wast');
    const {Compartment, EditorState} = await load('@codemirror/state');
    const {getIndentation, IndentContext, indentNodeProp, indentUnit, syntaxTree} =
        await load('@codemirror/language');
    const source = '\t(module\r\n  (func (; 😀 ;) i32.const 0)\r  (func i32.const 1)\r\n)\r\n';
    const emit = (name, value) => console.log(JSON.stringify({name, value}));
    // Convert the state's normalized UTF-16 boundaries to original UTF-8 bytes.
    function byteOffsets(raw) {
        let units = 0, bytes = 0;
        const result = [0];
        for (const [part] of raw.matchAll(/\r\n|[\s\S]/gu)) {
            bytes += Buffer.byteLength(part);
            units += part === '\r\n' ? 1 : part.length;
            result[units] = bytes;
        }
        return result;
    }

    let events, byte;
    function enter(cx) {
        const line = cx.state.doc.lineAt(cx.pos);
        assert.equal(cx.state.tabSize, 8);
        events.push([cx.node.name, [byte[cx.node.from], byte[cx.node.to]], byte[cx.pos],
            [byte[line.from], byte[line.to]], cx.textAfter, cx.unit, cx.state.readOnly]);
    }
    const result = (name, column) => (events.push([name, column]), column);
    const app = extra => cx => {
        enter(cx);
        const column = cx.node.from === 1 ? cx.baseIndent + extra : cx.continue() + cx.unit;
        return result('App', column);
    };
    const beforeApp = app(7), afterApp = app(13);
    const keyword = cx => {
        enter(cx);
        return result('Keyword', cx.state.doc.lineAt(cx.pos).number === 3 ? null : cx.continue() + 1);
    };
    const before = stock.configure({props: [indentNodeProp.add({App: beforeApp, Keyword: keyword})]});
    const sameRows = before.configure({});
    const after = before.configure({props: [indentNodeProp.add({App: afterApp})]});
    const slot = new Compartment();
    const state = EditorState.create({doc: source, extensions: [slot.of(before),
        EditorState.readOnly.of(true), EditorState.tabSize.of(8), indentUnit.of('   ')]});

    function measure(state, raw, position, options = {}) {
        assert.equal(state.doc.toString(), raw.replace(/\r\n?/g, '\n'));
        events = [];
        byte = byteOffsets(raw);
        return {position: byte[position], column: getIndentation(new IndentContext(state, options), position), events};
    }
    const text = state.doc.toString();
    const first = text.indexOf('const') + 1, second = text.lastIndexOf('const') + 1;
    const queries = [first, second, state.doc.line(4).from, state.doc.length]
        .map(pos => measure(state, source, pos));
    queries.push(measure(state, source, first, {
        simulateBreak: first, simulateDoubleBreak: true,
        overrideIndentation: at => at === 0 ? 5 : -1,
    }));
    assert.deepEqual(queries.map(q => q.column), [19, null, 15, 0, 16]);
    assert.deepEqual(queries[0].events.map(event => event[0]),
        ['Keyword', 'App', 'App', 'App', 'App', 'Keyword']);
    assert.equal(queries[1].events.length, 2); // null must not call an ancestor
    emit('source', source);
    emit('queries', queries);

    const update = spec => state.update(spec).state;
    const install = lang => slot.reconfigure(lang);
    const tree = syntaxTree(state);
    const identity = [update({selection: {anchor: first}}), update({effects: install(before)}),
        update({effects: install(sameRows)})].map(next => syntaxTree(next) === tree);
    assert.deepEqual(identity, [true, true, false]);
    assert.ok(sameRows.parser.nodeSet.types.every((row, i) => row === before.parser.nodeSet.types[i]));
    const from = text.indexOf('0'), editAt = byteOffsets(source)[from];
    const editedSource = source.replace('0', '10');
    const edited = [before, sameRows, after].map((lang, i) => {
        const next = update({changes: {from, to: from + 1, insert: '10'}, effects: install(lang)});
        const editedTree = syntaxTree(next);
        const raw = lang.parser.parse(editedSource);
        const expectedApp = i === 2 ? afterApp : beforeApp;
        for (const current of [editedTree, raw]) current.iterate({enter(node) {
            if (node.name === 'App') assert.equal(node.type.prop(indentNodeProp), expectedApp);
            if (node.name === 'Keyword') assert.equal(node.type.prop(indentNodeProp), keyword);
        }});
        const appRow = editedTree.topNode.firstChild.type;
        return [editedTree === tree, appRow === tree.topNode.firstChild.type,
            measure(next, editedSource, first).column];
    });
    assert.deepEqual(edited, [[false, true, 19], [false, true, 19], [false, false, 25]]);
    emit('identity', {identity, editAt, editedSource, edited});
} finally {
    if (path === undefined) delete process.env.PATH;
    else process.env.PATH = path;
    rmSync(root, {recursive: true, force: true});
}
