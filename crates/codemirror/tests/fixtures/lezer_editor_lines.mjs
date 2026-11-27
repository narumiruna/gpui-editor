// Generate with Node and the pinned npm archives plus node_modules:
// node lezer_editor_lines.mjs <archive-root> > lezer_editor_lines.jsonl
// node lezer_editor_lines.mjs <archive-root> --input > lezer_editor_input.jsonl
import assert from 'node:assert/strict';
import {execFileSync} from 'node:child_process';
import {createHash} from 'node:crypto';
import {readFileSync, realpathSync} from 'node:fs';
import {resolve} from 'node:path';
import {pathToFileURL} from 'node:url';
import {projectedOffsets, projectInput} from './editor_line_offsets.mjs';

const root = resolve(process.argv[2]);
const {packages, referenceDependencies} = JSON.parse(readFileSync(new URL('../../../../docs/codemirror-upstream.json', import.meta.url)));
function pinned(name) {
  const {version, integrity} = [...packages, ...referenceDependencies].find(p => p.name === name);
  const archive = resolve(root, name.replace('/', '__'), `${name.split('/')[1]}-${version}.tgz`);
  const file = realpathSync(resolve(root, 'node_modules', name, 'dist/index.js'));
  assert.equal('sha512-' + createHash('sha512').update(readFileSync(archive)).digest('base64'), integrity);
  assert.deepEqual(execFileSync('tar', ['-xOf', archive, 'package/dist/index.js']), readFileSync(file));
  return pathToFileURL(file).href;
}
const [lezerURL, languageURL, stateURL] = [
  '@codemirror/lang-lezer', '@codemirror/language', '@codemirror/state',
  '@lezer/lezer', '@lezer/lr', '@lezer/common', '@lezer/highlight', '@codemirror/streamparser', '@codemirror/view',
].map(pinned);
const {lezer} = await import(lezerURL);
const {foldable, forceParsing, getIndentation, indentOnInput, syntaxTree} = await import(languageURL);
const {EditorSelection, EditorState, Transaction} = await import(stateURL);

const sources = [
  '// α😀\r@top Main {\r\n  // separate 🦀\r  Word\r}\rWord { "漢" }\n',
  '@tokens {\r\n  Word { $[a-z] }\r  Number { @digit+ }\r\n}\r',
  'Rule {\r  /* α\r    beta */ Other\r  Next\r}\n',
  '{(<>([d}S{y}',
];

function printInput() {
  for (const [name, marked, typed, event] of [
    ['top Rule closer', '@top Rule {\r\n  Word\r    |\r\nWord { "😀" }', '}', 'input.type'],
    ['nested token closer', '@tokens {\r\n  Word {\r    "😀"\r      |\r\n}\r\n', '}', 'input.type'],
    ['rule closer completion', 'Rule {\r  Word\r    |', '}', 'input.complete'],
    ['non-brace input', 'Rule {\r  Word\r    |', ']', 'input.type'],
    ['paste is not reindented', 'Rule {\r  Word\r    |', '}', 'input.paste'],
  ]) {
    const source = marked.replace('|', '');
    const before = marked.slice(0, marked.indexOf('|'));
    const normalized = before.replace(/\r\n?/g, '\n').length;
    const {byteAt: original} = projectedOffsets(source);
    const state = EditorState.create({doc: source, selection: EditorSelection.single(normalized),
      extensions: [lezer(), indentOnInput()]});
    const result = state.update({...state.replaceSelection(typed), annotations: Transaction.userEvent.of(event)});
    const output = projectInput(source, result);
    const finalCaret = projectedOffsets(output).byteAt(result.newSelection.main.head);
    console.log(JSON.stringify({name, source, caret: original(normalized), typed, event, result: output, finalCaret}));
  }
}

if (process.argv.includes('--input')) printInput();
for (const source of process.argv.includes('--input') ? [] : sources) {
  const {byteAt: original} = projectedOffsets(source);
  const view = {state: EditorState.create({doc: source, extensions: lezer()}), dispatch(spec) {
    this.state = this.state.update(spec).state;
  }};
  assert(forceParsing(view, view.state.doc.length, 5000));
  const {state} = view;
  const nodes = [];
  let depth = 0;
  syntaxTree(state).iterate({enter(node) {
    nodes.push([depth++, node.name, original(node.from), original(node.to)]);
  }, leave() { --depth; }});
  const lines = [];
  for (let number = 1; number <= state.doc.lines; ++number) {
    const line = state.doc.line(number);
    const fold = foldable(state, line.from, line.to);
    lines.push({range: [original(line.from), original(line.to)],
      indent: getIndentation(state, line.from), fold: fold ? [original(fold.from), original(fold.to)] : null});
  }
  console.log(JSON.stringify({source, nodes, lines}));
}
