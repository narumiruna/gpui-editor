// Run `node lezer_recovery_repeats.mjs <pinned-package-directory>` to generate
// lezer_recovery_repeats.json. The directory must contain the pinned node_modules.
import { readFileSync } from 'node:fs';
import { createRequire } from 'node:module';
import { dirname, resolve } from 'node:path';

const require = createRequire(resolve(process.argv[2], 'package.json'));
const packages = {
  '@lezer/json': '1.0.3',
  '@lezer/generator': '1.8.1',
  '@lezer/lr': '1.4.13',
  '@lezer/common': '1.5.7',
  '@lezer/highlight': '1.2.5',
};
for (const [name, pin] of Object.entries(packages)) {
  const file = resolve(dirname(require.resolve(name)), '../package.json');
  const { version } = JSON.parse(readFileSync(file));
  if (version !== pin) throw new Error(`Expected ${name} ${pin}, got ${version}`);
}
const { parser: json } = require('@lezer/json');
const { buildParser } = require('@lezer/generator');
const blocks = buildParser(String.raw`@top Main { Block* }
Block { "(" Word (":" Word)* ")" }
@skip { space | Comment }
@tokens { Word { $[a-z]+ } space { $[ \t\n\r]+ } Comment { "#" $[a-z]* } }`);

function records(parser, source) {
  const byte = pos => {
    if (/[\uDC00-\uDFFF]/.test(source[pos])) throw new Error(`Split scalar at ${pos}`);
    return Buffer.byteLength(source.slice(0, pos));
  };
  const nodes = [];
  let depth = 0;
  parser.parse(source).iterate({
    enter(node) { nodes.push([depth++, node.name, byte(node.from), byte(node.to)]); },
    leave() { depth--; },
  });
  return `    {"source": ${JSON.stringify(source)}, "nodes": [\n${nodes.map(node =>
    `      ${JSON.stringify(node)}`).join(',\n')}\n    ]}`;
}

const sections = [
  ['json', json, ['[true2,\n4,:🐍', '[1,2,:🐍', '[1,:🐍']],
  ['blocks', blocks, ['(one:two:', '(one:two:🐍\r\n', '(one:two:@ #end',
    '(one:two:@ ) (three:four:@', '(one:@', '(one:two:@)']],
];
console.log(`{\n  "packages": ${JSON.stringify(packages)},\n${sections.map(([name, parser, cases]) =>
  `  "${name}": [\n${cases.map(source => records(parser, source)).join(',\n')}\n  ]`).join(',\n')}\n}`);
