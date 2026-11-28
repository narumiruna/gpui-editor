// Emit only the folded composite set rows for stream_parser_composite_string_sets.jsonl.
// Use Node 26.10.0. Verify the @codemirror/streamparser 6.0.0 tarball against
// docs/codemirror-upstream.json before extracting it. Pass package/dist/index.js.
import assert from 'node:assert/strict';
import { pathToFileURL } from 'node:url';

assert.equal(process.version, 'v26.10.0');
assert.equal(process.versions.v8, '14.6.202.34-node.34');
const { StringStream } = await import(pathToFileURL(process.argv[2]).href);
const R = String.raw;
const units = text => Array.from({length: text.length}, (_, index) => text.charCodeAt(index));
const capture = text => text === undefined ? null : units(text);
const entries = object => Object.entries(object ?? {});
const intersection = R`[[\q{a|ab|}]&&[\q{A|AB|}]]`;

function add(source, texts, {flags = 'div', pos = 0, consume = true, lastIndex = 0} = {}) {
  for (const text of texts) {
    const row = {source: source.isWellFormed() ? source : units(source), flags, text: units(text), pos, consume, lastIndex};
    let regex;
    try {
      regex = new RegExp(source, flags);
    } catch (error) {
      if (!(error instanceof SyntaxError)) throw error;
      console.log(JSON.stringify({...row, invalid: true}));
      continue;
    }
    regex.lastIndex = lastIndex;
    const stream = new StringStream(text, 4, 2);
    stream.pos = pos;
    const match = stream.match(regex, consume);
    const result = match === null ? null : regex.global ? match.map(units) : {
      captures: Array.from(match, capture),
      index: match.index,
      input: units(match.input),
      names: entries(match.groups).map(([name, value]) => [name, capture(value)]),
      indices: Array.from(match.indices, value => value ?? null),
      namedIndices: entries(match.indices.groups).map(([name, value]) => [name, value ?? null]),
    };
    console.log(JSON.stringify({...row, result, end: stream.pos, lastAfter: regex.lastIndex}));
  }
}

add(`^${intersection}$`, ['', 'a', 'A', 'ab', 'aB', 'Ab', 'AB', 'aBB']);
add(R`^[\q{A}a]$`, ['A', 'a', 'B']);
add(R`^[\x41--\q{A}]$`, ['A', 'a', '']);
add(R`^[\q{A}--a]$`, ['A', 'a', '']);
add(R`^[\q{A}&&A]$`, ['a', 'A']);
add(R`^[\q{a}&&\q{A}]$`, ['a', 'A']);
add(R`^[\q{\&}&&\&]$`, ['&', 'a']);
add(R`^[Σ[\q{A|Ab|}]]$`, ['Σ', 'σ', 'ς', 'aB', 'a', 'A', '']);
add(R`^[[\q{A|Ab|}]--[\q{aB}]--[\q{A}]]$`, ['', 'a', 'A', 'AB', 'ab']);
add(R`^[\u{3a3}--\q{ς}]$`, ['Σ', 'ς', 'σ']);
add(R`^[K--\q{K}]$`, ['K', 'K', 'k']);
add(R`^[\q{K|Ke|}&&\q{K|kE|}]$`, ['K', 'k', 'KE', 'KE', '']);
add(R`^[\q{ſA}&&\q{Sa}]$`, ['ſA', 'sA', 'Sa', 'A']);
add(R`^[\q{ßa|İa|ıa}&&\q{ẞA|İA|ıA}]$`, ['ẞA', 'ssa', 'İa', 'ia', 'ıA', 'IA']);
add(R`^[𐐀--\q{𐐨}]$`, ['𐐀', '𐐨']);
add(R`^[\q{𐐀|\uD801\uDC00a}&&\q{\u{10428}|𐐨A}]$`, ['𐐀', '𐐨', '𐐀A', '𐐨a', '𐐨b']);
add(R`^[\q{\u{D801}\u{DC00}}--\q{𐐀}]$`, ['𐐀', '𐐨', '\uD801']);
add(R`^[\q{\u{D801}\u{DC00}}&&\q{𐐀}]$`, ['𐐀', '𐐨']);
add(R`^([[\q{𐐨b|\u{D801}\u{DC00}|}]])`, ['𐐀B', '𐐨b']);
add(R`^[\q{` + '\uD801' + R`|AA}&&\q{\u{D801}|aa}]$`, ['\uD801', 'Aa', '𐐀']);
add(`^(?<head>${intersection})(?<tail>B)$`, ['ab', 'AB', 'aBB', 'B']);
add(`^..(?<=(?<head>${intersection}))$`, ['AB', 'aa', 'BB']);
add(R`^(?<head>[a--\q{A}])\k<head>$`, ['Aa', 'AA', 'aa']);
add(R`^(?<head>[a--\q{A}])(?-i:\k<head>)$`, ['Aa', 'AA']);
add(`^(${intersection})(?<x>x)\\2\\k<x>$`, ['ABxXX', 'xXx', 'AXXX']);
add(R`^(?i:[a--\q{A}](?-i:[a--\q{A}]))z$`, ['Aaz', 'AaZ', 'aaz'], {flags: 'dv'});
add(R`^(?-i:[a--\q{A}](?i:[a--\q{A}]))z$`, ['aAz', 'aAZ'], {flags: 'div'});
add(R`^(?<x>[\q{𐐀|𐐀a}&&\q{𐐨|𐐨A}])(?<tail>b)?$`, ['xx𐐨AB', 'xx𐐀A'], {pos: 2});
add(R`^(?<x>[\q{𐐀|𐐀a}&&\q{𐐨|𐐨A}])(?<tail>b)?$`, ['xx𐐨AB'], {pos: 2, consume: false});
add(intersection, ['ABa𐐀a', '𐐀'], {flags: 'dgiv', lastIndex: 9});
add(intersection, ['xABa', 'xA'], {flags: 'diyv', pos: 1});
add(R`[a--\q{A}]`, ['xxAaz', 'xxaAz'], {flags: 'diyv', pos: 1, lastIndex: 1, consume: false});
for (const source of [
  R`[\q{A}&&&]`,
  R`[[\q{a|ab|}]&&&]`,
  R`[[\q{a|}]--]`,
  R`[[\q{a|}]&&[a]--[b]]`,
  R`[[\q{a|}]a&&[a]]`,
  R`[^\q{a|ab|}]`,
  R`[[\q{a|}]&&[^\q{x|}]]`,
  R`[[\q{\u{+41}|}]]`,
]) add(source, ['a']);
add(R`[[\q{a|ab|}]&&&]`, ['a'], {flags: 'dv'});

add(R`^[[A-C]--\q{A}]$`, ['A', 'a', 'B', 'b', 'C', 'c', 'D', '']);
add(R`^[\q{K|S|Q|XY}&&[K-S]]$`, ['K', 'k', 'K', 'S', 's', 'ſ', 'Q', 'q', 'Xy', '']);
add(R`^[[K-S]--\q{K|ſ}]$`, ['K', 'k', 'K', 'S', 's', 'ſ']);
add(R`^[[Σ-Τ]--\q{ς}]$`, ['Σ', 'σ', 'ς', 'Τ', 'τ', 'Υ']);
add(R`^[[Z-a]--\q{Z}]$`, ['Z', 'z', '[', '`', 'A', 'a', 'Y', 'b']);
add(R`^(?<token>[A-C\x31-\x33\q{Ke|X|}])$`, ['', 'A', 'a', 'D', '1', '3', '4', 'X', 'x', 'KE', 'ke']);
add(R`^[[𐐀-𐐂]--\q{\u{10428}}]$`, ['𐐀', '𐐨', '𐐁', '𐐩', '𐐂', '𐐪', '𐐃']);
add(R`^(?<token>[\q{𐐀|𐐁|𐐀a}&&[\uD801\uDC00-\u{10401}]])$`, ['xx𐐨', 'xx𐐀', 'xx𐐩', 'xx𐐨A', 'xx𐐪'], {flags: 'diyv', pos: 2, consume: false});
add(R`^[[K-K]--\q{K}]$`, ['K', 'k', 'K']);
add(R`^[\q{\-|\/|0|AA|}&&[\--0]]$`, ['a', '-', '0', '.', '/', 'Aa', '']);
add(R`^[[\uD7FF-\uD801]--\q{\uD800}]$`, ['\uD7FF', '\uD800', '\uD801', '𐐀']);
add(R`^[[A-C]--\q{A}]$`, ['A', 'a', 'B', 'b', 'C', 'c'], {flags: 'dv'});
add(R`^[A-Z]$`, ['K', 'k', 'K', 'S', 's', 'ſ'], {flags: 'di'});
for (const source of [
  R`[[Z-A]--\q{A}]`,
  R`[[a-A]--\q{A}]`,
  R`[A-C&&\q{A}]`,
  R`[\q{A}&&A-C]`,
  R`[A-C--\q{A}]`,
  R`[\q{A}--A-C]`,
  R`[[A-C]&&\q{A}--[B-C]]`,
  R`[A-\q{AA}]`,
  R`[A-\d\q{A}]`,
  R`[\q{A}-C]`,
  R`[A-B-C\q{A}]`,
  R`[[A-C]&&&\q{A}]`,
]) add(source, ['a']);
