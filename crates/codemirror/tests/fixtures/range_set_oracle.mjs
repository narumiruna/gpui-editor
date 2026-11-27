// Run with Node, the path to state-6.7.6.tgz, and the path to its extracted dist/index.js.
// Redirect standard output to range_set_oracle.json.
import { createHash } from 'node:crypto';
import { readFileSync } from 'node:fs';
import { pathToFileURL } from 'node:url';

const [archivePath, modulePath] = process.argv.slice(2);
const integrity = 'sha512-kAz+AncRtKuIknedxT1bq4XwXv4UowhbkHU1myPrtVb/jZtImWuV5BXzv5vK6i3kYACsdiZiQKFQQ5Mq7elW8w==';
const moduleSha256 = 'cf4ffe7d177bf2c7c5da66c49be0811cc79a29e12cb658238876bbdcef12939c';
if (`sha512-${createHash('sha512').update(readFileSync(archivePath)).digest('base64')}` !== integrity)
  throw new Error('Expected @codemirror/state 6.7.6 archive');
if (createHash('sha256').update(readFileSync(modulePath)).digest('hex') !== moduleSha256)
  throw new Error('Expected @codemirror/state 6.7.6 dist/index.js');
const { ChangeSet, RangeSet, RangeValue } = await import(pathToFileURL(modulePath));

class Value extends RangeValue {
  constructor(name, startSide, endSide, point) {
    super();
    Object.assign(this, { name, startSide, endSide, point });
  }
  eq(other) { return this.name === other.name; }
}

// These positions model ASCII text, so UTF-16 units and UTF-8 bytes agree.
const mark = (from, to, name) => new Value(name, 1, -1, false).range(from, to);
const point = (from, to, name, start = 1, end = 1) => new Value(name, start, end, true).range(from, to);
const of = (...ranges) => RangeSet.of(ranges);
const names = values => Array.from(values, value => value.name);
const changes = (length, ...spec) => ChangeSet.of(spec, length);

function iter(sets, from) {
  const result = [];
  for (let cur = RangeSet.iter(sets, from); cur.value; cur.next())
    result.push([cur.rank, cur.from, cur.to, cur.value.name]);
  return result;
}

function spans(sets, from, to, minPointSize) {
  const events = [];
  const open = RangeSet.spans(sets, from, to, {
    span(a, b, active, start) { events.push(['span', a, b, names(active), start]); },
    point(a, b, value, active, start, rank) {
      events.push(['point', a, b, value.name, names(active), start, rank]);
    },
  }, minPointSize);
  return { events, open };
}

function diff(old, next, textDiff, minPointSize) {
  const result = [];
  RangeSet.compare(old, next, textDiff.desc, {
    compareRange(a, b, x, y) { result.push(['range', a, b, names(x), names(y)]); },
    comparePoint(a, b, x, y) { result.push(['point', a, b, x?.name ?? null, y?.name ?? null]); },
    boundChange(pos) { result.push(['bound', pos]); },
  }, minPointSize);
  return result;
}

const tied = of(...['first', 'second', 'third', 'fourth'].map(name => point(1, 3, name)));
const joined = RangeSet.join([tied, of(mark(0, 4, 'cover'))]);
const deeper = of(mark(0, 4, 'cover'), point(1, 3, 'deep'));
const emptyPoints = of(...['a', 'b', 'c', 'd'].map(name => point(1, 1, name, 1, 2)));
const low = of(mark(0, 12, 'wide'), mark(0, 12, 'equal'), point(2, 8, 'main', 0, 1),
               point(4, 8, 'shadowed'), point(8, 8, 'before', -1, -1), point(8, 8, 'after'));
const high = of(mark(1, 12, 'above'), point(2, 8, 'loser', 0, 1), point(6, 10, 'cross'));
const shared = of(mark(0, 10, 'shared'), point(11, 11, 'widget'));
const updated = shared.update({ add: [mark(12, 14, 'add')] });
const rebuilt = shared.update({ add: [mark(12, 14, 'add')], filterFrom: 11, filterTo: 11, filter: () => true });
const shift = changes(16, { from: 0, insert: '++' });
const later = of(mark(3, 10, 'later'), point(11, 11, 'widget'));
const tail = mark(13, 15, 'extra');
const full = of(...Array.from({ length: 250 }, (_, i) => point(i, i, 'same')));
const fullUpdated = full.update({ add: [point(500, 500, 'extra')] });
const clipped = of(mark(0, 12, 'keep'), point(2, 9, 'old', -1, 1), point(9, 9, 'edge'));
const replacement = changes(12, { from: 3, to: 6, insert: '+++++' }, { from: 9, to: 11, insert: '+' });
const clippedMapped = clipped.map(replacement);
const altered = clippedMapped.update({
  add: [point(8, 12, 'new', -1, 2)],
  filter: (from, to, value) => value.name !== 'old',
});
const extra = of(mark(2, 4, 'extra'));
const layered = of(mark(0, 10, 'top'), mark(1, 9, 'lower'));
const atomic = of(point(1, 5, 'atom'));
const across = of(mark(0, 7, 'across'));
const between = [];
low.between(2, 8, (from, to, value) => { between.push([from, to, value.name]); });

console.log(JSON.stringify({
  source: { version: '6.7.6', integrity, moduleSha256 },
  ties: {
    iter: iter([tied], 0),
    join: iter([joined], 2),
    spans: spans([RangeSet.empty, tied], 2, 3),
    diff: diff([], [tied], changes(4)),
    empty: spans([emptyPoints], 1, 1),
  },
  precedence: {
    iter: iter([low, RangeSet.empty, high], 3),
    between,
    spans: spans([low, RangeSet.empty, high], 0, 12),
    clipped: spans([low, RangeSet.empty, high], 3, 8),
    lowerLayer: spans([deeper], 0, 4, 0),
    skipSet: diff([], [deeper], changes(4), 0),
    joinedSet: diff([], [RangeSet.join([deeper, RangeSet.empty])], changes(4), 0),
  },
  reuse: {
    unchanged: diff([shared], [shared, extra], changes(16)),
    updated: diff([shared], [updated, extra], changes(16)),
    rebuilt: diff([shared], [rebuilt, extra], changes(16)),
    shifted: diff([later], [later.map(shift).update({ add: [tail] })], shift),
    lowerChunk: diff([layered], [layered, extra], changes(16)),
    atomicChunk: diff([atomic], [atomic, across], changes(16)),
    equal: RangeSet.eq([full], [fullUpdated], 0, 100),
    cloned: RangeSet.eq([full], [RangeSet.join([full])], 0, 100),
  },
  clipping: {
    left: spans([clipped], 0, 2),
    both: spans([clipped], 4, 7),
    right: spans([clipped], 9, 9),
    diff: diff([clipped], [altered], replacement),
  },
}, null, 2));
