// Run from the repository root with Node 22.14+ and the pinned archive directory:
// node crates/codemirror/tests/fixtures/markdown_editor_lines.mjs <archive-root>
import assert from 'node:assert/strict';
import {mkdtempSync, realpathSync, rmSync} from 'node:fs';
import {tmpdir} from 'node:os';
import {join, resolve} from 'node:path';
import {pinnedModules} from './pinned_modules.mjs';

const lf = '# 😀\n\n```wat options\n(module\n (func $f))\n```\n\n```json\n{"漢":"😀"}\n```\n\n```plain\ntext\n```\n';
const cases = [
    ['audit', '```wast\r(module)\r```'],
    ['LF', lf], ['CRLF', lf.replaceAll('\n', '\r\n')], ['CR', lf.replaceAll('\n', '\r')],
    ['quoted', '> ```wast\r> (module ;; 😀\r>  (func $f))\r> ```\r\rTail'],
    ['seam edits', '# 😀\r\n\r\nHands off\r\n\r\n```wat\r\n(module)\r\n```\r\n\r\n```json\r\n{"漢":"😀"}\r\n```\r\n\r\n# Anchor\r\n\r\nTail', [
        ['```wat\r\n', 7, 8, ''],
        ['```wat\r(', 7, 7, '\n'],
        ['\r\nHands off', 1, 1, '\r'],
        ['\r\r\nHands off', 1, 2, ''],
        ['(module)\r\n```\r\n\r\n```json', 0, 24, '(module ;; 😀\r (func))\r```\r\r```json'],
        ['```json\r\n', 7, 9, ' '],
        ['```json {', 7, 8, '\r'],
    ]],
    ['stale-prefix', '# A\r\n\r# B\r\n# C\r\nTail', [
        ['\r# B', 1, 4, '\nBody'],
        ['\r\nBody', 1, 2, 'x\n'],
        ['\rx\nBody', 1, 2, ''],
    ]],
];
const normalize = raw => raw.replace(/\r\n?/g, '\n');
const bytes = text => Buffer.byteLength(text);
const emit = row => console.log(JSON.stringify(row));

// One entry per normalized UTF-16 boundary, in raw UTF-8 bytes. Do not map
// surrogate interiors. The two bytes of CRLF map to one logical break.
function byteMap(raw) {
    const map = [0];
    let to = 0;
    for (const ch of raw.match(/\r\n|[^]/gu) ?? []) {
        if (ch.length === 2 && ch !== '\r\n') map.push(null);
        map.push(to += bytes(ch));
    }
    return map;
}

const scratch = realpathSync(mkdtempSync(join(tmpdir(), 'markdown-editor-lines-')));
try {
    const {pins, load} = pinnedModules(resolve(process.argv[2]), scratch);
    const {EditorState} = await load('@codemirror/state');
    const {markdown, insertNewlineContinueMarkup, deleteMarkupBackward} = await load('@codemirror/lang-markdown');
    const {wastLanguage} = await load('@codemirror/lang-wast');
    const {jsonLanguage} = await load('@codemirror/lang-json');
    const {forceParsing, syntaxTree, foldable} = await load('@codemirror/language');
    const {NodeProp, IterMode} = await load('@lezer/common');
    const packages = ['@codemirror/lang-markdown', '@codemirror/lang-wast', '@codemirror/lang-json',
        '@lezer/markdown', '@codemirror/state'].map(packageName => {
        const {name, version, tarball, integrity} = pins.get(packageName);
        return {name, version, tarball, integrity};
    });
    emit({capture: 'Default Markdown and controlled WAT/WAST/JSON resolver on editor lines', packages});

    const configs = [{}, {codeLanguages: name => ({wat: wastLanguage, wast: wastLanguage, json: jsonLanguage})[name]}];
    const complete = state => {
        const view = {state, dispatch(spec) { this.state = this.state.update(spec).state; }};
        assert.ok(forceParsing(view, state.doc.length, 5000));
        return view.state;
    };
    const open = (raw, config) => {
        const tail = raw.indexOf('Tail');
        const head = tail < 0 ? 0 : normalize(raw.slice(0, tail)).length + 1;
        const anchor = tail < 0 ? 0 : head + 3;
        return complete(EditorState.create({doc: raw, selection: {anchor, head}, extensions: markdown(config)}));
    };
    const languageAt = (state, at, side) => [wastLanguage, jsonLanguage]
        .find(language => language.isActiveAt(state, at, side))?.name ?? 'markdown';
    function treeRows(tree, byte, base = 0) {
        const rows = [];
        let depth = 0;
        tree.iterate({mode: IterMode.IgnoreMounts,
            enter(node) { rows.push([depth++, node.name, byte(base + node.from), byte(base + node.to)]); },
            leave() { --depth; },
        });
        return rows;
    }
    function snapshot(state, raw) {
        assert.equal(state.doc.toString(), normalize(raw));
        const map = byteMap(raw);
        const byte = at => {
            assert.equal(typeof map[at], 'number', `invalid normalized boundary ${at}`);
            return map[at];
        };
        const tree = syntaxTree(state), mounts = [], probes = new Set(), folds = [];
        for (let cursor = tree.cursor(IterMode.IgnoreMounts);;) {
            const mount = cursor.tree?.prop(NodeProp.mounted);
            if (mount?.overlay) {
                const ranges = mount.overlay.map(({from, to}) => [cursor.from + from, cursor.from + to]);
                for (const [from, to] of ranges) {
                    for (const at of [from, from + 1, to - 1, to]) {
                        if (typeof map[at] === 'number') probes.add(at);
                    }
                }
                mounts.push({ranges: ranges.map(range => range.map(byte)),
                    source: ranges.map(range => state.sliceDoc(...range)).join(''),
                    tree: treeRows(mount.tree, byte, ranges[0][0])});
            }
            if (!cursor.next()) break;
        }
        const assoc = [...probes].sort((a, b) => a - b).map(at => [byte(at),
            ...[-1, 1].flatMap(side => [languageAt(state, at, side), tree.resolveInner(at, side).name])]);
        for (let i = 1; i <= state.doc.lines; ++i) {
            const {from, to} = state.doc.line(i);
            const fold = foldable(state, from, to);
            if (fold) folds.push([byte(from), byte(to), byte(fold.from), byte(fold.to)]);
        }
        const {anchor, head} = state.selection.main;
        return {host: treeRows(tree, byte), selection: [byte(anchor), byte(head)], mounts, assoc, folds};
    }
    function capture(name, raw, states, edit, cm) {
        const [stock, controlled] = states.map(state => snapshot(state, raw));
        assert.deepEqual(stock.host, controlled.host);
        assert.deepEqual(stock.selection, controlled.selection);
        const map = byteMap(raw);
        for (const [at] of controlled.assoc) {
            for (const side of [-1, 1]) assert.equal(languageAt(states[0], map.indexOf(at), side), 'markdown');
        }
        const {host, selection, ...defaults} = stock;
        const {mounts, assoc, folds} = controlled;
        emit({name, raw, edit, cm, host, selection, default: defaults, controlled: {mounts, assoc, folds}});
    }
    for (const [name, initial, edits = []] of cases) {
        let raw = initial, states = configs.map(config => open(initial, config));
        capture(name, raw, states);
        for (const [find, from, to, insert] of edits) {
            const index = raw.indexOf(find);
            assert.ok(index >= 0, find);
            const start = index + from, end = index + to;
            const edit = [bytes(raw.slice(0, start)), bytes(raw.slice(0, end)), insert];
            // Include adjacent break bytes so rejoining or splitting a CRLF is
            // expressed as a change to whole logical editor lines.
            const a = raw[start - 1] === '\r' ? start - 1 : start;
            const b = raw[end] === '\n' ? end + 1 : end;
            const cmFrom = normalize(raw.slice(0, a)).length;
            const cmTo = cmFrom + normalize(raw.slice(a, b)).length;
            const cmInsert = normalize(raw.slice(a, start) + insert + raw.slice(end, b));
            raw = raw.slice(0, start) + insert + raw.slice(end);
            states = states.map((state, i) => {
                const updated = complete(state.update({changes: {from: cmFrom, to: cmTo, insert: cmInsert}}).state);
                assert.deepEqual(snapshot(updated, raw), snapshot(open(raw, configs[i]), raw));
                return updated;
            });
            capture(name, raw, states, edit, [cmFrom, cmTo, cmInsert]);
        }
    }

    // Command observations document the out-of-scope raw-line policy.
    for (const [raw, caret, command] of [
        ['lead\r- alpha\r- beta', 12, 'continue'],
        ['lead\r> one\r> two', 10, 'continue'],
        ['lead\r> one\r> two', 13, 'backspace'],
        ['lead\r- one\r- two', 13, 'backspace'],
    ]) {
        const run = command === 'continue' ? insertNewlineContinueMarkup : deleteMarkupBackward;
        const state = open(raw, {}).update({selection: {anchor: normalize(raw.slice(0, caret)).length}}).state;
        let result;
        assert.ok(run({state, dispatch(spec) { result = spec.state; }}));
        emit({name: 'markup', raw, caret, command, out: result.doc.toString(), cursor: result.selection.main.head});
    }
} finally {
    rmSync(scratch, {recursive: true, force: true});
}
