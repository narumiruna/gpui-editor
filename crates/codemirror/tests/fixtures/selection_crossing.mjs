// Run from the repository root: node crates/codemirror/tests/fixtures/selection_crossing.mjs [archive-root]
import assert from 'node:assert/strict';
import {mkdtempSync, realpathSync, rmSync} from 'node:fs';
import {tmpdir} from 'node:os';
import {join, resolve} from 'node:path';
import {pinnedModules} from './pinned_modules.mjs';

const root = mkdtempSync(join(realpathSync(tmpdir()), 'selection-crossing.'));
process.on('exit', () => rmSync(root, {recursive: true, force: true}));
const {load, pins} = pinnedModules(resolve(process.argv[2] ?? '.'), root);
assert.equal(pins.get('@codemirror/state').version, '6.7.6');
assert.equal(pins.get('@codemirror/commands').version, '6.11.2');
assert.equal(pins.get('@codemirror/collab').version, '6.1.1');
const {EditorSelection: Selection, EditorState, Transaction} = await load('@codemirror/state');
const {history, isolateHistory, undo, redo, undoSelection, redoSelection, undoDepth, redoDepth} = await load('@codemirror/commands');
const {collab, receiveUpdates, getSyncedVersion, sendableUpdates} = await load('@codemirror/collab');

// Native bytes <-> UTF-16 offsets in CodeMirror's logical-line document.
// Reject the middle of a surrogate pair or a CRLF, rather than rounding.
function projection(text) {
    const bToU = new Map([[0, 0]]), uToB = new Map([[0, 0]]);
    let bytes = 0, units = 0;
    for (const {0: token} of text.matchAll(/\r\n|[\s\S]/gu)) {
        bytes += Buffer.byteLength(token);
        units += token == '\r\n' ? 1 : token.length;
        bToU.set(bytes, units);
        uToB.set(units, bytes);
    }
    const get = (map, offset) => {
        assert(map.has(offset), `not a projected boundary: ${offset}`);
        return map.get(offset);
    };
    return {toUnits: offset => get(bToU, offset), toBytes: offset => get(uToB, offset)};
}
const selection = (source, ranges, main) => {
    const {toUnits} = projection(source);
    return Selection.create(ranges.map(([a, h, assoc, bidi, goal, undirectional]) => {
        const anchor = toUnits(a), head = toUnits(h);
        return undirectional ? Selection.undirectionalRange(anchor, head) : anchor == head
            ? Selection.cursor(anchor, assoc, bidi, goal) : Selection.range(anchor, head, goal, bidi, assoc);
    }), main);
};
const snapshot = (range, toBytes) => [...[range.anchor, range.head, range.from, range.to].map(toBytes),
    range.assoc, range.bidiLevel, range.goalColumn ?? null, range.undirectional];
const selected = (value, toBytes) => ({main: value.mainIndex, ranges: value.ranges.map(r => snapshot(r, toBytes))});
const logical = source => source.replace(/\r\n|\r/g, '\n');
const make = (source, ranges, main = 0, extensions = []) => EditorState.create({doc: source,
    selection: selection(source, ranges, main), extensions: [EditorState.allowMultipleSelections.of(true), ...extensions]});
const edit = (source, edits) => edits.map(([start, end, insert]) => {
    const {toUnits} = projection(source);
    return {from: toUnits(start), to: toUnits(end), insert};
});
const applyBytes = (source, edits) => {
    const bytes = Buffer.from(source);
    let at = 0;
    const parts = [];
    for (const [from, to, insert] of edits) {
        parts.push(bytes.subarray(at, from), Buffer.from(insert));
        at = to;
    }
    return Buffer.concat([...parts, bytes.subarray(at)]).toString();
};
const emit = row => console.log(JSON.stringify(row));
function mapping(name, doc, ranges, main, changes, expected) {
    const state = make(doc, ranges, main);
    const next = applyBytes(doc, changes);
    const {toBytes} = projection(next);
    const tx = state.update({changes: edit(doc, changes)});
    const desc = tx.changes.desc;
    assert.equal(tx.state.doc.toString(), logical(next), name);
    const bare = state.selection.ranges.map(r => snapshot(r.map(desc), toBytes));
    const afterBare = state.selection.ranges.map(r => snapshot(r.map(desc, 1), toBytes));
    const mapped = selected(state.selection.map(desc), toBytes);
    const after = selected(state.selection.map(desc, 1), toBytes);
    assert.deepEqual(selected(tx.newSelection, toBytes), mapped, name);
    assert.deepEqual(selected(tx.state.selection, toBytes), mapped, name);
    assert.deepEqual(bare.map(r => r.slice(0, 4)), expected, name);
    const inputs = ranges.map(([a, h, assoc = 0, bidi = null, goal = null, undirectional = false]) =>
        [a, h, assoc, bidi, goal, undirectional]);
    emit({op: 'map', name, doc, ranges: inputs, main, changes, next, bare, afterBare, mapped, after});
}
const ascii = '0123456789';
const decorated = (a, h) => [a, h, 1, 4, 9.5];
for (const [name, [lo, hi], changes, bounds] of [
    ['strict', [3, 6], [[2, 8, 'XYZ']], [5, 2]],
    ['left-tie', [2, 6], [[2, 8, 'XYZ']], [2, 2]],
    ['right-tie', [3, 8], [[2, 8, 'XYZ']], [5, 5]],
    ['both-ties', [2, 8], [[2, 8, 'XYZ']], [2, 5]],
    ['overlap-left', [1, 6], [[2, 8, 'XYZ']], [1, 2]],
    ['overlap-right', [3, 9], [[2, 8, 'XYZ']], [5, 6]],
    ['strict-deletion', [3, 6], [[2, 8, '']], [2, 2]],
    ['different-adjacent-replacements', [3, 7], [[2, 5, 'X'], [5, 8, 'YZ']], [3, 3]],
    ['insertion-at-lower', [3, 6], [[3, 3, 'XY']], [5, 8]],
    ['insertion-at-upper', [3, 6], [[6, 6, 'XY']], [3, 6]],
    ['insertion-inside', [3, 6], [[4, 4, 'XY']], [3, 8]],
    ['unrelated', [3, 6], [[8, 8, 'XY']], [3, 6]],
]) {
    mapping(`${name}-forward`, ascii, [decorated(lo, hi)], 0, changes, [[...bounds, ...bounds]]);
    mapping(`${name}-reverse`, ascii, [decorated(hi, lo)], 0, changes, [[...bounds.toReversed(), ...bounds]]);
}
mapping('cursor-insertion', ascii, [decorated(3, 3)], 0, [[3, 3, 'XY']], [[3, 3, 3, 3]]);
mapping('cursor-inside-replacement', ascii, [decorated(3, 3)], 0, [[2, 8, 'XYZ']], [[2, 2, 2, 2]]);
mapping('cursor-at-replacement-start', ascii, [decorated(2, 2)], 0, [[2, 8, 'XYZ']], [[2, 2, 2, 2]]);
mapping('cursor-at-replacement-end', ascii, [decorated(8, 8)], 0, [[2, 8, 'XYZ']], [[5, 5, 5, 5]]);
mapping('strict-undirectional', ascii, [[3, 6, 0, 0, null, true]], 0, [[2, 8, 'XYZ']], [[5, 2, 5, 2]]);
for (const [doc, anchor, head, end, insert, edge] of [
    ['aé🦊Ωz', 3, 7, 9, '漢😀', 8],
    ['a\r\né🦊Ωz', 5, 9, 11, '漢\r\n😀', 10],
]) {
    const kind = doc.includes('\r') ? 'projected-crlf' : 'unicode';
    mapping(`strict-${kind}-forward`, doc, [decorated(anchor, head)], 0, [[1, end, insert]], [[edge, 1, edge, 1]]);
    mapping(`strict-${kind}-reverse`, doc, [decorated(head, anchor)], 0, [[1, end, insert]], [[1, edge, edge, 1]]);
}
assert.throws(() => projection('a🦊z').toBytes(2));
assert.throws(() => projection('a🦊z').toUnits(2));
assert.throws(() => projection('a\r\nz').toUnits(2));
const split = EditorState.create({doc: 'a🦊z', selection: Selection.single(2)});
assert.equal(split.selection.main.head, 2);
for (const main of [1, 2, 3]) {
    mapping(`coincident-spans-main-${main}`, ascii,
        [[0, 1], decorated(3, 4), decorated(7, 6), [9, 10]], main, [[2, 8, 'XYZ']],
        [[0, 1, 0, 1], [5, 2, 5, 2], [2, 5, 5, 2], [6, 7, 6, 7]]);
}
mapping('touching-span-and-cursor', ascii,
    [[0, 2], decorated(3, 6), decorated(8, 8), [9, 10]], 2, [[2, 8, 'XYZ']],
    [[0, 2, 0, 2], [5, 2, 5, 2], [5, 5, 5, 5], [6, 7, 6, 7]]);
mapping('deletion-merge-control', ascii,
    [[0, 2], decorated(3, 4), decorated(7, 6), [9, 10]], 2, [[2, 8, '']],
    [[0, 2, 0, 2], [2, 2, 2, 2], [2, 2, 2, 2], [3, 4, 3, 4]]);

for (const [name, initial] of [['forward', [3, 6]], ['reverse', [6, 3]]]) {
    let state = make(ascii, [initial]);
    state = state.update({changes: {from: 2, to: 8, insert: 'XYZ'}}).state;
    const json = state.selection.toJSON();
    const restored = Selection.fromJSON(json);
    const changes = state.changes([{from: 2, insert: '<'}, {from: 5, insert: '>'}]);
    const raw = state.selection.map(changes), ordered = restored.map(changes);
    assert.deepEqual([raw.main.from, raw.main.to], [7, 2]);
    assert.deepEqual([ordered.main.from, ordered.main.to], [3, 6]);
    emit({op: 'remap', name, initial, json, raw: selected(raw, Number), ordered: selected(ordered, Number)});
}

const state = make(ascii, [[3, 6]]);
const replacement = {from: 2, to: 8, insert: 'XYZ'};
const expectation = (name, tx) => emit({op: 'api', name, selection: selected(tx.state.selection, Number)});
expectation('merged-explicit-selection', state.update({selection: Selection.single(3, 6)}, {changes: replacement}));
expectation('same-spec-explicit-selection', state.update({changes: replacement, selection: Selection.single(1, 2)}));
expectation('same-text-replacement', state.update({changes: {...replacement, insert: '234567'}}));
const two = make(ascii, [[1, 1], [9, 9]], 1);
expectation('change-by-range', two.update(two.changeByRange(range => range.from == 1
    ? {range: Selection.range(3, 6)} : {range: Selection.cursor(7), changes: replacement})));
for (const insert of ['XYZ', '234567']) {
    const client = make(ascii, [[3, 6]], 0, [collab({clientID: 'local'})]);
    const received = receiveUpdates(client, [{clientID: 'peer', changes: client.changes({...replacement, insert})}]).state;
    emit({op: 'collab', insert, doc: received.doc.toString(), selection: selected(received.selection, Number),
        version: getSyncedVersion(received), queued: sendableUpdates(received).length});
}

function replay(name, source, initial, steps) {
    let doc = source;
    let state = make(doc, [initial], 0, [history()]);
    const snapshots = [];
    for (const [op, data] of steps) {
        switch (op) {
            case 'local':
            case 'remote': {
                const annotations = op == 'local' ? isolateHistory.of('full') : Transaction.addToHistory.of(false);
                state = state.update({changes: edit(doc, data), annotations}).state;
                doc = applyBytes(doc, data);
                break;
            }
            case 'select':
                state = state.update({selection: selection(doc, [data], 0),
                    annotations: [Transaction.userEvent.of('select.pointer'), isolateHistory.of('full')]}).state;
                break;
            default:
                assert(({undo, redo, undoSelection, redoSelection})[op]({state, dispatch: tr => { state = tr.state; }}), `${name}: ${op}`);
                doc = state.doc.toString(); // History inputs below use only LF or no line breaks.
        }
        assert.equal(state.doc.toString(), logical(doc));
        snapshots.push({op, doc, selection: selected(state.selection, projection(doc).toBytes),
            undo: undoDepth(state), redo: redoDepth(state)});
    }
    emit({op: 'history', name, source, initial, steps, snapshots});
}
replay('local-crossed-undo-redo', 'aé🦊Ωz', [3, 7], [
    ['local', [[1, 9, '漢😀']]], ['undo'], ['redo'], ['undo'], ['redo'],
]);
replay('remote-maps-both-branches', ascii, [6, 3], [
    ['local', [[10, 10, 'A']]], ['local', [[11, 11, 'B']]], ['undo'],
    ['remote', [[2, 8, 'XYZ']]], ['undo'], ['redo'], ['redo'],
]);
replay('remote-maps-selection-replay', ascii, [3, 6], [
    ['select', [0, 0]], ['select', [9, 9]], ['undoSelection'],
    ['remote', [[2, 8, 'XYZ']]], ['undoSelection'], ['redoSelection'], ['redoSelection'],
]);
replay('remote-deletion-control', ascii, [6, 3], [
    ['local', [[10, 10, 'A']]], ['local', [[11, 11, 'B']]], ['undo'],
    ['remote', [[2, 8, '']]], ['undo'], ['redo'], ['redo'],
]);
