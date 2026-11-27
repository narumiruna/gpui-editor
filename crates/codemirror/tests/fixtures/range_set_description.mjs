// node crates/codemirror/tests/fixtures/range_set_description.mjs <archive-root>
import {mkdtempSync, realpathSync, rmSync} from 'node:fs';
import {tmpdir} from 'node:os';
import {join, resolve} from 'node:path';
import {pinnedModules} from './pinned_modules.mjs';

const temp = realpathSync(mkdtempSync(join(tmpdir(), 'range-set-description-')));
try {
    const {ChangeDesc, MapMode, RangeSet, RangeValue} = await pinnedModules(resolve(process.argv[2]), temp)
        .load('@codemirror/state');
    class Value extends RangeValue {
        constructor(name, startSide, endSide, point, mapMode) {
            super();
            Object.assign(this, {name, startSide, endSide, point, mapMode: MapMode[mapMode]});
        }
        eq(other) { return this.name === other.name; }
    }
    // Geometry uses ASCII positions. The changed sections carry lengths only.
    const cases = [
        ['split insertions', [3, -1, 0, 2, 0, 3, 5, -1], [
            [0, 3, 'left', 1, -1, false, 'Simple'],
            [3, 3, 'before', -2, -2, true, 'TrackBefore'],
            [3, 3, 'straddle', -1, 1, true, 'TrackDel'],
            [3, 3, 'after', 1, 1, true, 'TrackAfter'],
            [3, 7, 'right', 1, -1, false, 'Simple'],
        ]],
        ['deletion modes', [3, -1, 4, 0, 3, -1], [
            [0, 9, 'whole', 1, -1, false, 'Simple'],
            [3, 3, 'kept start', 1, 1, true, 'TrackBefore'],
            [3, 3, 'deleted start', 1, 1, true, 'TrackAfter'],
            [5, 5, 'mapped center', 1, 1, true, 'Simple'],
            [5, 5, 'deleted center', 1, 1, true, 'TrackDel'],
            [7, 7, 'deleted end', 1, 1, true, 'TrackBefore'],
            [7, 7, 'kept end', 1, 1, true, 'TrackAfter'],
            [8, 10, 'shifted', 1, -1, false, 'Simple'],
        ]],
        ['strict coverage drops a chunk', [1, -1, 8, 2, 3, -1], [
            [2, 3, 'covered range', -1, 1, false, 'Simple'],
            [8, 8, 'covered point', 1, 1, true, 'Simple'],
        ]],
        ['touch keeps boundary mapping', [1, -1, 8, 2, 3, -1], [
            [1, 4, 'touching range', -1, 1, false, 'Simple'],
            [8, 8, 'touching point', 1, 1, true, 'Simple'],
        ]],
        ['shift an untouched layered set', [1, -1, 1, 4, 3, -1, 5, -1], [
            [3, 8, 'outer', 1, -1, false, 'Simple'],
            [4, 6, 'replace', -1, 1, true, 'TrackDel'],
            [10, 10, 'old eof', -1, -1, true, 'TrackBefore'],
        ]],
        ['split unchanged sections', [4, -1, 6, -1], [
            [1, 9, 'outer', 1, -1, false, 'Simple'],
            [4, 4, 'seam', 1, 1, true, 'TrackDel'],
        ]],
        ['insertion into empty document', [0, 2], [
            [0, 0, 'before', -1, -1, true, 'TrackBefore'],
            [0, 0, 'after', 1, 1, true, 'TrackAfter'],
        ]],
    ];
    for (const [name, sections, ranges] of cases) {
        const before = RangeSet.of(ranges.map(([from, to, ...value]) => new Value(...value).range(from, to)), true);
        const after = before.map(ChangeDesc.fromJSON(sections));
        const mapped = [];
        for (let cur = after.iter(); cur.value; cur.next()) mapped.push([cur.from, cur.to, cur.value.name]);
        console.log(JSON.stringify({name, sections, ranges, mapped}));
    }
} finally {
    rmSync(temp, {recursive: true, force: true});
}
