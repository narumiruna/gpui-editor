// Run `node lezer_recovery_depth.mjs <pinned-package-directory>` to generate
// lezer_recovery_depth.json. The directory must contain the pinned node_modules.
import {readFileSync} from 'node:fs';
import {createRequire} from 'node:module';
import {dirname, resolve} from 'node:path';

const pins = {
  '@codemirror/lang-wast': '6.0.2',
  '@lezer/lr': '1.4.13',
  '@lezer/common': '1.5.7',
};
function loadFrom(require, name) {
  const entry = require.resolve(name);
  const {version} = JSON.parse(readFileSync(resolve(dirname(entry), '../package.json')));
  if (version !== pins[name]) throw new Error(`Expected ${name} ${pins[name]}, got ${version}`);
  return createRequire(entry);
}
const require = createRequire(resolve(process.argv[2], 'package.json'));
const wastRequire = loadFrom(require, '@codemirror/lang-wast');
const lrRequire = loadFrom(wastRequire, '@lezer/lr');
loadFrom(lrRequire, '@lezer/common');
const {wastLanguage: {parser}} = require('@codemirror/lang-wast');

function summarize(opens, closes, tail) {
  const source = '('.repeat(opens) + ')'.repeat(closes) + tail;
  // These cases are ASCII, so tree offsets are also byte offsets.
  let depth = 0, records = 0, errors = 0, hash = 2166136261;
  let appEnd = null, firstError = null;
  parser.parse(source).iterate({
    enter(node) {
      if (node.name === 'App' && node.from === 1999) appEnd = node.to;
      if (node.type.isError && errors++ === 0) firstError = [node.from, node.to];
      records++;
      for (const byte of Buffer.from(`${depth++}:${node.name}:${node.from}:${node.to};`)) {
        hash = Math.imul(hash ^ byte, 16777619);
      }
    },
    leave() { depth--; },
  });
  return {opens, closes, tail, records, errors, hash: hash >>> 0, appEnd, firstError};
}

const cases = [
  [12, 12, ''], [12, 0, '?'],
  [2798, 0, ''], [2799, 0, ''], [2800, 0, ''], [2801, 0, ''],
  [3599, 0, ''], [3600, 0, ''], [4000, 0, ''], [4000, 0, '?'],
  [4000, 2000, '??'],
  [2798, 0, '(result i32)'], [2799, 0, ')(?'],
  [2799, 0, '?"' + 'z'.repeat(26)],
];
console.log(`{
  "packages": ${JSON.stringify(pins)},
  "cases": [
${cases.map(args => `    ${JSON.stringify(summarize(...args))}`).join(',\n')}
  ]
}`);
