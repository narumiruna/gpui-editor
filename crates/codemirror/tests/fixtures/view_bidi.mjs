// Generate view_bidi.jsonl with Node 22.15+:
// node view_bidi.mjs <unpacked manifest archives with node_modules> <find-cluster-break-1.0.4.tgz>
// The cluster archive is from https://registry.npmjs.org/@marijn/find-cluster-break/-/find-cluster-break-1.0.4.tgz.
import assert from 'node:assert/strict';
import { execFileSync } from 'node:child_process';
import { createHash } from 'node:crypto';
import { readFileSync, realpathSync } from 'node:fs';
import { registerHooks } from 'node:module';
import { resolve } from 'node:path';
import { pathToFileURL } from 'node:url';

const root = resolve(process.argv[2]);
const { packages } = JSON.parse(readFileSync(new URL('../../../../docs/codemirror-upstream.json', import.meta.url)));
function verify(archive, integrity, dist, installed) {
  assert.equal('sha512-' + createHash('sha512').update(readFileSync(archive)).digest('base64'), integrity);
  assert.deepEqual(execFileSync('tar', ['-xOf', archive, 'package/' + dist]), readFileSync(installed));
  return pathToFileURL(realpathSync(installed)).href;
}
function manifestPackage(name, dist) {
  const { version, integrity } = packages.find(p => p.name === '@codemirror/' + name);
  return verify(resolve(root, `@codemirror__${name}/${name}-${version}.tgz`), integrity,
                dist, resolve(root, `node_modules/@codemirror/${name}/${dist}`));
}
const viewURL = manifestPackage('view', 'dist/index.js');
const stateURL = manifestPackage('state', 'dist/index.js');
verify(process.argv[3], 'sha512-Wy0V7+SGUjnF9/TkiM1hKVDPj7jKXduPNboMVtHTA8dySMURWqfg/JZ9E2Sq8JgSJmkl7k7Qe9FLeMSrSraWmQ==',
       'src/index.js', resolve(root, 'node_modules/@marijn/find-cluster-break/src/index.js'));

// Expose the private line-only motion and its crossed text without changing
// either algorithm. bidiSpans uses the public decoration facet and real state.
const hook = registerHooks({
  load(url, context, nextLoad) {
    const loaded = nextLoad(url, context);
    if (url === viewURL) {
      return { ...loaded, source: loaded.source.toString() + '\nexport { moveVisually, movedOver };\n' };
    }
    return loaded;
  },
});
const v = await import(viewURL);
const { EditorSelection, EditorState } = await import(stateURL);
hook.deregister();

const { LTR, RTL } = v.Direction;
const auto = null;
const prefix = 'é😀\n';
// Input isolate coordinates below are local UTF-16 code units. Inner lists
// describe actual nested decorations. Output and assertions use UTF-8 bytes.
const cases = [
  ['empty-ltr', '', LTR],
  ['empty-rtl', '', RTL],
  ['ltr-non-ascii', 'café 東京 😀', LTR],
  ['rtl-ascii', 'a + 12.3%!', RTL],
  ['rtl-entire-line', 'אבג', LTR],
  ['mixed-hebrew-numbers', 'ab אב 12,34% [גד] z', LTR],
  ['mixed-hebrew-rtl', 'אב (foo) 100$ ? ג', RTL],
  ['mixed-arabic', 'x سَلَام 12,٣٤ + €45 / 67 z', LTR],
  ['bracket-context', 'a (אב [AB] ג) אב (גד) ?', LTR],
  ['unmatched-brackets', 'x {אב ] ג} (z אב', LTR],
  ['class-boundaries', 'é \u0590\u05f4\u05f5 \u06f9\u06fa \u08ac\u08ad \u200c\u200d \ufb50\ufdff\ufe00', RTL],
  ['unsupported-rtl-scripts', '𞤀 𐤀 \u202e ABC \u2067 ما \u2069', LTR],
  ['cluster-mixed', 'e\u0301.[שָׁ]سَ 👩🏾‍💻 🇯🇵🇨🇦', LTR],
  ['cluster-versus-full-uax29', 'abc\u200dxyz का 가 אב', RTL],
  ['explicit-rtl-with-latin-content', 'ab (foo 12) cd', LTR, [[3, 10, RTL]]],
  ['explicit-ltr-in-rtl', 'אב (גד 23) הו', RTL, [[3, 10, LTR]]],
  ['adjacent-isolates', 'a אב גד z', LTR, [[2, 4, RTL], [4, 7, RTL]]],
  ['explicit-nested', 'a אב x12 גד z', LTR, [[2, 11, RTL, [[5, 8, LTR]]]]],
  ['auto-skips-inner', 'A (foo אב) Z', LTR, [[2, 10, auto, [[3, 6, LTR]]]]],
  ['auto-neutral-defaults-ltr', 'אב (12.) גד', RTL, [[3, 8, auto]]],
  ['isolate-after-arabic-number', 'a ع1אבג2 x', LTR, [[4, 6, LTR]]],
  ['rtl-reverse-isolate-edges', 'אב Xגד ab ג D', RTL, [[3, 10, LTR, [[5, 6, RTL]]]]],
  ['neutral-joining-across-isolate', 'a אב - xyz - גד z', LTR, [[7, 10, LTR]]],
  ['bracket-crosses-isolate', 'x אב (x גד) ה v', LTR, [[6, 8, LTR]]],
  ['multibyte-before-isolate', '😀 אב (123) גד', RTL, [[6, 11, LTR]]],
  ['cutoff-4096-utf16', ['😀', 2047, 'אב'], RTL, [], false],
  ['cutoff-4097-utf16', ['😀', 2047, 'אבג'], RTL, [], false],
];

function byteAt(text, pos) {
  assert(!(pos > 0 && (text.charCodeAt(pos) & 0xfc00) === 0xdc00), 'surrogate boundary');
  return Buffer.byteLength(text.slice(0, pos));
}
function generate([id, input, dir, isolates = [], testMotion = true]) {
  const text = typeof input === 'string' ? input : input[0].repeat(input[1]) + input[2];
  const toByte = pos => byteAt(text, pos);
  const marks = [];
  function mark([from, to, direction, inner = []]) {
    marks.push(v.Decoration.mark({ bidiIsolate: direction }).range(prefix.length + from, prefix.length + to));
    inner.forEach(mark);
  }
  isolates.forEach(mark);
  function convert([from, to, dir, inner = []]) {
    return [toByte(from), toByte(to), dir, inner.map(convert)];
  }
  const state = EditorState.create({ doc: prefix + text,
    extensions: [v.EditorView.bidiIsolatedRanges.of(v.Decoration.set(marks, true))] });
  const line = state.doc.line(2);
  const order = v.EditorView.prototype.bidiSpans.call({ state, textDirectionAt: () => dir, bidiCache: [] }, line);
  const spans = order.map(s => [toByte(s.from), toByte(s.to), s.level, s.dir,
    toByte(s.side(false, dir)), toByte(s.side(true, dir))]);
  const finds = [];
  const walks = [];
  function selection(c) {
    return [Buffer.byteLength(prefix) + toByte(c.head - line.from), c.assoc, c.bidiLevel];
  }
  if (testMotion) {
    const boundaries = [...new Set(order.flatMap(s => [s.from, s.to]))].sort((a, b) => a - b);
    for (const pos of boundaries) {
      for (const assoc of [-1, 0, 1]) {
        finds.push([toByte(pos), assoc, null, v.BidiSpan.find(order, pos, -1, assoc)]);
      }
      for (const level of new Set(order.filter(s => s.from <= pos && pos <= s.to).map(s => s.level))) {
        finds.push([toByte(pos), 0, level, v.BidiSpan.find(order, pos, level, 0)]);
      }
    }
    for (const forward of [true, false]) {
      const start = EditorSelection.cursor(forward ? line.from : line.to, forward ? -1 : 1);
      const moves = [];
      const seen = new Set();
      for (let at = start;;) {
        const key = JSON.stringify(selection(at));
        assert(!seen.has(key), id + ': cycle at ' + key);
        seen.add(key);
        const next = v.moveVisually(line, order, dir, at, forward);
        if (!next) break;
        moves.push([...selection(next), v.movedOver]);
        at = next;
      }
      walks.push({ forward, start: selection(start), moves });
    }
  }
  return { id, text: input, prefix, dir, isolates: isolates.map(convert), spans, finds, walks };
}

for (const row of cases) process.stdout.write(JSON.stringify(generate(row)) + '\n');
