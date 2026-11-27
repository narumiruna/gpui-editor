// Run on Node with the manifest's pinned npm archives and node_modules.
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
const [jsonURL, languageURL, stateURL] = [
  '@codemirror/lang-json', '@codemirror/language', '@codemirror/state',
  '@lezer/json', '@lezer/lr', '@lezer/common', '@lezer/highlight', '@codemirror/streamparser', '@codemirror/view',
].map(pinned);
const {json} = await import(jsonURL);
const {forceParsing, matchBrackets} = await import(languageURL);
const {EditorState} = await import(stateURL);

function offsets(source) {
  const map = new Map([[0, 0]]);
  let pos = 0, byte = 0;
  for (const [part] of source.matchAll(/\r\n|[^]/gu)) {
    pos += part === '\r\n' ? 1 : part.length;
    byte += Buffer.byteLength(part);
    map.set(pos, byte);
  }
  return map;
}

for (const source of [
  '{\r\n "(key)": ["[value]", {"😀": "(漢)"}]\r\n}',
  '{\r "x": [1, 2, {"c": "}"}],\r "y": {"} [": "]"}\r}',
  '["(", "[", "]", ")", 6]',
  '{"(one": 1, "two)": 2, "s": ")"]}',
  '{"a": [1, 2}}',
  '{"a": ["[x", {"b": 4}',
  '{"a": [1 ], "b": {}} trailing ( [ )',
  '"🦀 [ ( )"',
]) {
  const map = offsets(source);
  const view = {state: EditorState.create({doc: source, extensions: json()}), dispatch(spec) {
    this.state = this.state.update(spec).state;
  }};
  assert(forceParsing(view, view.state.doc.length, 5000));
  const {state} = view;
  const range = ({from,to}) => {
    assert(map.has(from) && map.has(to));
    return [map.get(from), map.get(to)];
  };
  const hits = [...map].map(([pos, byte]) => {
    const hit = matchBrackets(state, pos, -1)
      || (pos > 0 && matchBrackets(state, pos - 1, 1))
      || matchBrackets(state, pos, 1)
      || (pos < state.doc.length && matchBrackets(state, pos + 1, -1));
    return [byte, hit ? [range(hit.start), hit.end ? range(hit.end) : null, hit.matched] : null];
  });
  console.log(JSON.stringify({source, hits}));
}
