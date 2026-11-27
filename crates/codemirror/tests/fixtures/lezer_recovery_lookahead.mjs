// Run `node lezer_recovery_lookahead.mjs <pinned-package-directory>` to generate
// lezer_recovery_lookahead.json. The directory must contain the pinned node_modules.
import {readFileSync} from 'node:fs';
import {createRequire} from 'node:module';
import {dirname, resolve} from 'node:path';

const require = createRequire(resolve(process.argv[2], 'package.json'));
const packages = {'@lezer/generator': '1.8.1', '@lezer/lr': '1.4.13', '@lezer/common': '1.5.7'};
for (const [name, pin] of Object.entries(packages)) {
  const file = resolve(dirname(require.resolve(name)), '../package.json');
  const {version} = JSON.parse(readFileSync(file));
  if (version !== pin) throw new Error(`Expected ${name} ${pin}, got ${version}`);
}
const {buildParserFile} = require('@lezer/generator');
const {ExternalTokenizer, LRParser} = require('@lezer/lr');

const grammar = String.raw`@top Main { item* }
@top Mark { mark }
item { Group | First | Preferred | Word | String | Probe }
Group { Open item* Close }
First { FirstStart item* "]" }
Preferred { PreferredStart item* "]" }
FirstStart { "[" ~start }
PreferredStart[@dynamicPrecedence=1] { "[" ~start }
@skip { space | Comment }
@external tokens probe from "./probe.js" { Probe, mark }
@tokens {
  Open { "(" }
  Close { ")" }
  Word { ($[a-zé] | "😀")+ }
  String { '"' !["]* '"' }
  Comment { "#" $[a-z]* }
  space { $[ \t\n]+ }
}`;

const {parser: code, terms} = buildParserFile(grammar, {moduleStyle: 'cjs'});
const termModule = {};
new Function('module', terms)(termModule);
const ids = termModule.exports;
let table;
const exports = {};
new Function('require', 'exports', code)(name => {
  if (name === '@lezer/lr') return {LRParser: {deserialize(spec) {
    table = {...spec, tokenizers: spec.tokenizers.map(slot => typeof slot === 'number' ? slot : 'probe')};
    return LRParser.deserialize(spec);
  }}};
  if (name === './probe.js') return {probe: new ExternalTokenizer(input => {
    const delimiter = input.next;
    if (delimiter !== 96 && delimiter !== 126) return;
    for (let offset = 1, unit; (unit = delimiter === 96 ? input.advance() : input.peek(offset)) >= 0; offset++) {
      if (unit === 96) {
        input.acceptToken(ids.Probe, delimiter === 96 ? 1 : offset + 1);
        return;
      }
    }
    if (delimiter === 126) input.acceptToken(ids.mark, 1);
  })};
  throw new Error(`Unexpected module ${name}`);
}, exports);

const cases = [
  ['dfa-below', '?"' + 'a'.repeat(25)],
  ['dfa-above', '?"' + 'a'.repeat(26)],
  ['dfa-bmp', '?"' + 'é'.repeat(25)],
  ['dfa-astral-below', '?"' + '😀'.repeat(12) + 'a'],
  ['dfa-astral-above', '?"' + '😀'.repeat(13)],
  ['end-before-eof', '?"' + 'a'.repeat(25) + '\n'],
  ['named-deletion', ')"' + 'a'.repeat(26)],
  ['group', '(?"' + 'a'.repeat(26) + ')'],
  ['group-skip-eof', '(?"' + 'a'.repeat(26) + ' #end'],
  ['external-below', '?`' + 'a'.repeat(25)],
  ['external-above', '?`' + 'a'.repeat(26)],
  ['external-astral', '?`' + '😀'.repeat(13)],
  ['external-closed', '?`' + 'a'.repeat(26) + '`'],
  ['external-short-below', '?~' + 'a'.repeat(25)],
  ['external-short-above', '?~' + 'a'.repeat(26)],
  ['cached-branch', '[a?`' + 'a'.repeat(26) + ']'],
  ['valid', '("' + 'a'.repeat(26) + '" `a😀` é)'],
];

function records(source) {
  const byte = pos => {
    if (/[\uDC00-\uDFFF]/.test(source[pos])) throw new Error(`Split scalar at ${pos}`);
    return Buffer.byteLength(source.slice(0, pos));
  };
  const nodes = [];
  let depth = 0;
  exports.parser.parse(source).iterate({
    enter(node) { nodes.push([depth++, node.name, byte(node.from), byte(node.to)]); },
    leave() { depth--; },
  });
  return nodes;
}

console.log(`{\n  "packages": ${JSON.stringify(packages)},\n  "table": ${JSON.stringify(table)},\n  "terms": ${JSON.stringify({Probe: ids.Probe, mark: ids.mark})},\n  "cases": [\n${cases.map(([id, source]) =>
  `    ${JSON.stringify({id, source, nodes: records(source)})}`).join(',\n')}\n  ]\n}`);
