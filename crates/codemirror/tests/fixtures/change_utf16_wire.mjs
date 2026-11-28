// node crates/codemirror/tests/fixtures/change_utf16_wire.mjs <archive-root>
import assert from 'node:assert/strict';
import {mkdtempSync, realpathSync, rmSync} from 'node:fs';
import {tmpdir} from 'node:os';
import {join, resolve} from 'node:path';
import {pinnedModules} from './pinned_modules.mjs';

const root = realpathSync(mkdtempSync(join(tmpdir(), 'change-utf16-wire-')));
try {
    const {load} = pinnedModules(resolve(process.argv[2]), root);
    const {ChangeSet, ChangeDesc, EditorSelection, MapMode, Text} = await load('@codemirror/state');
    const texts = [
        ['empty', '', []],
        ['empty replacement marker', '', [[0]]],
        ['empty line replacement marker', '', [[0, '']]],
        ['marker before deletion', 'é😀尾', [1, [0], [2], 1]],
        ['split unchanged', 'é😀尾\r\nz', [1, 2, 3, 1]],
        ['insert into empty', '', [[0, '🍵', 'è']]],
        ['replace one supplementary scalar', 'é😀尾', [1, [2, '🦀極'], 1]],
        ['split before CRLF', 'a\r\n😀b', [1, 1, [1, '\r', '', 'ß'], 2, 1]],
        ['mixed CR and LF', 'ẞ\rβ\r\n🦀\nJ', [1, [2, '🍵'], 2, 2, [1], 1]],
        ['adjacent split deletions', '猫🐟🐸é', [1, [2], [2], 1]],
        ['adjacent insertions', '🦀é', [2, [0, '🍵'], [0, '', 'β'], 1]],
        ['same text replacement geometry', 'ß🐟A', [[1, 'ß'], [2, '🐟'], 1]],
        ['delete all', 'é😀\r\n', [[5]]],
    ];
    const boundaries = string => {
        const result = new Map([[0, 0]]);
        let units = 0, bytes = 0;
        for (const ch of string) {
            units += ch.length;
            bytes += Buffer.byteLength(ch);
            result.set(units, bytes);
        }
        return result;
    };
    for (const [name, source, input] of texts) {
        const before = Text.of(source.split('\n'));
        const set = ChangeSet.fromJSON(input);
        const after = set.apply(before);
        const result = after.toString();
        const [a, b] = [boundaries(source), boundaries(result)];
        const wire = set.toJSON(), desc = set.desc.toJSON();
        assert.deepEqual(ChangeDesc.fromJSON(desc).toJSON(), desc);
        const inverse = set.invert(before);
        assert.equal(inverse.apply(after).toString(), source);
        const geometry = [];
        set.iterChanges((fromA, toA, fromB, toB) => {
            geometry.push([a.get(fromA), a.get(toA), b.get(fromB), b.get(toB)]);
        }, true);
        assert(!geometry.flat().includes(undefined), name);
        const positions = [];
        for (const [units, bytes] of a) {
            for (const side of [-1, 1]) {
                for (const [mode, name] of [[MapMode.Simple, 'simple'], [MapMode.TrackDel, 'delete'],
                    [MapMode.TrackBefore, 'before'], [MapMode.TrackAfter, 'after']]) {
                    const mapped = set.desc.mapPos(units, side, mode);
                    assert(mapped == null || b.has(mapped));
                    positions.push([bytes, side, name, mapped == null ? null : b.get(mapped)]);
                }
            }
        }
        const offsets = [...a.keys()];
        const ranges = offsets.length >= 4
            ? [EditorSelection.cursor(offsets[1]), EditorSelection.range(offsets.at(-1), offsets[2])]
            : [EditorSelection.range(offsets.at(-1), 0)];
        const selected = EditorSelection.create(ranges, offsets.length % ranges.length).toJSON();
        assert.deepEqual(EditorSelection.fromJSON(selected).toJSON(), selected);
        const selectionBytes = {ranges: selected.ranges.map(({anchor, head}) =>
            ({anchor: a.get(anchor), head: a.get(head)})), main: selected.main};
        const row = {name, source, wire, desc, result, inverse: inverse.toJSON(),
            geometry, positions, selected, selectionBytes};
        if (JSON.stringify(input) !== JSON.stringify(wire)) row.input = input;
        if (name === 'marker before deletion') {
            const identityBefore = ChangeSet.empty(before.length);
            const identityAfter = ChangeSet.empty(after.length);
            const edit = ChangeSet.of({from: after.length, insert: '!'}, after.length);
            row.transforms = {
                map: set.map(identityBefore).toJSON(),
                mapDesc: set.mapDesc(identityBefore.desc).toJSON(),
                composeBefore: identityBefore.compose(set).toJSON(),
                composeAfter: set.compose(identityAfter).toJSON(),
                edit: edit.toJSON(),
                composeEdit: set.compose(edit).toJSON(),
            };
        }
        console.log(JSON.stringify(row));
    }
} finally {
    rmSync(root, {recursive: true, force: true});
}
