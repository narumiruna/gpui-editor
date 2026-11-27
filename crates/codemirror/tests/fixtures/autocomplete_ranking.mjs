// Run from the repository root with Node and the autocomplete npm tarball path.
// Redirect stdout to autocomplete_ranking.jsonl. The collation reference is en-US.
import { createHash } from 'node:crypto';
import { execFileSync } from 'node:child_process';
import { readFileSync } from 'node:fs';
import { runInNewContext } from 'node:vm';

const archive = process.argv[2];
const manifest = JSON.parse(readFileSync(new URL('../../../../docs/codemirror-upstream.json', import.meta.url)));
const pin = manifest.packages.find(pkg => pkg.name === '@codemirror/autocomplete');
const sri = 'sha512-' + createHash('sha512').update(readFileSync(archive)).digest('base64');
if (pin.version !== '6.20.4' || sri !== pin.integrity) throw new Error('Wrong autocomplete archive');
if (new Intl.Collator().resolvedOptions().locale !== 'en-US') throw new Error('Use an en-US locale');

const dist = execFileSync('tar', ['-xOzf', archive, 'package/dist/index.js'], { encoding: 'utf8' });
function extract(from, to) {
  const start = dist.indexOf(from), end = dist.indexOf(to, start);
  if (start < 0 || end < 0) throw new Error(`Missing ${from} or ${to}`);
  return dist.slice(start, end);
}
const compare = dist.match(/^ +compareCompletions: (.+),$/m)[1];
// Run the matcher, ranking and default comparator from the distribution. For
// well-formed Unicode strings, these code point helpers have the same behavior
// as the imports from @codemirror/state.
const { FuzzyMatcher, StrictMatcher, sortOptions, compareCompletions } = runInNewContext(`
  ${extract('class Option {', 'function cur(')}
  ${extract('class FuzzyMatcher {', 'const completionConfig')}
  ${extract('function score(option)', 'class CompletionDialog')}
  const compareCompletions = ${compare};
  ({ FuzzyMatcher, StrictMatcher, sortOptions, compareCompletions });
`, {
  codePointAt: (str, pos) => str.codePointAt(pos),
  codePointSize: code => code < 0x10000 ? 1 : 2,
  fromCodePoint: String.fromCodePoint,
  completionConfig: Symbol('completionConfig'),
});

function byteOffset(text, offset, atEnd) {
  // Expand a match endpoint inside a surrogate pair to cover the whole scalar.
  if (offset > 0 && offset < text.length && text.codePointAt(offset - 1) > 0xffff) {
    offset += atEnd ? 1 : -1;
  }
  return Buffer.byteLength(text.slice(0, offset));
}
function ranges(label, flat) {
  return Array.from({ length: flat.length / 2 }, (_, i) => [
    byteOffset(label, flat[2 * i], false), byteOffset(label, flat[2 * i + 1], true),
  ]);
}
function emit(row) {
  console.log(JSON.stringify(row));
}
function match(query, label) {
  function outcome(Matcher) {
    const result = new Matcher(query).match(label);
    return result && [result.score, ranges(label, result.matched)];
  }
  emit({ query, label, fuzzy: outcome(FuzzyMatcher), strict: outcome(StrictMatcher) });
}
for (const [query, label] of [
  ['', ''], ['', 'café🦀'], ['x', ''], ['s', 'String'], ['s', 'from_str'],
  ['abc', 'abc'], ['abc', 'abcDef'], ['abc', 'prefix_abc'], ['abc', 'aVeryBigCar'],
  ['fb', 'foo-bar'], ['fb', 'parse_foo_bar'], ['fb', 'foo-long-bar'], ['fb', 'frob'],
  ['ab', 'xAAb'], ['fqv', 'from_query_value'], ['srt', 'std::result::Result'],
  ['café', 'CAFÉTitle'], ['Café', 'Cafe\u0301Title'], ['ß', 'SS'], ['İ', 'item'],
  ['ΟΣ', 'οςx'], ['αβ', 'α-Β'], ['𐐨b', '𐐀-Bar'], ['𐐨', '𐐀Type'],
  ['🦀', '🦀Type'], ['A😀', 'a😀'], ['A😀', 'za😀'], ['😀A😀', '😀a😀'],
  ['A😀', 'x'.repeat(198) + 'a😀'], ['A😀', 'x'.repeat(199) + 'a😀'],
  ['😀B', 'x'.repeat(199) + '😀b'], ['😀b', 'x'.repeat(199) + '😀b'],
  ['AB', '😀'.repeat(100) + 'ab'], ['ab', '😀'.repeat(100) + 'ab'],
]) match(query, label);

function rank(name, doc, input, { strict = false, stable = false } = {}) {
  const defaultCompare = stable ? () => 0 : compareCompletions;
  const active = input.map((source, sourceIndex) => ({
    hasResult: () => true,
    from: source.from ?? 0,
    to: source.to ?? doc.length,
    result: {
      filter: source.filter,
      options: source.options.map((option, index) => ({ ...option, sourceIndex, index })),
    },
  }));
  const rows = sortOptions(active, {
    facet: () => ({ filterStrict: strict, compareCompletions: defaultCompare }),
    sliceDoc: (from, to) => doc.slice(from, to),
  });
  emit({
    name, doc, strict, stable,
    sources: input.map((source, index) => ({
      range: [byteOffset(doc, active[index].from, false), byteOffset(doc, active[index].to, true)],
      filter: source.filter !== false,
      options: source.options,
    })),
    rows: rows.map(({ completion, score, match: matched }) => [
      completion.sourceIndex, completion.index, score, ranges(completion.label, matched),
    ]),
  });
}
const labels = (...names) => ({ options: names.map(label => ({ label })) });
rank('initials depend on earlier candidates in the same result', 'fb', [
  labels('foo', 'parse_foo_bar', 'foo_bar', 'scan_foo_bar', 'x.foo-bar', 'scanFooBar', 'frob'),
  labels('read_foo_bar', 'fbExact', 'from-foo-bar'),
]);
rank('astral highlights and boost', '😀 A😀', [
  { from: 3, options: [{ label: 'za😀' }, { label: 'a😀', boost: 20 }, { label: 'A😀' }] },
  { from: 0, to: 2, ...labels('🦀', '😀Browser') },
]);
rank('empty fuzzy pattern and identifier collation', '', [
  labels('Zebra', 'eclair', 'Éclair', 'éclair', 'file10', 'file2', 'fileName', 'filename',
    'file.name', 'file-name', 'file_name', '$document', 'Array', 'array', 'a10', 'a2', ''),
  { options: [
    { label: 'fromUtf8', sortText: 'résumé' }, { label: 'toUtf8', sortText: 're\u0301sume\u0301' },
    { label: 'serialize', sortText: '' }, { label: 'two', sortText: 'a2' },
  ] },
]);
rank('empty strict pattern at a UTF-8 offset', 'é😀', [
  { from: 3, ...labels('beta', '', 'Alpha') },
  { from: 1, to: 1, filter: false, ...labels('pinned', '') },
], { strict: true });
rank('by-label ties and adjacent duplicate preferences', 'v', [
  { options: [
    { label: 'value', type: 'function' }, { label: 'value', info: 'documentation' },
    { label: 'value', type: 'keyword' }, { label: 'value', detail: 'alternate' },
    { label: 'value', detail: 'alternate', boost: 2 }, { label: 'value', apply: 'values()' },
    { label: 'value', apply: 'values()', info: 'insert documentation' },
    { label: 'valueOf', displayLabel: 'Get value' }, { label: 'valueOf', displayLabel: '' },
  ] },
]);
rank('adjacent duplicates retain the source and section of the selected candidate', 'é é', [
  { to: 1, ...labels('écho') },
  { from: 2, options: [
    { label: 'écho', section: { name: 'Definitions', rank: 'dynamic' }, info: 'definition' },
    { label: 'écrit', section: { name: 'Definitions', rank: 'dynamic' } },
  ] },
]);
rank('stable source ties and unfiltered scores', 'a', [
  labels('azure', 'alpha', 'alpine'),
  { filter: false, ...labels('zero', 'apple') },
  { options: [{ label: 'azure', info: 'not adjacent' }, { label: 'ace' }, { label: 'alpha' }] },
], { stable: true });
rank('mixed section ordering, UTF-16 names and first section metadata', 'fo', [
  { options: [
    { label: 'fo', section: { name: 'Zulu', rank: 'dynamic' } },
    { label: 'fold', section: 'Tools' },
    { label: 'font', section: { name: 'Other', rank: 'dynamic' } },
    { label: 'format', section: { name: 'Tools', rank: 1 } },
    { label: 'for', section: { name: 'First', rank: 5 } },
    { label: 'foo', section: { name: 'Last', rank: 1000000001 } },
    { label: 'forward', section: '😀' },
    { label: 'foobar', section: '\uE000' },
    { label: 'fooBar' },
  ] },
  { filter: false, options: [{ label: 'recent', section: { name: 'Cached', rank: 'dynamic' } }] },
]);
rank('a zero best-score is replaced by the next dynamic score', 'fo', [
  { options: [
    { label: 'fo', section: { name: 'Exact', rank: 'dynamic' } },
    { label: 'format', section: { name: 'Exact', rank: 'dynamic' } },
    { label: 'fold', section: { name: 'Fold', rank: 'dynamic' }, boost: 1 },
  ] },
]);
