// Run: node crates/codemirror/tests/fixtures/wast_fold_node_prop.mjs <pinned archive directory>
import assert from 'node:assert/strict';
import {mkdtempSync, realpathSync, rmSync, symlinkSync} from 'node:fs';
import {tmpdir} from 'node:os';
import {join, resolve} from 'node:path';
import {pinnedModules} from './pinned_modules.mjs';

assert.equal(process.argv.length, 3, 'provide the directory of local npm archives');
const root = mkdtempSync(join(realpathSync(tmpdir()), 'gpui-wast-fold.'));
const path = process.env.PATH;
try {
    symlinkSync('/usr/bin/tar', join(root, 'tar'));
    process.env.PATH = root; // A missing archive cannot invoke curl.
    const {load} = pinnedModules(resolve(process.argv[2]), root);
    const {wastLanguage: stock} = await load('@codemirror/lang-wast');
    const {Compartment, EditorState} = await load('@codemirror/state');
    const {foldable, foldNodeProp, foldService, syntaxTree} = await load('@codemirror/language');
    const emit = (name, value, expected) => {
        assert.deepEqual(value, expected, name);
        console.log(JSON.stringify({name, value}));
    };
    const doc = '(module\n  (func)\n  1\n)\n(; note\nmore ;)\n"text"\n';
    const nested = '(module (func\n  1))\n';
    const comment = '(module (; note\nmore ;)\n)\n';
    const state = (doc, extensions) => EditorState.create({doc, extensions});
    const calls = [];
    const app = (node, state) => {
        assert.equal(node.type.prop(foldNodeProp), app);
        assert.equal(state.sliceDoc(node.from, node.from + 1), '(');
        calls.push([node.from, state.selection.main.head]);
        return {from: node.from + 2 + state.selection.main.head, to: node.to - 2};
    };
    const props = values => stock.configure({props: [foldNodeProp.add(type => values[type.name])]});
    const configured = props({App: app, BlockComment: null});
    const pair = range => range ? [range.from, range.to] : null;
    const query = (name, s, bounds, range, order = []) =>
        emit(name, [pair(foldable(s, ...bounds)), calls.splice(0)], [range, order]);
    const stockState = state(doc, stock), configuredState = state(doc, configured);
    for (const tree of [configured.parser.parse(doc), syntaxTree(configuredState)]) {
        const first = tree.topNode.firstChild;
        assert.equal(first.type.prop(foldNodeProp), app);
        assert.equal(first.nextSibling.type.prop(foldNodeProp), null);
    }
    query('stock app', stockState, [0, 7], [1, 21]);
    query('stock comment', stockState, [23, 30], [25, 36]);
    query('configured app', configuredState, [0, 7], [2, 20], [[0, 0]]);
    query('disabled comment', configuredState, [23, 30], null);
    query('disabled comment with ancestor', state(comment, configured), [0, 15], [2, 23], [[0, 0]]);
    const slot = new Compartment();
    const initial = state(nested, slot.of(configured));
    query('inner then outer', initial, [0, 13], [2, 17], [[8, 0], [0, 0]]);
    query('inner only', initial, [9, 13], [10, 16], [[8, 0]]);
    query('fold starts before line', initial, [11, 13], null, [[8, 0], [0, 0]]);
    query('node starts at line end', initial, [0, 8], [2, 17], [[8, 0], [0, 0]]);
    query('node ends at line end', initial, [0, 18], null, [[0, 0]]);
    const skipInner = props({App: (node, s) => {
        calls.push([node.from, s.selection.main.head]);
        return node.from === 0 ? {from: 2, to: 17} : null;
    }});
    query('callback skips inner', state(nested, skipInner), [0, 13], [2, 17], [[8, 0], [0, 0]]);
    const crossesNode = props({App: node => node.from === 8 ? {from: 13, to: 19} : null});
    query('range starts at line end and crosses node end', state(nested, crossesNode), [13, 13], [13, 19]);
    const selected = initial.update({selection: {anchor: 1}}).state;
    assert.equal(syntaxTree(selected), syntaxTree(initial));
    query('same tree, new state', selected, [0, 13], [3, 17], [[8, 1], [0, 1]]);
    const edited = selected.update({changes: {from: 16, to: 17, insert: '10'}}).state;
    assert.equal(syntaxTree(edited).topNode.firstChild.type.prop(foldNodeProp), app);
    query('same configuration after edit', edited, [0, 13], [3, 18], [[8, 1], [0, 1]]);
    const changed = selected.update({effects: slot.reconfigure(stock)}).state;
    assert.notEqual(syntaxTree(changed), syntaxTree(initial));
    query('reconfigured', changed, [0, 13], [1, 18]);
    query('retained state', selected, [0, 13], [3, 17], [[8, 1], [0, 1]]);
    query('service wins', state(nested, [configured, foldService.of(() => ({from: 4, to: 16}))]),
        [0, 13], [4, 16]);

    // Pinned JS has no native UTF-8/range validation. Rust tests the residuals.
    const tooFar = props({App: node => node.from === 0 ? {from: 2, to: 100} :
        {from: node.from + 2, to: node.to - 2}});
    query('upstream accepts past EOF', state(nested, tooFar), [0, 13], [2, 100]);
    const unicode = '(module "😀"\n  1)\n';
    const split = props({App: node => ({from: 10, to: node.to - 1})});
    query('upstream accepts split surrogate', state(unicode, split), [0, 12], [10, 16]);
    query('upstream queries reversed bounds', initial, [14, 13], null, [[8, 0], [0, 0]]);
} finally {
    process.env.PATH = path;
    rmSync(root, {recursive: true, force: true});
}
