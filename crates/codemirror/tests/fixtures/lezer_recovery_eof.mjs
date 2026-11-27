// Install these exact packages in a temporary directory with npm --ignore-scripts.
// Run `node lezer_recovery_eof.mjs <directory> > lezer_recovery_eof.json`.
import { readFileSync } from 'node:fs';
import { createRequire } from 'node:module';
import { dirname, resolve } from 'node:path';

const pins = {
  '@lezer/json': '1.0.3',
  '@lezer/lr': '1.4.13',
  '@lezer/common': '1.5.7',
  '@lezer/highlight': '1.2.5',
};
function loadFrom(require, name) {
  const entry = require.resolve(name);
  const {version} = JSON.parse(readFileSync(resolve(dirname(entry), '../package.json')));
  if (version !== pins[name]) throw new Error(`Expected ${name} ${pins[name]}, got ${version}`);
  return createRequire(entry);
}
const require = createRequire(resolve(process.argv[2], 'package.json'));
const jsonRequire = loadFrom(require, '@lezer/json');
const lrRequire = loadFrom(jsonRequire, '@lezer/lr');
loadFrom(lrRequire, '@lezer/common');
loadFrom(jsonRequire, '@lezer/highlight');
const {parser} = require('@lezer/json');

function records(source) {
  // These cases are ASCII, so UTF-16 and UTF-8 positions agree.
  const nodes = [];
  let depth = 0;
  parser.parse(source).iterate({
    enter(node) { nodes.push([depth++, node.name, node.from, node.to]); },
    leave() { depth--; },
  });
  return nodes;
}

const cases = ['['.repeat(5) + '1', '['.repeat(12) + '1', '{"a":['.repeat(6) + 'true'];
console.log(`{\n  "packages": ${JSON.stringify(pins)},\n  "cases": [\n${cases.map(source =>
  `    {"source": ${JSON.stringify(source)}, "nodes": [\n${records(source).map(node =>
    `      ${JSON.stringify(node)}`).join(',\n')}\n    ]}`).join(',\n')}\n  ]\n}`);
