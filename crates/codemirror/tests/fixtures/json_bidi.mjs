// Generate from the repo root with Node 26.10.0:
// node crates/codemirror/tests/fixtures/json_bidi.mjs <artifact-dir> > crates/codemirror/tests/fixtures/json_bidi.jsonl
// Use the manifest archives and their node_modules links. The view stub runs
// the unmodified upstream parser and plugin without a DOM.
import assert from 'node:assert/strict';
import {execFileSync} from 'node:child_process';
import {createHash} from 'node:crypto';
import {readFileSync, realpathSync} from 'node:fs';
import {findPackageJSON} from 'node:module';
import {join} from 'node:path';
import {pathToFileURL} from 'node:url';
import {projectedOffsets, projectInput} from './editor_line_offsets.mjs';

const root = realpathSync(process.argv[2]);
const {packages, referenceDependencies} = JSON.parse(readFileSync(
  new URL('../../../../docs/codemirror-upstream.json', import.meta.url),
));
const paths = new Map();
for (const name of [
  'codemirror', '@codemirror/lang-json', '@codemirror/language', '@codemirror/state',
  '@codemirror/view', '@codemirror/streamparser', '@codemirror/commands', '@codemirror/search',
  '@codemirror/autocomplete', '@codemirror/lint', '@lezer/json', '@lezer/common',
  '@lezer/lr', '@lezer/highlight',
]) {
  const pin = [...packages, ...referenceDependencies].find(pkg => pkg.name === name);
  const dir = join(root, name.replace('/', '__'));
  const archive = join(dir, pin.tarball.split('/').at(-1));
  const pkg = join(dir, 'package');
  assert.equal(`sha512-${createHash('sha512').update(readFileSync(archive)).digest('base64')}`, pin.integrity);
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
const {json, jsonLanguage} = await load('@codemirror/lang-json');
const {parser} = await load('@lezer/json');
const {basicSetup, minimalSetup} = await load('codemirror');
const {EditorState} = await load('@codemirror/state');
const {Direction: {LTR, RTL}, EditorView} = await load('@codemirror/view');
const {forceParsing, syntaxTree, bidiIsolates} = await load('@codemirror/language');
const {NodeProp} = await load('@lezer/common');
for (const pinnedParser of [parser, jsonLanguage.parser]) {
  const types = pinnedParser.nodeSet.types.flatMap(type => {
    const isolate = type.prop(NodeProp.isolate);
    return isolate === undefined ? [] : [[type.name, isolate]];
  });
  assert.deepEqual(types, [['String', 'auto'], ['PropertyName', 'auto']]);
}
assert.deepEqual(json().support, []);

function only(source, part) {
  const from = source.indexOf(part);
  assert(from >= 0);
  return [from, from + part.length];
}

const sample = '{"abc אב": "عرب xyz"}';
const ascii = '{"escapes": "\\u05d0", "data": ["abc", true, null, -1.5]}';
const mixed = '{\r\n  "אב": "abc",\r  "plainKey": "plainValue",\n  "xyz": "عرب 🦀",\r\n  "nonText": [true, null, -1.5]\n}';
const fragments = '["אב...גד", "ascii", "عرب"]';
const touch = '["אב","عرب"]';
const clipped = '["אב...גד", "ascii"]';
const comma = only(touch, ',');
const gap = '[\r\n"אב",\r\n"abcd",\r\n"גד"\r]';
const edited = '{\r\n  "אב": "one",\r  "emoji": "🦀",\n  "third": "عرب"\r\n}';
const cases = [
  ['stock-json', sample, {setup: 'stock'}],
  ['basic-setup', sample, {setup: 'basic'}],
  ['minimal-setup', sample, {setup: 'minimal'}],
  ['explicit', sample],
  ['ascii-default', ascii],
  ['ascii-always', ascii, {always: true}],
  ['ascii-rtl', ascii, {direction: RTL}],
  ['ascii-per-line', ascii, {perLine: true}],
  ['mixed-breaks', mixed],
  ['empty-viewport', sample, {always: true, visible: []}],
  ['fragmented', fragments, {always: true, visible: [only(fragments, 'אב'), only(fragments, 'عرب')]}],
  ['duplicate-touch', fragments, {always: true, visible: [only(fragments, 'אב'), only(fragments, 'גד')]}],
  ['touch-both-ends', touch, {always: true, visible: [comma]}],
  ['touch-start-point', touch, {always: true, visible: [[comma[1], comma[1]]]}],
  ['rtl-past-viewport-end', touch, {visible: [[0, 2]]}],
  ['rtl-before-viewport-start', clipped, {visible: [only(clipped, 'ascii')]}],
  ['short-crlf-gap', gap],
  ['long-crlf-gap', gap.replace('abcd', 'abcde')],
  ['skipped-string', '["אב" "גד", "عرب"]'],
  ['edit-and-repair', edited, {always: true}, [
    ['remove separator', '"one",', '"one"'],
    ['restore separator', '"one"', '"one",'],
    ['remove property name', '"emoji"', ''],
    ['restore property name', '  :', '  "emoji":'],
    ['remove end quote', 'عرب"', 'عرب'],
    ['restore end quote', 'عرب', 'عرب"'],
    ['insert before first key', '"אב"', '"extra": "😀", "אב"'],
    ['remove extra property', '"extra": "😀", ', ''],
  ]],
];

for (const [id, initial, options = {}, edits = []] of cases) {
  const {setup = 'explicit', always = false, direction = LTR, perLine = false} = options;
  const isolation = bidiIsolates({alwaysIsolate: always});
  const [plugin] = isolation;
  const extension = {stock: [], basic: basicSetup, minimal: minimalSetup, explicit: isolation}[setup];
  const view = {
    state: EditorState.create({doc: initial, extensions: [json(), extension, EditorView.perLineTextDirection.of(perLine)]}),
    textDirection: direction,
    plugin: key => key === plugin ? value : null,
    dispatch(spec) { this.state = this.state.update(spec).state; },
  };
  let value;
  function snapshot(source) {
    const {units, byteAt} = projectedOffsets(source);
    assert.equal(view.state.doc.toString(), source.replace(/\r\n?/g, '\n'));
    const visible = options.visible ?? [[0, units]];
    view.visibleRanges = visible.map(([from, to]) => ({from, to}));
    assert(forceParsing(view, units, 5000));
    const sources = view.state.facet(EditorView.bidiIsolatedRanges);
    const ranges = [];
    if (setup === 'explicit') {
      assert.equal(sources.length, 1);
      value = plugin.create(view);
      const marks = sources[0](view);
      assert.equal(marks, view.state.facet(EditorView.outerDecorations)[0](view));
      for (let mark = marks.iter(); mark.value; mark.next()) {
        const {class: name, attributes, inclusive, bidiIsolate} = mark.value.spec;
        assert.deepEqual([name, attributes.dir, inclusive, bidiIsolate], ['cm-iso', 'auto', true, null]);
        ranges.push([byteAt(mark.from), byteAt(mark.to)]);
      }
    } else {
      assert.equal(sources.length, 0);
    }
    const syntax = [];
    let recovered = false;
    syntaxTree(view.state).iterate({enter(node) {
      if (node.type.isError) recovered = true;
      if (node.type.prop(NodeProp.isolate)) syntax.push([node.name, byteAt(node.from), byteAt(node.to)]);
    }});
    return {source, visible: visible.map(range => range.map(byteAt)), syntax, ranges, recovered};
  }

  let source = initial;
  const steps = [snapshot(source)];
  for (const [name, removed, insert] of edits) {
    const [from, to] = only(view.state.doc.toString(), removed);
    const {byteAt} = projectedOffsets(source);
    const change = [byteAt(from), byteAt(to), insert];
    const transaction = view.state.update({changes: {from, to, insert}});
    source = projectInput(source, transaction);
    view.state = transaction.state;
    steps.push({name, change, ...snapshot(source)});
  }
  assert.equal(source, initial);
  if (edits.length > 0) {
    const {name, change, ...restored} = steps.at(-1);
    assert.deepEqual(restored, steps[0]);
  }
  console.log(JSON.stringify({id, setup, always, direction, perLine, steps}));
}
