// Node 26.10.0, from the repo root:
// node crates/codemirror/tests/fixtures/json_edit_indent.mjs <artifact-dir> > crates/codemirror/tests/fixtures/json_edit_indent.jsonl
import assert from 'node:assert/strict';
import {execFileSync} from 'node:child_process';
import {createHash} from 'node:crypto';
import {readFileSync, realpathSync} from 'node:fs';
import {findPackageJSON} from 'node:module';
import {join, resolve} from 'node:path';
import {pathToFileURL} from 'node:url';
import {projectedOffsets, projectInput} from './editor_line_offsets.mjs';

const root = resolve(process.argv[2]);
const {packages, referenceDependencies} = JSON.parse(readFileSync(
  new URL('../../../../docs/codemirror-upstream.json', import.meta.url),
));
const paths = new Map();
for (const name of [
  '@codemirror/lang-json', '@codemirror/commands', '@codemirror/language',
  '@codemirror/state', '@codemirror/view', '@lezer/json', '@lezer/common',
  '@lezer/lr', '@lezer/highlight',
]) {
  const pin = [...packages, ...referenceDependencies].find(pkg => pkg.name === name);
  const dir = join(root, name.replace('/', '__'));
  const archive = join(dir, pin.tarball.split('/').at(-1));
  assert.equal(`sha512-${createHash('sha512').update(readFileSync(archive)).digest('base64')}`, pin.integrity);
  const pkg = join(dir, 'package');
  assert.equal(realpathSync(join(root, 'node_modules', name)), pkg);
  for (const file of ['package.json', 'dist/index.js']) {
    assert.deepEqual(readFileSync(join(pkg, file)), execFileSync('tar', ['-xOf', archive, `package/${file}`]));
  }
  const metadata = JSON.parse(readFileSync(join(pkg, 'package.json')));
  assert.equal(metadata.version, pin.version);
  assert.equal(metadata.exports.import ?? metadata.exports['.'].import, './dist/index.js');
  paths.set(name, {pkg, url: pathToFileURL(join(pkg, 'dist/index.js')), deps: Object.keys(metadata.dependencies ?? {})});
}
for (const {url, deps} of paths.values()) {
  for (const name of deps.filter(dep => paths.has(dep))) {
    assert.equal(realpathSync(findPackageJSON(name, url)), join(paths.get(name).pkg, 'package.json'));
  }
}
const load = name => import(paths.get(name).url);
const exports = await load('@codemirror/lang-json');
assert.deepEqual(Object.keys(exports), ['json', 'jsonLanguage', 'jsonParseLinter']);
const {json, jsonLanguage} = exports;
const {EditorState, Prec, Transaction} = await load('@codemirror/state');
const {forceParsing, getIndentation, IndentContext, indentService} = await load('@codemirror/language');
const {indentSelection, insertNewlineAndIndent} = await load('@codemirror/commands');
const support = json();
assert.equal(support.language, jsonLanguage);
assert.deepEqual(support.support, []);

function viewOf(state, observe = () => {}) {
  const view = {state, dispatch(spec) {
    const transaction = spec instanceof Transaction ? spec : view.state.update(spec);
    view.state = transaction.state;
    observe(transaction);
  }};
  return view;
}

function reindent(source, state) {
  let result = source;
  const view = viewOf(state, transaction => {
    if (transaction.docChanged) result = projectInput(source, transaction);
  });
  assert(forceParsing(view, state.doc.length, 5000));
  view.dispatch({selection: {anchor: 0, head: state.doc.length}});
  assert(indentSelection(view));
  return result;
}

function snapshot(source, state) {
  const view = viewOf(state);
  assert(forceParsing(view, state.doc.length, 5000));
  state = view.state;
  const logical = state.doc.toString();
  assert.equal(logical, source.replace(/\r\n?/g, '\n'));
  const offsets = projectedOffsets(source);
  const indents = [];
  let pos = 0;
  for (const scalar of ['', ...logical]) {
    pos += scalar.length;
    const lineStart = state.doc.lineAt(pos).from;
    const query = options => getIndentation(new IndentContext(state, options), pos);
    indents.push([offsets.byteAt(pos), query({}), query({simulateBreak: pos}),
      query({simulateBreak: pos, simulateDoubleBreak: true}),
      query({overrideIndentation: from => from === lineStart ? 7 : -1})]);
  }
  assert.equal(pos, state.doc.length);
  view.dispatch({selection: {anchor: pos}});
  assert(insertNewlineAndIndent(view));
  const result = view.state.doc.toString();
  const enter = result.slice(pos);
  assert.equal(result, logical + enter);
  assert.match(enter, /^\n *$/);
  assert.equal(view.state.selection.main.head, result.length);
  return {source, indents, enter, selection: reindent(source, state)};
}

for (const [name, initial, edits] of [
  ['top-level wrong closer', '[\r\n  "😀"\r\n]', [
    ['use wrong closer', ']', '}'],
    ['delete wrong closer', '}', ''],
    ['restore closer', '', ']'],
  ]],
  ['wrong closer in a nested array', '[\r\n  [\r    "😀"]\r\n]', [
    ['replace both closers with a wrong closer', ']\r\n]', ' }'],
    ['repair inner closer', '}', ']'],
    ['restore outer closer', '', '\r\n]'],
  ]],
  ['unfinished array in an object property', '{\r\n  "名": [\r  true\r\n  ]\r\n}', [
    ['remove both closers', '\r\n  ]\r\n}', ''],
    ['restore both closers', '', '\r\n  ]\r\n}'],
  ]],
  ['recovered object starts on a reindented line', '{\r\n  "a": 1\r\n}', [
    ['add a second object', '  "a": 1\r\n}', '+1\r\n123}{\r\n+1'],
  ]],
]) {
  let source = initial;
  const view = viewOf(EditorState.create({doc: source, extensions: support}));
  const fresh = () => snapshot(source, EditorState.create({doc: source, extensions: support}));
  const steps = [{name: 'strict', ...fresh()}];
  for (const [name, removed, insert] of edits) {
    assert(source.endsWith(removed));
    const prefix = source.slice(0, source.length - removed.length);
    const start = Buffer.byteLength(prefix), end = Buffer.byteLength(source);
    view.dispatch({changes: {from: projectedOffsets(prefix).units, to: view.state.doc.length, insert}});
    assert(forceParsing(view, view.state.doc.length, 5000));
    source = prefix + insert;
    const got = snapshot(source, view.state);
    assert.deepEqual(got, fresh());
    steps.push({name, change: [start, end, insert], ...got});
  }
  console.log(JSON.stringify({name, steps}));
}

for (const [name, source] of [
  ['skipped brace followed by an invalid string', '[\n  1,\r\n  "名",\n  []\n{"a\\q"123'],
  ['unfinished array after a lone CR', '[\n  1,\r123 "名",\n@  []\n]['],
  ['unfinished array after a blank line', '[\n \n,\r\n  "名",\n :\n]['],
  ['skipped bracket followed by a string', '"a\\q"@{false0@\n@}"a"true[\n123["a"[]'],
  ['skipped brace followed by an invalid token', '}[\ntrue{@:+1}"a\\q""a\\q"+1"a"'],
  ['recovered array uses original indentation without overrides', '[\n  ][\n0]'],
]) {
  const step = snapshot(source, EditorState.create({doc: source, extensions: support}));
  console.log(JSON.stringify({name, steps: [{name: 'initial', ...step}]}));
}

const source = '[\n][\n0]';
const state = EditorState.create({doc: source, extensions: support});
const secondLine = state.doc.line(2).from;
const atSecondLine = column => indentService.of((context, pos) => pos === secondLine ? column : undefined);
const withServices = EditorState.create({doc: source, extensions: [
  support, atSecondLine(3), Prec.high(atSecondLine(7)),
]});
const step = snapshot(source, state);
const serviceSelection = reindent(source, withServices);
assert.equal(serviceSelection, '[\n       ][\n         0]');
console.log(JSON.stringify({name: 'higher priority service feeds a later JSON line',
  steps: [{name: 'initial', ...step, serviceSelection}]}));
