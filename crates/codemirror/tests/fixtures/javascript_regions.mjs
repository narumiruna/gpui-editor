// Run with Node 22.14+ and the local pinned archives, including runtime dependencies:
// node crates/codemirror/tests/fixtures/javascript_regions.mjs <archive-root> [--emit]
// This oracle parses text. It does not run the source or call a lint verifier.
import assert from 'node:assert/strict';
import {existsSync, mkdtempSync, readFileSync, realpathSync, rmSync} from 'node:fs';
import {tmpdir} from 'node:os';
import {join, resolve} from 'node:path';
import {projectedOffsets} from './editor_line_offsets.mjs';
import {pinnedModules} from './pinned_modules.mjs';

const archives = resolve(process.argv[2]);
const root = realpathSync(mkdtempSync(join(tmpdir(), 'javascript-regions-')));
try {
    const {load, pins} = pinnedModules(archives, root);
    // Check the pinned dependency set before loading, so missing archives fail offline.
    for (const name of [
        '@codemirror/autocomplete', '@codemirror/lang-css', '@codemirror/lang-html',
        '@codemirror/lang-javascript', '@codemirror/lang-json', '@codemirror/lang-markdown',
        '@codemirror/language', '@codemirror/lint', '@codemirror/state', '@codemirror/streamparser',
        '@codemirror/view', '@lezer/common', '@lezer/css', '@lezer/highlight', '@lezer/html',
        '@lezer/javascript', '@lezer/json', '@lezer/lr', '@lezer/markdown', '@marijn/find-cluster-break',
        'crelt', 'style-mod', 'w3c-keyname',
    ]) {
        const file = join(archives, name.replace('/', '__'), `${name.split('/').at(-1)}-${pins.get(name).version}.tgz`);
        assert(existsSync(file), `missing local archive: ${file}`);
    }
    const {javascriptLanguage: js, jsxLanguage: jsx, typescriptLanguage: ts, tsxLanguage: tsx} =
        await load('@codemirror/lang-javascript');
    const {html} = await load('@codemirror/lang-html');
    const {markdown} = await load('@codemirror/lang-markdown');
    const {jsonLanguage: json} = await load('@codemirror/lang-json');
    const {cssLanguage: css} = await load('@codemirror/lang-css');
    const {EditorState} = await load('@codemirror/state');
    const {ensureSyntaxTree} = await load('@codemirror/language');
    for (const language of [jsx, ts, tsx]) assert.equal(language.data, js.data);
    assert.notEqual(json.data, js.data);

    const htmlSupport = html();
    const htmlSource = htmlSupport.language.parser.parse('<script type="application/json">{}</script>');
    assert(htmlSource.topNode.getChild('Element').getChild('SingleExpression'));

    const code = {js, javascript: js, jsx, ts, tsx, css, json, html: htmlSupport.language};
    const modes = {
        js, jsx, ts, tsx, json, html: htmlSupport,
        markdown: markdown({codeLanguages: name => code[name]}), markdownDefault: markdown(),
    };
    // Dialect labels describe the native parser. Null marks a pinned region
    // whose source range the native query cannot return safely.
    const cases = [
        ['js', '', ['js']],
        ['js', 'α😀\rβ\r\nγ\n', ['js']],
        ['jsx', '<p/>', ['jsx']],
        ['ts', 'let a: T;', ['ts']],
        ['tsx', '<T/>', ['tsx']],
        ['json', '{"x":"😀"}', []],
        ['html', '<p>😀</p><script>😀 alpha;\r\nbeta;</script><style>x{}</style>' +
            '<script type="text/typescript">typed</script><script type="application/json">{}</script>',
            ['js', 'ts', 'js']],
        ['html', '<p onclick="😀\r\nrun()" onblur=hide() ONCLICK="no()" onload="" style="color:red"></p>',
            ['js', 'js']],
        ['html', '<script></script><script>\r\n</script><script type="x-unknown">ignored</script>' +
            '<script type="importmap">{}</script><script type="APPLICATION/LD+JSON">[]</script>' +
            '<script lang="ts" type="text/babel">a</script><script type="text/babel">b</script>' +
            '<script type="text/typescript-jsx">c</script><script type="module">d</script>',
            ['js', 'js', 'js', 'js', 'ts', 'jsx', 'tsx', 'js']],
        ['html', '<script>let x =', ['js']],
        ['html', '<p onclick="let x =', [null]],
        ['markdown', '> ```js\r\n> first\r\n> second\r\n> ```\r\n\r\n```tsx\r\nlast\r\n```\r\n\r\n```css\r\nignored\r\n```\r\n',
            ['js', 'js', 'tsx']],
        ['markdown', '> ```js\n> one\n>\n> two\n> ```\n', ['js', 'js', 'js']],
        ['markdown', '```js\n```\n\n```js\n\n```\n\n```json\n{}\n```\n\n```unknown\nx\n```', ['js']],
        ['markdown', '```javascript extra\r\n😀;\r\nsecond;\r\n```\r\n\r\n<p onclick="open()">text</p>\r\n\r\n' +
            '<script>\r\nrun();\r\n</script>\r\n\r\ninline <b onblur="hide()">bold</b>\r\n',
            ['js', 'js', 'js', 'js']],
        ['markdownDefault', '```js\nx\n```\n\n<script>x;</script>', ['js']],
        ['markdown', '> <script>one;</script>\r\n', ['js']],
        ['markdown', '```html\r\n<script>\r\nrun()\r\n</script>\r\n```\r\n', ['js']],
        ['markdown', '> <script>\r\n> first;\r\n> second;\r\n> </script>\r\n', [null]],
        ['markdown', '> ```html\r\n> <script>\r\n> run()\r\n> </script>\r\n> ```\r\n', [null]],
        ['markdown', '```js\rlet a;\r```\r', ['js']],
        ['markdown', '```js\nfirst;\r```\nshouldNotLint;\n```\n', ['js']],
        ['markdown', '> ```js\r> 😀 first;\r\n> second;\r> ```\r\r```tsx\r<T/>\r```', ['js', 'js', 'tsx']],
        ['markdown', '```html\r\n<script>😀;\rnext;</script>\r\n```', ['js']],
        ['markdown', '> <script>\r> first;\r> second;\r> </script>', [null]],
    ];
    const rows = cases.map(([mode, doc, dialects]) => {
        const state = EditorState.create({doc, extensions: modes[mode]});
        assert(ensureSyntaxTree(state, state.doc.length, 1000));
        const {byteAt} = projectedOffsets(doc);
        const ranges = js.findRegions(state);
        assert.equal(ranges.length, dialects.length, `${mode}: ${doc}`);
        const bytes = Buffer.from(doc);
        const regions = ranges.map(({from, to}, i) => {
            const range = [byteAt(from), byteAt(to)];
            return {range, raw: bytes.subarray(...range).toString(), dialect: dialects[i]};
        });
        return JSON.stringify({mode, doc, regions});
    });
    const actual = rows.join('\n') + '\n';
    if (process.argv.includes('--emit')) process.stdout.write(actual);
    else assert.equal(actual, readFileSync(new URL('./javascript_regions.jsonl', import.meta.url), 'utf8'));
} finally {
    rmSync(root, {recursive: true, force: true});
}
