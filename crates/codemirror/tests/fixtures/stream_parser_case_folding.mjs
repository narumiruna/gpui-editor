// Generate the UTF-16 stream's simple fold table on Node 26.10.0:
// curl -fsSLO https://www.unicode.org/Public/17.0.0/ucd/CaseFolding.txt
// node stream_parser_case_folding.mjs CaseFolding.txt > case_fold_ranges.rs
import assert from 'node:assert/strict';
import {createHash} from 'node:crypto';
import {readFileSync} from 'node:fs';

const data = readFileSync(process.argv[2]);
assert.equal(createHash('sha256').update(data).digest('hex'),
  'ff8d8fefbf123574205085d6714c36149eb946d717a0c585c27f0f4ef58c4183');
const entries = [];
for (const line of data.toString('utf8').split('\n')) {
  const [code, status, folded] = line.split('#')[0].split(';').map(value => value.trim());
  if (status !== 'C' && status !== 'S') continue;
  const from = Number.parseInt(code, 16), to = Number.parseInt(folded, 16);
  const source = String.fromCodePoint(from), target = String.fromCodePoint(to);
  // V8 canonicalizes a singleton \q operand and matches it without folding input.
  const positive = new RegExp(`^[\\q{\\u{${code}}}]$`, 'iv');
  const negative = new RegExp(`^[^\\q{\\u{${code}}}]$`, 'iv');
  assert(positive.test(target) && !positive.test(source), code);
  assert(!negative.test(target) && negative.test(source), code);
  entries.push([from, to]);
}
assert.equal(entries.length, 1512);
console.log('// Generated from Unicode 17.0 CaseFolding.txt (C and S entries).');
console.log('// See docs/licenses/unicode-data.txt. Generate with stream_parser_case_folding.mjs.');
console.log('&[');
for (let start = 0; start < entries.length;) {
  const [first, target] = entries[start];
  const delta = target - first;
  const stride = entries[start + 1]?.[0] - first === 2 ? 2 : 1;
  let end = start + 1;
  while (end < entries.length && entries[end][0] - entries[end - 1][0] === stride
      && entries[end][1] - entries[end][0] === delta) ++end;
  const last = entries[end - 1][0];
  console.log(`    (0x${first.toString(16)}, 0x${last.toString(16)}, ${stride}, ${delta}),`);
  start = end;
}
console.log(']');
