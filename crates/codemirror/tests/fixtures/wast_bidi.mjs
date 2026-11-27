// Run with Node 26.10.0 and a directory of pinned archives and node_modules.
// The actual language plugin and EditorView methods use a measured-view stub.
import assert from 'node:assert/strict';
import {execFileSync} from 'node:child_process';
import {createHash} from 'node:crypto';
import {readFileSync, realpathSync} from 'node:fs';
import {resolve} from 'node:path';
import {pathToFileURL} from 'node:url';

const root = resolve(process.argv[2]);
const {packages, referenceDependencies} = JSON.parse(readFileSync(new URL('../../../../docs/codemirror-upstream.json', import.meta.url)));
function pinned(name) {
  const {version, integrity} = [...packages, ...referenceDependencies].find(p => p.name === name);
  const artifact = resolve(root, name.replace('/', '__'), `${name.split('/')[1]}-${version}.tgz`);
  const file = realpathSync(resolve(root, 'node_modules', name, 'dist/index.js'));
  assert.equal('sha512-' + createHash('sha512').update(readFileSync(artifact)).digest('base64'), integrity);
  assert.deepEqual(execFileSync('tar', ['-xOf', artifact, 'package/dist/index.js']), readFileSync(file));
  return pathToFileURL(file).href;
}
const [wastURL, languageURL, stateURL, viewURL, commonURL] = [
  '@codemirror/lang-wast', '@codemirror/language', '@codemirror/state', '@codemirror/view',
  '@lezer/common', '@lezer/lr', '@lezer/highlight', '@codemirror/streamparser',
].map(pinned);
const {wast} = await import(wastURL);
const {bidiIsolates, forceParsing, syntaxTree} = await import(languageURL);
const {EditorState} = await import(stateURL);
const {Direction, EditorView} = await import(viewURL);
const {NodeProp} = await import(commonURL);

// Map normalized JS code-unit boundaries back to the native input bytes.
function inputPositions(source) {
  const positions = [0];
  let bytes = 0;
  for (const match of source.matchAll(/\r\n|[^]/gu)) {
    const text = match[0];
    bytes += Buffer.byteLength(text);
    if (text.length === 2 && text !== '\r\n') positions.push(null);
    positions.push(bytes);
  }
  return pos => {
    const at = positions[pos];
    assert(Number.isSafeInteger(at), 'expected an original byte boundary');
    return at;
  };
}

const mixed = '(module (data "ABC אב 🦀") ;; عرب\r\n  (;עברית\r  ASCII ;)\r  (export "漢字" (func $ascii))\n)';
const ascii = '(module (data "abc") ;; text\r  (; block ;)\r\n)';
const fragments = '(module (data "אב...גד")\n ;; عرب\n  (data "ده"))';
const gaps = ';; אב\r\n"abcde"\r\n;; גד';
const {LTR, RTL} = Direction;
const only = (text, value) => {
  const from = text.indexOf(value);
  assert(from >= 0);
  return [from, from + value.length];
};
const cases = [
  ['mixed-breaks', mixed],
  ['ascii-default', ascii],
  ['ascii-always', ascii, {always: true}],
  ['ascii-rtl', ascii, {direction: RTL}],
  ['ascii-per-line', ascii, {perLine: true}],
  ['fragmented', fragments, {always: true, visible: [only(fragments, 'אב'), only(fragments, 'ده')]}],
  ['duplicate-touch', fragments, {always: true, visible: [only(fragments, 'אב'), only(fragments, 'גד')]}],
  ['touch-close', fragments, {always: true, visible: [only(fragments, '")\n')].map(([from]) => [from + 1, from + 2])}],
  ['short-crlf-gap', gaps],
  ['long-crlf-gap', gaps.replace('abcde', 'abcdef')],
];
for (const [id, source, options = {}] of cases) {
  const {always = false, perLine = false, direction = LTR, visible = [[0, source.replace(/\r\n/g, '\n').length]]} = options;
  const native = inputPositions(source);
  const ext = bidiIsolates({alwaysIsolate: always});
  const [plugin] = ext;
  const view = {
    state: EditorState.create({doc: source, extensions: [wast(), ext, EditorView.perLineTextDirection.of(perLine)]}),
    textDirection: direction, textDirectionAt: () => direction, bidiCache: [],
    visibleRanges: visible.map(([from, to]) => ({from, to})),
    plugin: key => key === plugin ? value : null,
    dispatch(spec) { this.state = this.state.update(spec).state; },
  };
  assert(forceParsing(view, view.state.doc.length, 10000));
  const value = plugin.create(view);
  const [isolates] = view.state.facet(EditorView.bidiIsolatedRanges);
  const [outerMarks] = view.state.facet(EditorView.outerDecorations);
  const marks = isolates(view);
  assert.equal(marks, outerMarks(view));
  const ranges = [];
  for (let mark = marks.iter(); mark.value; mark.next()) {
    const {class: name, attributes, inclusive, bidiIsolate} = mark.value.spec;
    assert.deepEqual([name, attributes.dir, inclusive, bidiIsolate], ['cm-iso', 'auto', true, null]);
    ranges.push([native(mark.from), native(mark.to)]);
  }
  const syntax = [];
  syntaxTree(view.state).iterate({enter(node) {
    if (node.type.prop(NodeProp.isolate)) syntax.push([node.name, native(node.from), native(node.to)]);
  }});
  const lines = [];
  for (let i = 1; i <= view.state.doc.lines; ++i) {
    const line = view.state.doc.line(i);
    const local = offset => native(line.from + offset) - native(line.from);
    const order = EditorView.prototype.bidiSpans.call(view, line);
    lines.push(order.map(({from, to, level}) => [local(from), local(to), level]));
  }
  console.log(JSON.stringify({id, source, always, direction, perLine, visible: visible.map(range => range.map(native)), syntax, ranges, lines}));
}
