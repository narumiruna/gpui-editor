// Verify css_recovery.json with Node 22.14+ and headless Chromium:
// node crates/codemirror/tests/fixtures/css_recovery.mjs <archive-root> <chromium-executable>
import assert from 'node:assert/strict';
import {execFileSync} from 'node:child_process';
import {mkdtempSync, readFileSync, realpathSync, rmSync, writeFileSync} from 'node:fs';
import {tmpdir} from 'node:os';
import {join, resolve} from 'node:path';
import {pathToFileURL} from 'node:url';
import {pinnedModules} from './pinned_modules.mjs';

const root = realpathSync(mkdtempSync(join(tmpdir(), 'css-recovery-')));
try {
    const {install, installed} = pinnedModules(resolve(process.argv[2]), root);
    install('@codemirror/lang-css');
    const imports = Object.fromEntries([...installed].map(([name, {url}]) => [name, url.href]));
    const expected = JSON.parse(readFileSync(new URL('css_recovery.json', import.meta.url)));
    const inputs = expected.map(({mode, input}) => ({mode, input}));
    const page = join(root, 'run.html');
    const script = String.raw`
import {css, cssLanguage, cssCompletionSource} from '@codemirror/lang-css';
import {CompletionContext} from '@codemirror/autocomplete';
import {foldable, getIndentation, syntaxTree} from '@codemirror/language';
import {EditorState} from '@codemirror/state';
import {highlightTree, tagHighlighter, tags} from '@lezer/highlight';

const encoder = new TextEncoder();
const highlighter = tagHighlighter([
    ...Object.entries(tags).filter(([, tag]) => tag.id != null).map(([cls, tag]) => ({tag, class: cls})),
    {tag: tags.constant(tags.className), class: 'className.constant'},
]);
const support = css();
const results = inputs.map(({mode, input}) => {
    const pos = input.indexOf('§'), raw = input.replace('§', '');
    const at = unit => encoder.encode(raw.slice(0, unit)).length;
    const language = mode === 'styles' ? [cssLanguage.configure({top: 'Styles'}), support.support] : support;
    const state = EditorState.create({doc: raw, extensions: [language, EditorState.lineSeparator.of('\n')]});
    const tree = syntaxTree(state), nodes = [], highlights = [], lines = [];
    let depth = 0;
    tree.iterate({enter: node => nodes.push([node.name, at(node.from), at(node.to), depth++]),
        leave: () => { depth--; }});
    highlightTree(tree, highlighter, (from, to, tag) => highlights.push([at(from), at(to), tag]));
    for (let i = 1; i <= state.doc.lines; i++) {
        const line = state.doc.line(i), fold = foldable(state, line.from, line.to);
        lines.push([getIndentation(state, line.from), fold && [at(fold.from), at(fold.to)]]);
    }
    const sources = state.languageDataAt('autocomplete', pos);
    if (sources.length !== 1 || sources[0] !== cssCompletionSource) throw Error('unexpected CSS source');
    const completions = [false, true].map(explicit => {
        const result = sources[0](new CompletionContext(state, pos, explicit));
        if (!result) return null;
        let hash = 2166136261;
        for (const {label, type, apply} of result.options) {
            for (const byte of encoder.encode(label + '\0' + type + '\0' + (apply ?? '') + '\n')) {
                hash = Math.imul(hash ^ byte, 16777619) >>> 0;
            }
        }
        return [at(result.from), result.options[0]?.type ?? 'empty', result.options.length, hash];
    });
    return {mode, input, nodes, highlights, lines, completions};
});
document.body.textContent = btoa(String.fromCharCode(...encoder.encode(JSON.stringify(results))));
`;
    const safeJson = value => JSON.stringify(value).replaceAll('<', '\\u003c');
    writeFileSync(page, '<!doctype html><meta charset="utf-8"><body>' +
        `<script type="importmap">${safeJson({imports})}</script>` +
        `<script type="module">const inputs=${safeJson(inputs)};${script}</script></body>`);
    const browser = resolve(process.argv[3]);
    const html = execFileSync(browser, [
        '--headless', '--no-first-run', '--disable-background-networking', '--disable-component-update',
        '--allow-file-access-from-files', '--dump-dom', '--virtual-time-budget=5000',
        `--user-data-dir=${join(root, 'profile')}`, pathToFileURL(page).href,
    ], {encoding: 'utf8', timeout: 30000, stdio: ['ignore', 'pipe', 'pipe']});
    const encoded = html.match(/<body>([A-Za-z0-9+/=]+)<\/body>/)?.[1];
    assert.ok(encoded, 'missing browser output');
    const actual = JSON.parse(Buffer.from(encoded, 'base64'));
    assert.deepEqual(actual, expected);
    console.log(`${actual.length} pinned CSS cases matched`);
} finally {
    rmSync(root, {recursive: true, force: true});
}
