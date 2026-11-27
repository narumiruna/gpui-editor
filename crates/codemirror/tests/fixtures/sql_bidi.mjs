// Generate sql_bidi.jsonl: node sql_bidi.mjs <manifest archives with node_modules>
// Requires Node 22.15+. The view stub supplies geometry inputs only; the
// pinned SQL parser, language plugin, state facet, and bidi ordering run as-is.
import assert from 'node:assert/strict';
import { execFileSync } from 'node:child_process';
import { createHash } from 'node:crypto';
import { readFileSync, realpathSync } from 'node:fs';
import { registerHooks } from 'node:module';
import { resolve } from 'node:path';
import { pathToFileURL } from 'node:url';

const root = resolve(process.argv[2]);
const { packages, referenceDependencies } = JSON.parse(readFileSync(new URL('../../../../docs/codemirror-upstream.json', import.meta.url)));
function pinned(name) {
  const { version, integrity } = [...packages, ...referenceDependencies].find(p => p.name === name);
  const artifact = resolve(root, name.replace('/', '__'), `${name.split('/')[1]}-${version}.tgz`);
  const dist = 'dist/index.js';
  const installed = realpathSync(resolve(root, 'node_modules', name, dist));
  assert.equal('sha512-' + createHash('sha512').update(readFileSync(artifact)).digest('base64'), integrity);
  assert.deepEqual(execFileSync('tar', ['-xOf', artifact, 'package/' + dist]), readFileSync(installed));
  return pathToFileURL(installed).href;
}
const [sqlURL, langURL, stateURL, viewURL, commonURL] = [
  '@codemirror/lang-sql', '@codemirror/language', '@codemirror/state', '@codemirror/view', '@lezer/common',
  '@codemirror/autocomplete', '@lezer/lr', '@lezer/highlight',
].map(pinned);
const hook = registerHooks({
  load(url, context, nextLoad) {
    const loaded = nextLoad(url, context);
    return url === viewURL
      ? { ...loaded, source: loaded.source.toString() + '\nexport { getIsolatedRanges };\n' }
      : loaded;
  },
});
const sql = await import(sqlURL);
const { bidiIsolates, forceParsing, syntaxTree } = await import(langURL);
const { EditorState } = await import(stateURL);
const { Direction, EditorView, getIsolatedRanges } = await import(viewURL);
const { NodeProp } = await import(commonURL);
hook.deregister();

const { LTR, RTL } = Direction;
const only = (text, part) => {
  const from = text.indexOf(part);
  assert(from >= 0);
  return [from, from + part.length];
};
const mixed = `/* אב\nعرب */ SELECT ('אב' + 12), s."جد", B'01', X'AB'; -- גד!`;
const ascii = `SELECT 'left', "name"; /* abc */ -- end`;
const windows = `SELECT 'אב' -- גד\nSELECT 'a';\nSELECT 'جد';\nSELECT 'b'`;
const gap = `-- אב\n'a'\n-- جد\nSELECT 'not marked';\n-- גד`;
const touch = `SELECT 'אב'"גד"; -- جد`;
const repeated = `SELECT 'אב...גד'; -- جد`;
const long = ["SELECT '", 4085, "א😀גד';"];
const sourceText = input => typeof input === 'string' ? input : input[0] + 'a'.repeat(input[1]) + input[2];
const cases = [
  ['mixed', 'StandardSQL', mixed],
  ['ascii-default', 'StandardSQL', ascii],
  ['ascii-always', 'StandardSQL', ascii, { always: true }],
  ['ascii-rtl', 'StandardSQL', ascii, { direction: RTL }],
  ['ascii-per-line', 'StandardSQL', ascii, { perLine: true }],
  ['multiple-visible-ranges', 'StandardSQL', windows, { always: true, visible: [only(windows, 'אב'), only(windows, 'جد')] }],
  ['touching-endpoint', 'StandardSQL', touch, { always: true, visible: [[touch.indexOf('"'), touch.indexOf('"') + 1]] }],
  ['duplicate-touch', 'StandardSQL', repeated, { always: true, visible: [only(repeated, 'אב'), only(repeated, 'גד')] }],
  ['short-and-long-ltr-gaps', 'StandardSQL', gap],
  ['clipped-line-suffix-before-rtl', 'StandardSQL', touch, { visible: [only(touch, '"גד')] }],
  ['clipped-line-suffix-after-rtl', 'StandardSQL', `-- אב tail`, { visible: [[6, 10]] }],
  ['mysql', 'MySQL', 'SELECT `جد`, "אב", # גד\n  @@foo, 0b10;'],
  ['postgres-dollars', 'PostgreSQL', 'SELECT $x$אב\nABC 😀$x$, n\'جد\'; -- גד'],
  ['mssql-brackets', 'MSSQL', 'SELECT [אב], "جد", B\'01\';'],
  ['long-line-default', 'StandardSQL', long, { visible: [only(sourceText(long), '😀גד')] }],
  ['long-line-with-rtl-line', 'StandardSQL', [long[0], long[1], long[2] + '\n-- א'], { visible: [only(sourceText(long), '😀גד')] }],
];
function byteAt(text, pos) {
  assert(!(pos > 0 && (text.charCodeAt(pos) & 0xfc00) === 0xdc00), 'surrogate boundary');
  return Buffer.byteLength(text.slice(0, pos));
}

for (const [id, dialect, input, options = {}] of cases) {
  const source = sourceText(input);
  const { always = false, perLine = false, direction = LTR, visible = [[0, source.length]] } = options;
  const lang = sql[dialect].language;
  const isolateTypes = lang.parser.nodeSet.types
    .filter(t => t.prop(NodeProp.isolate) !== undefined)
    .map(t => [t.id, t.name, t.prop(NodeProp.isolate)]);
  assert.deepEqual(isolateTypes, [[1, 'LineComment', 'auto'], [2, 'BlockComment', 'auto'], [3, 'String', 'auto'], [19, 'QuotedIdentifier', 'auto']]);
  const isolates = bidiIsolates({ alwaysIsolate: always });
  const [plugin] = isolates;
  const view = {
    state: EditorState.create({ doc: source, extensions: [lang, isolates, EditorView.perLineTextDirection.of(perLine)] }),
    textDirection: direction, visibleRanges: visible.map(([from, to]) => ({ from, to })),
    textDirectionAt: () => direction, bidiCache: [], plugin: key => key === plugin ? value : null,
    dispatch(spec) { this.state = this.state.update(spec).state; },
  };
  assert(forceParsing(view, view.state.doc.length, 10000));
  const { state } = view;
  const value = plugin.create(view);
  const [getMarks] = state.facet(EditorView.bidiIsolatedRanges);
  const [getOuterMarks] = state.facet(EditorView.outerDecorations);
  const marks = getMarks(view);
  assert.equal(marks, getOuterMarks(view));
  const pos = p => byteAt(source, p);
  const ranges = [];
  for (let cur = marks.iter(); cur.value; cur.next()) {
    const { class: className, attributes, inclusive, bidiIsolate } = cur.value.spec;
    assert.deepEqual([className, attributes.dir, inclusive, bidiIsolate], ['cm-iso', 'auto', true, null]);
    ranges.push([pos(cur.from), pos(cur.to)]);
  }
  const syntax = [];
  syntaxTree(state).iterate({ enter(node) {
    const property = node.type.prop(NodeProp.isolate);
    if (property) syntax.push([node.name, pos(node.from), pos(node.to)]);
  } });
  const lines = [];
  for (let number = 1; number <= state.doc.lines; number++) {
    const line = state.doc.line(number);
    const local = p => byteAt(line.text, p);
    const spans = v => EditorView.prototype.bidiSpans.call(v, line).map(s => [local(s.from), local(s.to), s.level]);
    const plain = { ...view, state: EditorState.create({ doc: state.doc }), bidiCache: [] };
    const clip = i => [local(i.from), local(i.to), i.inner.map(clip)];
    lines.push({ isolated: getIsolatedRanges(view, line).map(clip), spans: spans(view), plain: spans(plain) });
  }
  process.stdout.write(JSON.stringify({ id, dialect, input, always, perLine, direction,
    visible: visible.map(r => r.map(pos)), ranges, syntax, lines }) + '\n');
}
