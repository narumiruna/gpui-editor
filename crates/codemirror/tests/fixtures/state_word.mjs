// From the repo root, run with the upstream archive directory and the
// @marijn/find-cluster-break 1.0.4 tarball as arguments. Redirect stdout to
// crates/codemirror/tests/fixtures/state_word.jsonl.
// Add --columns to emit state_columns.jsonl instead.
import assert from 'node:assert/strict';
import { execFileSync } from 'node:child_process';
import { createHash } from 'node:crypto';
import { readFileSync, realpathSync } from 'node:fs';
import { findPackageJSON } from 'node:module';
import { dirname, resolve } from 'node:path';
import { pathToFileURL } from 'node:url';

const manifest = JSON.parse(readFileSync(new URL('../../../../docs/codemirror-upstream.json', import.meta.url)));
const { name, version, tarball, integrity } = manifest.packages.find(pin => pin.name === '@codemirror/state');
const directory = resolve(process.argv[2], '@codemirror__state');
const stateArchive = resolve(directory, tarball.split('/').at(-1));
const index = pathToFileURL(resolve(directory, 'package/dist/index.js'));
const cluster = {
  name: '@marijn/find-cluster-break',
  version: '1.0.4',
  integrity: 'sha512-Wy0V7+SGUjnF9/TkiM1hKVDPj7jKXduPNboMVtHTA8dySMURWqfg/JZ9E2Sq8JgSJmkl7k7Qe9FLeMSrSraWmQ==',
};

function verify(archive, pin, installed, entry) {
  const hash = createHash('sha512').update(readFileSync(archive)).digest('base64');
  assert.equal(`sha512-${hash}`, pin.integrity, pin.name);
  for (const file of ['package.json', entry]) {
    assert.deepEqual(readFileSync(resolve(installed, file)),
      execFileSync('tar', ['-xOf', archive, `package/${file}`]), `${pin.name}/${file}`);
  }
  const pkg = JSON.parse(readFileSync(resolve(installed, 'package.json')));
  assert.equal(pkg.name, pin.name);
  assert.equal(pkg.version, pin.version);
  assert.equal(pkg.exports.import ?? pkg.exports['.']?.import ?? pkg.main, `./${entry}`);
}

verify(stateArchive, { name, version, integrity }, resolve(directory, 'package'), 'dist/index.js');
const clusterDir = dirname(realpathSync(findPackageJSON(cluster.name, index)));
verify(process.argv[3], cluster, clusterDir, 'src/index.js');
assert.equal(process.versions.unicode, '17.0');
const { EditorState, CharCategory, countColumn, findColumn, findClusterBreak } = await import(index);

function record(name, source, providers, probes = []) {
  // Map UTF-16 document offsets to original UTF-8 byte offsets. A CRLF is
  // one document unit; an offset inside a surrogate pair has no byte mapping.
  const bytes = [0];
  let byte = 0;
  for (const ch of source.match(/\r\n|[^]/gu) ?? []) {
    if (ch.length === 2 && ch !== '\r\n') bytes.push(null);
    bytes.push(byte += Buffer.byteLength(ch));
  }
  const state = EditorState.create({ doc: source, extensions: providers.map(({ chars, scope }) =>
    EditorState.languageData.of((_, at, side) => {
      assert.equal(side, -1);
      return !scope || (scope[0] <= bytes[at] && bytes[at] < scope[1]) ? [{ wordChars: chars }] : [];
    })) });
  assert.equal(state.sliceDoc(), source.replaceAll('\r\n', '\n'));
  assert.equal(state.doc.length + 1, bytes.length);
  const categories = probes.map(([at, cluster]) => {
    const pos = bytes.indexOf(at);
    assert.notEqual(pos, -1);
    return [at, cluster, CharCategory[state.charCategorizer(pos)(cluster)]];
  });
  const words = bytes.flatMap((byte, pos) => {
    if (byte === null) return [];
    const word = state.wordAt(pos);
    if (!word) return [[byte, null]];
    assert.ok(Number.isInteger(bytes[word.from]));
    assert.ok(Number.isInteger(bytes[word.to]));
    return [[byte, [bytes[word.from], bytes[word.to]]]];
  });
  return { name, source, providers, categories, words };
}

function columnCases() {
  return ['', 'plain', 'one\ttwo\t\u0000', 'e\u0301\t前', '\t\u0301e',
    'किरण\tकी', '\u0600x', 'cr\r\nlf', '!\u200da\t🧑\u200d💻',
    '🇨🇦🇺🇸🇯\tx', '\ufeff\u0085\u00a0\t\u2028', 'X\u{e0100}\u{1f3fe}Y']
    .flatMap(text => [1, 2, 4, 8].map(tabSize => {
      const bytes = new Map([[0, 0]]);
      let byte = 0, unit = 0;
      for (const ch of text) bytes.set(unit += ch.length, byte += Buffer.byteLength(ch));
      const count = [...bytes].map(([at, offset]) => [offset, countColumn(text, tabSize, at)]);
      const find = Array.from({ length: countColumn(text, tabSize) + 3 }, (_, column) => {
        const byteAt = strict => {
          const offset = findColumn(text, column, tabSize, strict);
          if (offset < 0) return null;
          assert.ok(bytes.has(offset));
          return bytes.get(offset);
        };
        return [column, byteAt(false), byteAt(true)];
      });
      const breaks = tabSize === 1 ? [...bytes].flatMap(([pos, offset]) => [false, true].map(include => {
        const before = bytes.get(findClusterBreak(text, pos, false, include));
        const after = bytes.get(findClusterBreak(text, pos, true, include));
        assert.ok(Number.isInteger(before) && Number.isInteger(after));
        return [offset, include, before, after];
      })) : [];
      return { text, tabSize, count, find, breaks };
    }));
}

const cases = process.argv.includes('--columns') ? columnCases() : [
  record('empty', '', [], [[0, '']]),
  record('default-categories', 'é e\u0301 𐐀²_!\r\n漢\n', [], [
    [0, ''], [0, '\t\u00a0\ufeff\u2028'], [0, '\u0085'], [0, ' \u0301'],
    [0, 'é'], [0, '\u0301'], [0, '\u05b0'], [0, '𐐀'], [0, '²'], [0, '_'], [0, '!?'],
  ]),
  record('ascii-extra-and-scope', 'a-b.c\r\na-b.c', [
    { chars: '', scope: [3, 4] }, { chars: '-', scope: [0, 4] }, { chars: '.' },
  ], [[1, '-'], [1, '.'], [3, '-'], [3, '.'], [4, '.'], [4, '-']]),
  record('bmp-extra-and-whitespace', 'a·b \ufeff !\u0301', [{ chars: '· \ufeff\u0301' }], [
    [0, '·'], [0, ' '], [0, '\ufeff'], [0, ' \u0301'], [0, '!\u0301'], [0, '!'],
  ]),
  record('astral-extra', 'a😁b 🨀c 🀄d 😀e\r\n😁😀', [{ chars: '😀-' }], [
    [0, '😀'], [0, '😁'], [0, '🨀'], [0, '🀄'], [0, '!-'], [0, '!\u200d😁'],
  ]),
  record('non-emoji-zwj', '!-\u200da? !a\u200d-? !\u200d  b', []),
  record('spacing-mark-and-extending', '!ा ?\u0301 e\u0301 !\u05b0 !\u{1e6e3} a\u{e0100} \u0600b', []),
  record('regional-indicator-parity', '🇦🇧🇨🇩🇪 a🇦🇧🇨', [{ chars: '\u{1f5e7}' }]),
];
console.log(JSON.stringify({ pin: { name, version, integrity }, cluster, unicode: process.versions.unicode }));
for (const row of cases) console.log(JSON.stringify(row));
