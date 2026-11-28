// node crates/codemirror/tests/fixtures/capture_browser_fixture.mjs <archive-dir> <chrome> @codemirror/merge crates/codemirror/tests/fixtures/merge_override.mjs
import {
    Change, Chunk, MergeView, diff, getChunks, getOriginalDoc, originalDocChangeEffect,
    presentableDiff, unifiedMergeView,
} from '@codemirror/merge';
import {ChangeSet, Compartment, EditorState, Text} from '@codemirror/state';

function check(condition, message) {
    if (!condition) throw Error(message);
}

function equal(actual, expected) {
    check(JSON.stringify(actual) === JSON.stringify(expected),
          `expected ${JSON.stringify(expected)}, got ${JSON.stringify(actual)}`);
}

const encoder = new TextEncoder();
const text = s => Text.of(s.split('\n'));
const rows = [];
const log = row => rows.push(JSON.stringify(row));

function bytePositions(text) {
    let units = 0, bytes = 0;
    const offsets = new Map([[0, 0]]);
    for (const scalar of text) offsets.set(units += scalar.length, bytes += encoder.encode(scalar).length);
    return pos => {
        check(offsets.has(pos), `non-scalar endpoint: ${pos}`);
        return offsets.get(pos);
    };
}

function ranges(a, b, changes, offA = 0, offB = 0) {
    const startA = a(offA), startB = b(offB);
    return changes.map(({fromA, toA, fromB, toB}) => [
        a(offA + fromA) - startA, a(offA + toA) - startA,
        b(offB + fromB) - startB, b(offB + toB) - startB,
    ]);
}

function chunks(a, b, result) {
    const ma = bytePositions(a), mb = bytePositions(b);
    // The native chunk stops at real EOF instead of the upstream EOF + 1.
    return result.map(({fromA, toA, fromB, toB, precise, changes}) => ({
        range: [ma(fromA), ma(Math.min(toA, a.length)), mb(fromB), mb(Math.min(toB, b.length))],
        changes: ranges(ma, mb, changes, fromA, fromB),
        precise,
    }));
}

function config(override) {
    return {
        override,
        get scanLimit() { throw Error('override read scanLimit'); },
        get timeout() { throw Error('override read timeout'); },
    };
}

function captureChunks() {
    for (const [name, a, b, script] of [
        ['equal-empty', '', '', []],
        ['equal-keeps-range', 'same', 'same', [[1, 2, 1, 2]]],
        ['wide-replacement', 'foo cat', 'foo cot', [[0, 7, 0, 7]]],
        ['display-word-expansion', 'head\ncat\nfoot\n', 'head\ncut\nfoot\n', [[6, 7, 6, 7]]],
        ['display-joins-insertions', 'a-b', 'aX-Yb', [[1, 1, 1, 2], [2, 2, 3, 4]]],
        ['display-keeps-empty-split', 'a---b', 'A---B', [[0, 1, 0, 1], [2, 2, 2, 2], [4, 5, 4, 5]]],
        ['utf16-to-utf8', 'head\n😀 café\nfoot', 'head\n😀 caff\nfoot', [[11, 12, 11, 12]]],
    ]) {
        function run(compute) {
            let calls = 0;
            const supplied = script.map(row => new Change(...row));
            // Reset the pin's ambient precision. Native overrides have no such state.
            diff('', '');
            const result = compute(config((receivedA, receivedB) => {
                equal(receivedA, a);
                equal(receivedB, b);
                calls++;
                return supplied;
            }));
            equal(calls, 1);
            return result;
        }
        const ma = bytePositions(a), mb = bytePositions(b);
        const raw = ranges(ma, mb, run(conf => diff(a, b, conf)));
        equal(raw, ranges(ma, mb, script.map(row => new Change(...row))));
        const display = ranges(ma, mb, run(conf => presentableDiff(a, b, conf)));
        const result = run(conf => Chunk.build(text(a), text(b), conf));
        log({name, a, b, raw, display, chunks: chunks(a, b, result)});
    }

    // These ASCII edits meet both implementations' window boundaries. Other
    // windows need not match (the native margin counts bytes and rounds to lines).
    const pad = '012345678\n', repeat = 240;
    const template = [
        'left A\n{pad}cat here\n{pad}dog here\n{pad}right A\n',
        'left B\n{pad}cut here\n{pad}dig here\n{pad}right B\n',
    ];
    let [a, b] = template.map(s => text(s.replaceAll('{pad}', pad.repeat(repeat))));
    const steps = [
        {side: 'build', edits: []},
        {side: 'B', edits: [['cut here\n', 'more here\n'], ['dig here\n', 'dog here\n']]},
        {side: 'A', edits: [['cat here\n', 'mate here\n']]},
        {side: 'A', edits: [['mate here\n', 'more here\n']]},
        {side: 'A', edits: []},
    ];
    let latest;
    for (const step of steps) {
        const doc = step.side === 'A' ? a : b;
        const edits = ChangeSet.of(step.edits.map(([before, insert]) => {
            const [from, to] = position(doc.toString(), before);
            return {from, to, insert};
        }), doc.length);
        if (step.side === 'A') a = edits.apply(a);
        if (step.side === 'B') b = edits.apply(b);
        const sa = a.toString(), sb = b.toString();
        step.calls = [];
        const conf = config((windowA, windowB) => {
            const window = [...position(sa, windowA), ...position(sb, windowB)];
            const changes = diff(windowA, windowB, {scanLimit: 500});
            const raw = ranges(bytePositions(windowA), bytePositions(windowB), changes);
            step.calls.push({window, raw});
            return changes;
        });
        latest = step.side === 'build' ? Chunk.build(a, b, conf)
            : Chunk[`update${step.side}`](latest, a, b, edits.desc, conf);
        step.chunks = chunks(sa, sb, latest);
    }
    equal(steps.map(step => step.calls.length), [1, 2, 1, 1, 0]);
    log({name: 'aligned-update-windows', pad, repeat, template, steps});
}

function position(doc, slice) {
    const start = doc.indexOf(slice);
    check(start >= 0, `missing ${JSON.stringify(slice)}`);
    equal(start, doc.lastIndexOf(slice));
    return [start, start + slice.length];
}

function replace(doc, before, insert) {
    const [from, to] = position(doc.toString(), before);
    return {from, to, insert};
}

function recordDiff(calls, id, wide = () => false) {
    return config((a, b) => {
        calls.push([id, a, b]);
        // The pin has ambient precision. Reset it for a wide script.
        if (wide(a, b)) {
            diff('', '');
            return [new Change(0, a.length, 0, b.length)];
        }
        return diff(a, b);
    });
}

function capturePair() {
    const calls = [], steps = [];
    const doc = 'head\ncat\nfoot\n';
    const pair = new MergeView({
        a: {doc}, b: {doc, extensions: EditorState.changeFilter.of(tr => !tr.isUserEvent('reject'))},
        parent: document.body, diffConfig: recordDiff(calls, 'old', () => true),
    });
    function snapshot(op, previous) {
        const {a, b} = pair;
        const sa = a.state.doc.toString(), sb = b.state.doc.toString();
        const result = pair.chunks;
        for (const view of [a, b]) check(getChunks(view.state).chunks === result, 'chunks not shared');
        steps.push({op, a: sa, b: sb, head: b.state.selection.main.head, calls: calls.splice(0),
            chunks: chunks(sa, sb, result), ...(previous && {kept: previous === result})});
    }
    try {
        snapshot('build');
        let previous = pair.chunks;
        pair.reconfigure({diffConfig: recordDiff(calls, 'new', (a, b) => a === b)});
        snapshot('swap', previous);
        pair.b.dispatch({selection: {anchor: 1}});
        snapshot('select', previous);
        pair.b.dispatch({changes: replace(pair.b.state.doc, 'cat', 'cut'), userEvent: 'reject'});
        snapshot('filtered', previous);
        pair.b.dispatch({changes: replace(pair.b.state.doc, 'cat', 'cut')});
        snapshot('edit-b');
        pair.a.dispatch({changes: replace(pair.a.state.doc, 'cat', 'cut')});
        snapshot('edit-a');
        previous = pair.chunks;
        pair.reconfigure({diffConfig: undefined});
        snapshot('clear', previous);
        pair.a.dispatch({changes: replace(pair.a.state.doc, 'cut', 'cot')});
        snapshot('edit-native');
        equal(steps.map(step => step.calls.map(([id]) => id)), [['old'], [], [], [], ['new'], ['new'], [], []]);
        log({name: 'side-by-side-reconfigure', steps});
    } finally {
        pair.destroy();
    }
}

function captureUnified() {
    const calls = [], steps = [];
    const compartment = new Compartment();
    const old = unifiedMergeView({original: 'head\ncat\nfoot\n', diffConfig: recordDiff(calls, 'old'), gutter: false});
    let state = EditorState.create({doc: 'head\ncut\nfoot\n', extensions: compartment.of(old)});
    function snapshot(op, previous) {
        const a = getOriginalDoc(state).toString(), b = state.doc.toString();
        const result = getChunks(state).chunks;
        steps.push({op, a, b, head: state.selection.main.head, calls: calls.splice(0),
            chunks: chunks(a, b, result), ...(previous && {kept: previous === result})});
    }
    snapshot('build');
    const original = getOriginalDoc(state);
    const editA = ChangeSet.of(replace(original, 'cat', 'pet'), original.length);
    state = state.update({effects: originalDocChangeEffect(state, editA),
        changes: replace(state.doc, 'cut', 'pit'), selection: {anchor: 1}}).state;
    snapshot('edit-a-then-b');
    const previous = getChunks(state).chunks;
    state = state.update({effects: compartment.reconfigure(old)}).state;
    snapshot('same', previous);
    const next = unifiedMergeView({original: 'head\npot\nfoot\n', diffConfig: recordDiff(calls, 'new', () => true), gutter: false});
    state = state.update({effects: compartment.reconfigure(next), changes: replace(state.doc, 'pit', 'pot')}).state;
    snapshot('new-and-edit');
    equal(steps.map(step => step.calls.map(([id]) => id)), [['old'], ['old', 'old'], [], ['new', 'new']]);
    log({name: 'unified-override-lifecycle', steps});
}

captureChunks();
capturePair();
captureUnified();
globalThis.nativeSupport = rows;
