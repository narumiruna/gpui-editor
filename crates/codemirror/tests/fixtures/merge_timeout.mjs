// node merge_timeout.mjs <archive-dir> > merge_timeout.jsonl
import assert from 'node:assert/strict';
import {mkdtempSync, realpathSync, rmSync} from 'node:fs';
import {tmpdir} from 'node:os';
import {join, resolve} from 'node:path';
import {pinnedModules} from './pinned_modules.mjs';

function capture(times, run) {
    const now = Date.now;
    let reads = 0;
    Date.now = () => times[Math.min(reads++, times.length - 1)];
    try {
        return [run(), reads];
    } finally {
        Date.now = now;
    }
}

function byteOffsets(text) {
    const offsets = new Map([[0, 0]]);
    let units = 0, bytes = 0;
    for (const ch of text) offsets.set(units += ch.length, bytes += Buffer.byteLength(ch));
    return offsets;
}

function ranges(a, b) {
    const [ma, mb] = [byteOffsets(a), byteOffsets(b)];
    const offset = (map, pos) => {
        assert(map.has(pos), `non-scalar endpoint: ${pos}`);
        return map.get(pos);
    };
    const range = (fromA, toA, fromB, toB) =>
        [offset(ma, fromA), offset(ma, toA), offset(mb, fromB), offset(mb, toB)];
    return {
        changes: list => list.map(c => range(c.fromA, c.toA, c.fromB, c.toB)),
        chunks: list => list.map(c => {
            // Clamp upstream's EOF sentinel. Native chunks stop at real EOF.
            const bounds = range(c.fromA, Math.min(a.length, c.toA), c.fromB, Math.min(b.length, c.toB));
            const rel = c.changes.map(change => range(
                c.fromA + change.fromA, c.fromA + change.toA, c.fromB + change.fromB, c.fromB + change.toB,
            ).map((value, i) => value - bounds[i < 2 ? 0 : 2]));
            return [bounds, c.precise, rel];
        }),
    };
}

const scratch = mkdtempSync(join(realpathSync(tmpdir()), 'gpui-merge-timeout.'));
try {
    const {load} = pinnedModules(resolve(process.argv[2]), scratch);
    const {diff, presentableDiff, Chunk} = await load('@codemirror/merge');
    const {ChangeSet, Text} = await load('@codemirror/state');
    const text = s => Text.of(s.split('\n'));
    const depth = c => c.repeat(80) + '-----' + c.repeat(80);
    const anchor = c => c.repeat(120) + '-'.repeat(80) + c.repeat(201);
    const scanLimit = 500;
    const emit = row => console.log(JSON.stringify({...row, scanLimit}));
    for (const [name, a, b, timeout, times] of [
        ['expired-entry', 'abcd', 'abdc', 5, [1000, 1006]],
        ['zero-clears-deadline', 'abcd', 'abdc', 0, [1000, 1006]],
        ['at-deadline', 'abcd', 'abdc', 5, [1000, 1005]],
        ['expired-depth-zero', 'abcd', 'abdc', 5, [1000, 1005, 1006]],
        ['trim-fast-path', 'α😀\r\nabc\r\nβ', 'α😀\r\naxc\r\nβ', 5, [1000, 1006]],
        ['expired-utf16', 'éa😀\r\n', 'é😀a\r\n', 5, [1000, 1006]],
        ['depth-before-64', 'a'.repeat(64), 'b'.repeat(64), 5, [1000, 1005, 1005, 1006]],
        ['depth-64', depth('a'), depth('b'), 5, [1000, 1005, 1005, 1006]],
        ['depth-control', depth('a'), depth('b'), 5, [1000, 1005]],
        ['coarse-anchor', anchor('a'), anchor('b'), 5, [1000, 1006]],
    ]) {
        const config = {scanLimit, timeout};
        const asBytes = ranges(a, b);
        const [raw, reads] = capture(times, () => diff(a, b, config));
        const [display, displayReads] = capture(times, () => presentableDiff(a, b, config));
        const [chunks, chunkReads] = capture(times, () => Chunk.build(text(a), text(b), config));
        assert.equal(displayReads, reads, name);
        assert.equal(chunkReads, reads, name);
        emit({name, a, b, timeout, times, reads, precise: chunks.every(c => c.precise),
            raw: asBytes.changes(raw), display: asBytes.changes(display), chunks: asBytes.chunks(chunks)});
    }

    const pad = '0123456789\n', repetitions = 240;
    const linesA = ['left A', 'abcd', 'abcd', 'right A'];
    const linesB = ['left B', 'abcd', 'abcd', 'right B'];
    const makeDoc = lines => lines.join('\n' + pad.repeat(repetitions));
    const [a, before] = [makeDoc(linesA), makeDoc(linesB)];
    const edits = [...before.matchAll(/abcd/g)].map(match => [match.index + 2, match.index + 4, 'dc']);
    const changes = ChangeSet.of(edits.map(([from, to, insert]) => ({from, to, insert})), before.length);
    const original = Chunk.build(text(a), text(before), {scanLimit});
    const b = changes.apply(text(before));
    const timeout = 5, times = [1000, 1006, 1010, 1015];
    const [chunks, reads] = capture(times, () =>
        Chunk.updateB(original, text(a), b, changes.desc, {scanLimit, timeout}));
    emit({name: 'update-windows', linesA, linesB, pad, repetitions, edits, timeout, times, reads,
        chunks: ranges(a, b.toString()).chunks(chunks)});
} finally {
    rmSync(scratch, {recursive: true, force: true});
}
