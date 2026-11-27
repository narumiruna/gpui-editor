// Run `node language_data_oracle.mjs <pinned-package-directory>` to generate
// language_data_custom_names.jsonl. The directory must contain the tarballs,
// extracted packages, and pinned node_modules links.
import assert from 'node:assert/strict';
import { execFileSync } from 'node:child_process';
import { createHash } from 'node:crypto';
import { readFileSync } from 'node:fs';
import { resolve } from 'node:path';
import { pathToFileURL } from 'node:url';

const manifest = JSON.parse(readFileSync(new URL('../../../../docs/codemirror-upstream.json', import.meta.url)));
async function pinned(name, version) {
  const pin = manifest.packages.find(entry => entry.name === `@codemirror/${name}`);
  assert.equal(pin.version, version);
  const dir = resolve(process.argv[2], `@codemirror__${name}`);
  const tarball = resolve(dir, `${name}-${version}.tgz`);
  const digest = createHash('sha512').update(readFileSync(tarball)).digest('base64');
  assert.equal(`sha512-${digest}`, pin.integrity);
  const dist = 'package/dist/index.js';
  const file = resolve(dir, dist);
  assert.deepEqual(readFileSync(file), execFileSync('tar', ['-xOf', tarball, dist]));
  return import(pathToFileURL(file));
}

const { LanguageDescription: Description, LanguageSupport } = await pinned('language', '6.13.1');
const { languages } = await pinned('language-data', '6.5.2');
const stock = JSON.parse(readFileSync(new URL('../../src/language_data/oracle.json', import.meta.url)));
const id = (catalog, match) => match === null ? null : catalog.indexOf(match);
assert.deepEqual(languages.map(d => [d.name, d.alias, d.extensions,
  d.filename ? [d.filename.source, d.filename.flags] : null]), stock.catalog.map(row => row.slice(0, 4)));
for (const [file, expected] of stock.filenames) {
  assert.equal(id(languages, Description.matchFilename(languages, file)), expected, file);
}
for (const [name, fuzzy, expected] of stock.names) {
  assert.equal(id(languages, Description.matchLanguageName(languages, name, fuzzy)), expected, name);
}

await Promise.all(languages.map(async d => {
  assert.equal(d.support, undefined, d.name);
  const pending = d.load();
  assert.equal(d.load(), pending, d.name);
  assert.equal(d.support, undefined, d.name);
  const loaded = await pending;
  assert.ok(loaded instanceof LanguageSupport, d.name);
  assert.equal(d.support, loaded, d.name);
  assert.equal(d.load(), pending, d.name);
}));

const support = languages[0].support;
const extensionFirst = [Description.of({ name: 'Extension', extensions: ['conf'], support }),
  languages.find(d => d.name === 'Asterisk')];
for (const [file, expected] of stock.patternPriority) {
  assert.equal(id(extensionFirst, Description.matchFilename(extensionFirst, file)), expected, file);
}

// A preloaded description can have its own loader. Preloaded support is visible
// before and during either attempt, and a failed load does not clear it.
let attempt;
let calls = 0;
const seeded = Description.of({
  name: 'Seeded', support,
  load() {
    calls++;
    return new Promise((resolve, reject) => { attempt = { resolve, reject }; });
  },
});
assert.equal(seeded.support, support);
assert.equal(calls, 0);
const first = seeded.load();
const waiter = seeded.load();
assert.equal(first, waiter);
assert.equal(calls, 1);
assert.equal(seeded.support, support);
const error = new Error('first');
const failures = Promise.allSettled([first, waiter]);
attempt.reject(error);
for (const result of await failures) {
  assert.equal(result.status, 'rejected');
  assert.equal(result.reason, error);
}
assert.equal(seeded.support, support);
const retry = seeded.load();
const other = seeded.load();
assert.notEqual(retry, first);
assert.equal(retry, other);
assert.equal(calls, 2);
assert.equal(seeded.support, support);
const loaded = new LanguageSupport(support.language);
attempt.resolve(loaded);
assert.equal(await retry, loaded);
assert.equal(await other, loaded);
assert.equal(seeded.support, loaded);
assert.equal(seeded.load(), retry);
assert.equal(calls, 2);

// Use Description.of so custom aliases get the same normalization as stock ones.
const custom = [
  { name: 'Tool', alias: ['GO', '𐐀x'] },
  { name: '(Go)' },
  { name: 'Go' },
  { name: '😀' },
  { name: 'ΣΙΓΜΑΣ', alias: ['ΟΣ'] },
  { name: 'İ', alias: ['I'] },
  { name: 'Empty', alias: [''] },
].map(spec => Description.of({ ...spec, support }));
for (const name of ['TOOL', 'go', '(go)', '[GO]', '!go', 'go!', 'go_ (go)', '中goé',
  '𐐀X', 'a𐐀Xb', '😀', 'x😀y', 'é😀中', '😀!', '!😀', 'ΣΙΓΜΑΣ', 'ΟΣ', '[ΟΣ]',
  '[οσ]', 'İ', '[İ]', 'I', 'i', '', '!']) {
  const fuzzyMatch = Description.matchLanguageName(custom, name, true);
  assert.equal(Description.matchLanguageName(custom, name), fuzzyMatch);
  console.log(JSON.stringify([name, false, id(custom, Description.matchLanguageName(custom, name, false))]));
  console.log(JSON.stringify([name, true, id(custom, fuzzyMatch)]));
}
