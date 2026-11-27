// Run with Node and pinned npm artifacts:
// node markdown_markup.mjs <archive-root> > markdown_markup.jsonl
import assert from 'node:assert/strict';
import {execFileSync} from 'node:child_process';
import {createHash} from 'node:crypto';
import {readFileSync, realpathSync} from 'node:fs';
import {resolve} from 'node:path';
import {pathToFileURL} from 'node:url';

const root = resolve(process.argv[2]);
const {packages, referenceDependencies} = JSON.parse(readFileSync(new URL('../../../../docs/codemirror-upstream.json', import.meta.url)));
function verified(name) {
  const {version, integrity} = [...packages, ...referenceDependencies].find(p => p.name === name);
  const archive = resolve(root, name.replace('/', '__'), `${name.split('/')[1]}-${version}.tgz`);
  const file = realpathSync(resolve(root, 'node_modules', name, 'dist/index.js'));
  assert.equal('sha512-' + createHash('sha512').update(readFileSync(archive)).digest('base64'), integrity);
  assert.deepEqual(execFileSync('tar', ['-xOf', archive, 'package/dist/index.js']), readFileSync(file));
  return pathToFileURL(file).href;
}
const [markdownURL, stateURL, , , languageURL] = [
  '@codemirror/lang-markdown', '@codemirror/state', '@codemirror/autocomplete',
  '@codemirror/lang-html', '@codemirror/language', '@codemirror/view',
  '@lezer/markdown', '@lezer/common', '@lezer/highlight', '@lezer/lr',
].map(verified);
const {markdown, markdownKeymap, insertNewlineContinueMarkupCommand, deleteMarkupBackward} = await import(markdownURL);
const {EditorState, EditorSelection, Transaction} = await import(stateURL);
const {indentUnit} = await import(languageURL);
console.log(JSON.stringify(markdownKeymap.map(({key, preventDefault}) => [key, !!preventDefault])));

for (const [name, marked, command, nonempty = false, unit] of [
  ['two Unicode bullet items', '- α😀|\n- second|', 'enter'],
  ['overlapping ordered renumbering', '1. one|\n2. two|\n3. tail', 'enter'],
  ['quoted list and outer list', '> - α|\n> - β|\n\n- other|', 'enter'],
  ['unsupported second caret', '- α|\n\nplain|', 'enter'],
  ['partially selected source', '- α|\n- β|', 'enter', true],
  ['remove marker and excess padding', '- |猫\n-    |犬', 'delete'],
  ['remove quote and bullet', '> |🐍\n\n- |犬', 'delete'],
  ['unsupported second deletion', '- |猫\n\nplain|', 'delete'],
  ['exit an empty tight item', '1. one\n2. |\n\n- other|', 'tight'],
  ['normalize a nested list prefix', '- parent\n    - child|\n    - next', 'enter', false, '\t'],
  ['keep spaces with a two-tab indent unit', '- parent\n    - child|\n    - next', 'enter', false, '\t\t'],
  ['keep the quote before a nested list prefix', '> - parent\n>     - child|', 'enter', false, '\t'],
  ['keep spacing when deletion starts after the line start', '1. a\n   1. b\n   2. |c', 'delete', false, '\t'],
  ['normalize a deleted marker at the line start', '123. a\n124. |b', 'delete', false, '\t'],
]) {
  let source = '';
  const positions = [];
  for (const ch of marked) {
    if (ch === '|') positions.push(source.length);
    else source += ch;
  }
  const ranges = positions.map((at, i) => EditorSelection.range(nonempty && i === 0 ? at - 1 : at, at));
  const main = ranges.length - 1;
  const extensions = [EditorState.allowMultipleSelections.of(true), markdown()];
  if (unit) extensions.push(indentUnit.of(unit));
  const state = EditorState.create({doc: source, selection: EditorSelection.create(ranges, main), extensions});
  const run = command === 'delete' ? deleteMarkupBackward : insertNewlineContinueMarkupCommand(
    command === 'tight' ? {nonTightLists: false} : {});
  let result;
  const handled = run({state, dispatch(update) { result = update; }});
  assert.equal(handled, !!result);
  const byte = (text, unit) => Buffer.byteLength(text.slice(0, unit));
  const selection = ranges.map(range => [byte(source, range.anchor), byte(source, range.head)]);
  const output = result?.newDoc.toString();
  console.log(JSON.stringify({name, source, command, unit, selection, main, handled, output,
    selected: result?.newSelection.ranges.map(range => byte(output, range.head)),
    primary: result?.newSelection.mainIndex, event: result?.annotation(Transaction.userEvent),
    scroll: result?.scrollIntoView}));
}
