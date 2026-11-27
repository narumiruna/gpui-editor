// Run with the verified archive directory path followed by `brackets` or
// `quotes` to generate the corresponding JSONL on stdout.
import assert from 'node:assert/strict';
import { createHash } from 'node:crypto';
import { readFileSync, realpathSync } from 'node:fs';
import { basename, join, resolve } from 'node:path';
import { pathToFileURL } from 'node:url';

const root = resolve(process.argv[2]);
const manifest = JSON.parse(readFileSync(new URL('../../../../../docs/codemirror-upstream.json', import.meta.url)));
const pins = [...manifest.packages, ...manifest.referenceDependencies];
for (const name of ['@codemirror/lang-lezer', '@lezer/lezer', '@codemirror/language', '@codemirror/autocomplete', '@codemirror/state']) {
  const pin = pins.find(pin => pin.name === name);
  const dir = join(root, name.replace('/', '__'));
  const tarball = readFileSync(join(dir, basename(pin.tarball)));
  assert.equal(`sha512-${createHash('sha512').update(tarball).digest('base64')}`, pin.integrity);
  assert.equal(realpathSync(join(root, 'node_modules', name)), realpathSync(join(dir, 'package')));
  const pkg = JSON.parse(readFileSync(join(dir, 'package/package.json')));
  assert.equal(pkg.name, name);
  assert.equal(pkg.version, pin.version);
}
const load = name => import(pathToFileURL(join(root, 'node_modules', name, 'dist/index.js')));
const { EditorState } = await load('@codemirror/state');
const { lezer } = await load('@codemirror/lang-lezer');
const { forceParsing, matchBrackets } = await load('@codemirror/language');
const { closeBrackets, insertBracket } = await load('@codemirror/autocomplete');
const extensions = [lezer(), closeBrackets()];

function create(doc, anchor = 0, head = anchor) {
  const view = {
    state: EditorState.create({doc, selection: {anchor, head}, extensions}),
    dispatch(spec) { this.state = this.state.update(spec).state; },
  };
  assert.ok(forceParsing(view, view.state.doc.length, 5000));
  return view.state;
}

// EditorState normalizes CR and CRLF to LF and measures positions in UTF-16.
// Convert carets and hit ranges to byte offsets in the original fence body.
function byteOffsets(source) {
  const offsets = new Map([[0, 0]]);
  let units = 0, bytes = 0;
  for (const chunk of source.matchAll(/\r\n|[^]/gu)) {
    units += chunk[0] === '\r\n' ? 1 : chunk[0].length;
    bytes += Buffer.byteLength(chunk[0]);
    offsets.set(units, bytes);
  }
  return offsets;
}

function near(state, head, offsets) {
  // The evaluation order in the pinned bracketDeco helper.
  const result = matchBrackets(state, head, -1) ||
    (head > 0 && matchBrackets(state, head - 1, 1)) ||
    matchBrackets(state, head, 1) ||
    (head < state.doc.length && matchBrackets(state, head + 1, -1));
  const range = ({from, to}) => [offsets.get(from), offsets.get(to)];
  return result ? [range(result.start), result.end ? range(result.end) : null, result.matched] : null;
}

function bracketCases() {
  for (const [name, source] of [
    ['detectDelim, props, nested rules and parenthesized expressions',
      '@detectDelim\r\n@top Root[p="["] { Inner { ("}") } $[()] !prec }'],
    ['typed punctuation without detectDelim, angle brackets are not matched',
      'Root<p> { Call<(("{" | "}"))> }'],
    ['fallback in literal, line comment, block comment and charsets',
      'Root { "😀 (]" // [)\r\n /* {] */ $[()] ![{}] }'],
    ['fallback can cross distinct tokens of the same type',
      'Rule { "[" \']\' // (\r// )\r /* { */ /* } */ }'],
    ['fallback does not cross types', 'Rule { "[" // ]\n $[)] ![(] }'],
    ['unclosed body does not fall back to literal closer', 'Root { ("}")'],
    ['unclosed paren preserves the structural brace pair', 'Root { (A }'],
    ['malformed and recovered prop punctuation', 'Root[p="x" { (A) }'],
    ['opaque nested raw punctuation', 'Root { $[([)] ![{}] }'],
  ]) {
    const state = create(source);
    const offsets = byteOffsets(source);
    const checks = [...offsets].map(([head, at]) => [at, near(state, head, offsets)]);
    console.log(JSON.stringify({ name, source, checks }));
  }
  // Plain scans check the limit between chunks in normalized UTF-16, even
  // when the byte count is larger. A marked sibling pair has no such limit.
  for (const [count, matched] of [[4999, true], [5000, false]]) {
    const source = '// [' + '😀'.repeat(count) + '\r\n// ]';
    const offsets = byteOffsets(source), state = create(source);
    assert.equal(near(state, 3, offsets)?.[2], matched ? true : undefined);
    assert.equal(near(state, state.doc.length, offsets)[2], true);
  }
  const source = 'Root {\r\n' + ' '.repeat(15000) + '\r\n}';
  assert.equal(near(create(source), 5, byteOffsets(source))[2], true);
}

function snapshot(state) {
  const body = state.doc.toString();
  const offsets = byteOffsets(body);
  const {anchor, head, empty} = state.selection.main;
  return { body, selection: [offsets.get(anchor), offsets.get(head)], hit: empty ? near(state, head, offsets) : null };
}

function quoteCases() {
  for (const [name, marked, keys, reverse] of [
    ['new double quote pair and tracked skip', 'Root { |}', '""'],
    ['new single quote pair and tracked skip', 'Root { | }', "''"],
    ['before existing double literal opener', 'Root { |"😀)" }', '"'],
    ['before existing single literal opener', "Root { |'字]' }", "'"],
    ['inside double literal', 'Root { "😀  | }" }', '"'],
    ['inside single literal', "Root { '😀  | )' }", "'"],
    ['different quote inside a literal', 'Root { "😀  | }" }', "'"],
    ['untracked literal closer', 'Root { "😀|" }', '"'],
    ['after complete literal', 'Root { "字"| }', '"'],
    ['at end of unterminated literal', 'Root { "abc |', '"'],
    ['escaped delimiter in literal', 'Root { "abc \\" | }" }', '"'],
    ['line comment has no string context', 'Root { // |\n }', '"'],
    ['block comment has no string context', 'Root { /* | */ }', "'"],
    ['charset has no string context', 'Root { $[ | ] }', '"'],
    ['word next to quote uses ordinary insertion', 'Root { |name }', '"'],
    ['reverse selection stays selected with UTF-8 offsets', 'Root { "|字😀|" }', "'", true],
    ['fence body end', 'Root { } |', '"'],
  ]) {
    const [left, next, tail] = marked.split('|');
    const doc = left + next + (tail ?? '');
    const from = left.length, to = tail === undefined ? from : from + next.length;
    let state = create(doc, reverse ? to : from, reverse ? from : to);
    const before = snapshot(state), after = [];
    for (const key of keys) {
      state = (insertBracket(state, key) ?? state.update(state.replaceSelection(key))).state;
      after.push([key, snapshot(state)]);
    }
    console.log(JSON.stringify({ name, before, after }));
  }
}

if (process.argv[3] === 'brackets') bracketCases();
else if (process.argv[3] === 'quotes') quoteCases();
else throw new Error('Expected `brackets` or `quotes`');
