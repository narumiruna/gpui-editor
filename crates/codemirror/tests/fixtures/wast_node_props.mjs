// Run: node crates/codemirror/tests/fixtures/wast_node_props.mjs <pinned archive directory>
// The loader checks the SHA-512 pins in docs/codemirror-upstream.json.
import assert from 'node:assert/strict';
import {mkdtempSync, realpathSync, rmSync, symlinkSync} from 'node:fs';
import {tmpdir} from 'node:os';
import {join, resolve} from 'node:path';
import {pinnedModules} from './pinned_modules.mjs';

assert.equal(process.argv.length, 3, 'provide the directory of local npm archives');
const root = mkdtempSync(join(realpathSync(tmpdir()), 'gpui-wast-node-props.'));
const path = process.env.PATH;
try {
    // The loader can invoke tar, but a missing archive cannot invoke curl.
    symlinkSync('/usr/bin/tar', join(root, 'tar'));
    process.env.PATH = root;
    const {load} = pinnedModules(resolve(process.argv[2]), root);
    const {wastLanguage: stock} = await load('@codemirror/lang-wast');
    const {NodeProp, Tree} = await load('@lezer/common');
    const {Compartment, EditorState} = await load('@codemirror/state');
    const {foldable, getIndentation, matchBrackets, syntaxTree} = await load('@codemirror/language');
    const rows = lang => lang.parser.nodeSet.types;
    const app = lang => rows(lang)[6];
    const emit = (name, value) => console.log(JSON.stringify({name, value}));
    const events = [];
    const merged = new NodeProp({combine: (old, next) => {
        events.push(['combine', old, next]);
        return `${old}+${next}`;
    }});
    const value = new NodeProp(), unused = new NodeProp();
    const token = Object.freeze({label: 'shared value'});
    const seeded = stock.configure({props: [merged.add({App: 'seed'})]});
    const source = next => merged.add(t => {
        events.push([next, t.id, t.name, t.prop(merged) ?? null]);
        return t.name === 'App' ? next : undefined;
    });
    const configured = seeded.configure({props: [source('A'), source('B'),
        NodeProp.group.add({App: ['Alias']}), unused.add({Alias: token}),
        value.add({App: token, Expression: 'group'}),
        value.add(t => ({Number: 0, String: false, Keyword: null})[t.name]),
    ]});
    const inventory = lang => rows(lang).map(t => [t.id, t.name, t.isTop, t.isSkipped, t.isError,
        t.isAnonymous, t.prop(NodeProp.group) ?? null]);
    const names = ['⚠', 'LineComment', 'BlockComment', 'Module', ')', '(', 'App', 'Identifier',
        'Type', 'Keyword', 'Number', 'String', ''];
    assert.deepEqual(inventory(stock), names.map((name, id) => [id, name, id === 3, id < 3,
        id === 0, id === 12, id >= 6 && id <= 11 ? ['Expression'] : null]));
    assert.deepEqual(events, rows(stock).flatMap(t => t.name === 'App'
        ? [['A', 6, 'App', 'seed'], ['combine', 'seed', 'A'], ['B', 6, 'App', 'seed'], ['combine', 'seed+A', 'B']]
        : [['A', t.id, t.name, null], ['B', t.id, t.name, null]]));
    emit('stock', inventory(stock));
    emit('order', events);
    const changed = rows(configured).filter((t, i) => t !== rows(seeded)[i]).map(t => t.id);
    assert.deepEqual(changed, [6, 7, 8, 9, 10, 11]);
    emit('changed', changed);
    assert.equal(app(configured).prop(merged), 'seed+A+B');
    assert.equal(app(seeded).prop(merged), 'seed');
    assert.ok(app(configured).is('Alias') && !app(configured).is('Expression'));
    assert.equal(app(configured).prop(unused), undefined);
    assert.equal(app(configured.configure({props: [unused.add({Alias: token})]})).prop(unused), token);
    assert.notEqual(app(configured.configure({props: [value.add({App: token})]})), app(configured));
    assert.deepEqual(rows(configured).map(t => t.prop(value)),
        [undefined, undefined, undefined, undefined, undefined, undefined, token, 'group', 'group', null, 0, false, undefined]);
    assert.ok(rows(stock).every(t => t.prop(value) === undefined));
    const empty = configured.configure({props: []});
    assert.notEqual(empty.parser.nodeSet, configured.parser.nodeSet);
    assert.ok(rows(empty).every((t, i) => t === rows(configured)[i]));

    const doc = '(module\n  (func)\n  1\n)\n(; note\nmore ;)\n"text"\n';
    const state = EditorState.create({doc, extensions: configured});
    for (const tree of [configured.parser.parse(doc), syntaxTree(state)]) {
        assert.equal(tree.topNode.firstChild.type, app(configured));
        assert.equal(tree.topNode.firstChild.type.prop(value), token);
        assert.equal(tree.topNode.lastChild.type.prop(value), false);
    }
    assert.equal(events.length, 28); // No source ran during parsing or installation.
    const pair = r => r == null ? null : [r.from, r.to];
    const brackets = {brackets: '[]', maxScanDistance: 1};
    const match = matchBrackets(state, 0, 1, brackets);
    const queries = {
        indent: [17, 2, 12].map(at => getIndentation(state, at)),
        fold: [[0, 7], [23, 30]].map(([from, to]) => pair(foldable(state, from, to))),
        brackets: [pair(match.start), pair(match.end), match.matched],
    };
    assert.deepEqual(queries, {indent: [2, 2, 4], fold: [[1, 21], [25, 36]], brackets: [[0, 1], [21, 22], true]});
    emit('consumers', queries);

    const marker = new NodeProp();
    const before = stock.configure({bufferLength: 16, props: [marker.add({'App Keyword': 'old'})]});
    const sameRows = before.configure({});
    const after = before.configure({props: [marker.add({'App Keyword': 'new'})]});
    const slot = new Compartment();
    const long = '(module (func i32.const 0) (func i32.const 0))\n'.repeat(12);
    const initial = EditorState.create({doc: long, extensions: [slot.of(before), EditorState.readOnly.of(true)]});
    const update = spec => syntaxTree(initial.update(spec).state);
    const original = syntaxTree(initial);
    const identities = [update({selection: {anchor: 1}}), update({effects: slot.reconfigure(before)}),
        update({effects: slot.reconfigure(sameRows)})].map(tree => tree === original);
    assert.deepEqual(identities, [true, true, false]);
    emit('state identity', identities);
    const objects = (tree, into = new Set()) => {
        into.add(tree);
        if (tree instanceof Tree) for (const child of tree.children) objects(child, into);
        return into;
    };
    const originals = objects(original);
    const from = long.indexOf('0');
    const changes = {from, to: from + 1, insert: '10'};
    const reuse = [before, sameRows, after].map(lang => {
        const tree = update({changes, effects: slot.reconfigure(lang)});
        const counts = {old: 0, new: 0};
        tree.iterate({enter(n) {const mark = n.type.prop(marker); if (mark) counts[mark]++;}});
        return [[...objects(tree)].filter(obj => originals.has(obj)).length, counts.old, counts.new];
    });
    assert.deepEqual(reuse, [[173, 96, 0], [0, 96, 0], [0, 0, 96]]);
    emit('state markers', reuse.map(([, old, next]) => [old, next]));
} finally {
    if (path === undefined) delete process.env.PATH;
    else process.env.PATH = path;
    rmSync(root, {recursive: true, force: true});
}
