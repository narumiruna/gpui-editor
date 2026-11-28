// node yaml_editor_lines.mjs <archive-root> > yaml_editor_lines.jsonl
// node yaml_editor_lines.mjs <archive-root> --input > yaml_editor_input.jsonl
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
const [yamlURL, languageURL, stateURL] = [
  '@codemirror/lang-yaml', '@codemirror/language', '@codemirror/state',
  '@lezer/yaml', '@lezer/lr', '@lezer/common', '@lezer/highlight', '@codemirror/streamparser', '@codemirror/view',
].map(pinned);
const {yaml, yamlLanguage} = await import(yamlURL);
const {foldable, forceParsing, getIndentation, indentOnInput} = await import(languageURL);
const {EditorSelection, EditorState, Transaction} = await import(stateURL);

function printLines() {
  for (const source of [
    'start:\r  list:\r    - 😀\r    - hi\rfinish: 1',
    'title: |\r  😀 hi\r  second\rnext: no',
    'outer:\r  [😀,\r    true,\r    false\r  ]',
    'start:\r\n  expr: {\r    text: "名😀"\r      ',
    '{\r  word: 字\r}',
    '# pre 😀\r\n\r  # a\r\n\r# b\nkey: value',
    '---\r\n# a\n# b 😀\rkey:\r\n  nested: value',
    '%YAML 1.2\n---\r\n\r  # a\n\r# b 😀\r\nkey: {\n  nested: value',
    'one: x\r\n...\r\n---\n# a\r  # b 😀\r\nkey:\n  nested: value',
    '---\n  # only\nkey: value\n# trailing 😀',
    '---\r\n# a\r# b 😀\r\nkey: ' + 'x'.repeat(1024),
  ]) {
    const view = {state: EditorState.create({doc: source, extensions: yaml()}), dispatch(spec) {
      this.state = this.state.update(spec).state;
    }};
    assert(forceParsing(view, view.state.doc.length, 5000));
    const {state} = view;
    const {byteAt, units} = projectedOffsets(source);
    assert.equal(state.doc.length, units);
    const lines = [];
    for (let number = 1; number <= state.doc.lines; ++number) {
      const line = state.doc.line(number);
      const fold = foldable(state, line.from, line.to);
      lines.push([byteAt(line.from), byteAt(line.to), fold ? [byteAt(fold.from), byteAt(fold.to)] : null,
        getIndentation(state, line.from), getIndentation(state, line.to)]);
    }
    console.log(JSON.stringify({source, lines}));
  }
}

function printInput() {
  for (const [name, marked, typed, event] of [
    ['mapping closer', 'root: {\r\n  v: "名😀"\r    |', '}', 'input.type'],
    ['nested sequence closer', 'root:\r\n  [\r    "😀"\r      |', ']', 'input.complete'],
    ['paste is not reindented', 'root: {\r\n  v: "名😀"\r    |', '}', 'input.paste'],
  ]) {
    const source = marked.replace('|', '');
    const {units} = projectedOffsets(marked.slice(0, marked.indexOf('|')));
    const {byteAt} = projectedOffsets(source);
    const state = EditorState.create({doc: source, selection: EditorSelection.single(units),
      extensions: [yaml(), indentOnInput()]});
    const transaction = state.update({...state.replaceSelection(typed), annotations: Transaction.userEvent.of(event)});
    const result = projectInput(source, transaction);
    console.log(JSON.stringify({name, source, caret: byteAt(units), typed, event, result,
      finalCaret: projectedOffsets(result).byteAt(transaction.newSelection.main.head)}));
  }
}

// The stock TreeBuffer wraps a negative start offset in this input. The
// unbuffered parser exposes the same grammar node with an in-bounds start.
const bufferedSource = 'a: x\n...\n# pre\n# 📦\nb: y';
function secondDocument(parser) {
  const tree = parser.parse(bufferedSource);
  const document = tree.topNode.getChildren('Document').at(1);
  assert(document);
  assert.equal(document.to, bufferedSource.length);
  return document.from;
}
assert.equal(secondDocument(yamlLanguage.parser), 65_544);
assert.equal(secondDocument(yamlLanguage.parser.configure({bufferLength: 0})), 8);

if (process.argv.includes('--input')) printInput();
else printLines();
