// node crates/codemirror/tests/fixtures/range_set_value_identity.mjs <archive-root>
import {mkdtempSync, realpathSync, rmSync} from 'node:fs';
import {tmpdir} from 'node:os';
import {join, resolve} from 'node:path';
import {pinnedModules} from './pinned_modules.mjs';

const temp = realpathSync(mkdtempSync(join(tmpdir(), 'range-set-identity-')));
try {
    const {ChangeDesc, RangeSet, RangeValue} = await pinnedModules(resolve(process.argv[2]), temp)
        .load('@codemirror/state');
    class Value extends RangeValue {
        constructor(name, point = false) {
            super();
            this.name = name;
            this.point = point;
        }
    }
    const mark = new Value('wide');
    const widget = new Value('point', true);
    const of = value => RangeSet.of(value.range(0, 10));
    const from = of(mark);
    const point = of(widget);
    const edit = ChangeDesc.fromJSON([4, -1, 2, 2, 4, -1]);
    const plain = ChangeDesc.fromJSON([10, -1]);
    function diff(old, next, desc) {
        const events = [];
        RangeSet.compare([old], [next], desc, {
            compareRange(from, to, a, b) {
                events.push(['range', from, to, a.map(value => value.name), b.map(value => value.name)]);
            },
            comparePoint(from, to, a, b) {
                events.push(['point', from, to, a?.name ?? null, b?.name ?? null]);
            },
            boundChange(at) { events.push(['bound', at]); },
        });
        return events;
    }
    const mappedMark = from.map(edit);
    const mappedPoint = point.map(edit);
    const joined = RangeSet.join([from, RangeSet.empty]);
    const rebuilt = from.update({filter: () => true});
    const otherMark = of(new Value(mark.name));
    const otherPoint = of(new Value(widget.name, true));
    const copy = of(widget);
    const same = (a, b) => RangeSet.eq([a], [b], 0, 10);
    console.log(JSON.stringify({
        mappedMark: diff(from, mappedMark, edit),
        mappedPoint: diff(point, mappedPoint, edit),
        joinedAndMapped: diff(from, joined.map(edit), edit),
        rebuilt: diff(from, rebuilt, plain),
        distinctMark: diff(from, otherMark, edit),
        distinctPoint: diff(point, otherPoint, plain),
        equalMappedPoint: same(point, mappedPoint),
        equalSharedValue: same(point, copy),
        equalDistinctPoint: same(point, otherPoint),
    }));
} finally {
    rmSync(temp, {recursive: true, force: true});
}
