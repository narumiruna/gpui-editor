// node crates/codemirror/tests/fixtures/state_change_specs.mjs <archive-root>
import {mkdtempSync, realpathSync, rmSync} from 'node:fs';
import {tmpdir} from 'node:os';
import {join, resolve} from 'node:path';
import {pinnedModules} from './pinned_modules.mjs';

const root = realpathSync(mkdtempSync(join(tmpdir(), 'state-change-specs-')));
try {
    const {load} = pinnedModules(resolve(process.argv[2]), root);
    const {ChangeSet, EditorSelection, EditorState, MapMode, Text} = await load('@codemirror/state');
    const cases = [
        ['unordered disjoint ranges', 'abcdef', [[4, 5, 'Y'], [0, 1, 'X'], [2, 3, 'Z']]],
        ['overlapping replacements', 'abcdef', [[1, 4, 'X'], [2, 5, 'Y']]],
        ['reversed overlapping replacements', 'abcdef', [[2, 5, 'Y'], [1, 4, 'X']]],
        ['competing replacements and insertion', 'abcdef', [[1, 3, 'X'], [1, 3, 'Y'], [1, 1, 'Z']]],
        ['deleting across two older replacements', 'abcdefghi', [[2, 4, 'X'], [5, 7, 'Y'], [1, 8, '']]],
        ['inserting inside a replaced range', 'abcdef', [[0, 4, 'A'], [2, 2, 'B'], [2, 2, 'C']]],
        ['adjacent deletion with a no-op between', 'abcdef', [[1, 2, ''], [0, 0, ''], [2, 3, '']]],
        ['multiple insertions at a boundary', 'abcdef', [[1, 1, 'X'], [1, 1, 'Y'], [1, 3, 'Z'], [3, 3, 'Q']]],
        ['non-ASCII ranges and normalized insertions', '😀é\n猫x', [[2, 4, '界\r\n𐐀'], [0, 3, 'λ\rY'], [4, 4, '😀']]],
        ['empty batch', '😀', []],
    ];
    cases.push(
        ['nested specs keep group order', 'abcdef', null, [
            [{from: 2, to: 4, insert: 'X'}, [{from: 0, to: 3, insert: 'Y'}]],
            [], {from: 2, to: 2, insert: 'Z'},
        ]],
        ['prebuilt sets and literal text', 'abcdef', null, [
            {from: 1, to: 3, insert: 'a\rb'},
            [ChangeSet.of({from: 2, to: 4, insert: Text.of(['X\rY'])}, 6)],
            {from: 5, insert: Text.of(['M\rN'])}, {from: 0, to: 2, insert: 'P'},
        ]],
    );
    const byte = (text, at) => Buffer.byteLength(text.slice(0, at));
    for (const [name, source, specs, mixed] of cases) {
        const state = EditorState.create({doc: source});
        const changes = state.changes(mixed ?? specs.map(([from, to, insert]) => ({from, to, insert})));
        const doc = changes.apply(state.doc).toString();
        const edit = [];
        changes.iterChanges((from, to, _fromB, _toB, insert) => {
            edit.push([byte(source, from), byte(source, to), insert.toString()]);
        }, true);
        const map = [];
        let at = 0;
        for (const ch of [...source, '']) {
            const mapped = [-1, 1].map(assoc => [MapMode.Simple, MapMode.TrackDel, MapMode.TrackBefore, MapMode.TrackAfter]
                .map(mode => {
                    const pos = changes.mapPos(at, assoc, mode);
                    return pos == null ? null : byte(doc, pos);
                }));
            map.push([byte(source, at), mapped]);
            at += ch.length;
        }
        console.log(JSON.stringify({name, source, specs: specs?.map(([from, to, insert]) => [byte(source, from), byte(source, to), insert]) ?? null, doc, edit, map}));
    }
    for (const [name, source, separator, insert, ranges] of [
        ['replacement parses default line breaks', 'a\r\n猫😀\r\nz', '', 'é\r\nQ\rZ', [[0, 1], [5, 2]]],
        ['replacement parses only the configured separator', '猫::abc::😀X', '::', 'K::界\rZ', [[1, 2], [8, 8]]],
    ]) {
        const state = EditorState.create({doc: source, selection: EditorSelection.create(ranges.map(pair => EditorSelection.range(...pair)), 1),
            extensions: [EditorState.allowMultipleSelections.of(true), EditorState.lineSeparator.of(separator)]});
        const inputs = ranges.map(pair => pair.map(pos => byte(state.doc.toString(), pos)));
        const next = state.update(state.replaceSelection(insert)).state;
        const doc = next.doc.toString();
        console.log(JSON.stringify({name, source, separator, insert, inputs, doc, output: next.selection.ranges.map(range => [byte(doc, range.head), range.assoc]), main: next.selection.mainIndex}));
    }
} finally {
    rmSync(root, {recursive: true, force: true});
}
