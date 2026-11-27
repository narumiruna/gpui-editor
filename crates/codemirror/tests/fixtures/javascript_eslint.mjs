// Run with Node 22.14+ and the pinned archives (including runtime dependencies):
// node crates/codemirror/tests/fixtures/javascript_eslint.mjs <archive-root>
// Add --emit to print the fixture. The verifier only returns literal findings.
import assert from 'node:assert/strict';
import {mkdtempSync, readFileSync, realpathSync, rmSync} from 'node:fs';
import {tmpdir} from 'node:os';
import {join, resolve} from 'node:path';
import {projectedOffsets, projectInput} from './editor_line_offsets.mjs';
import {pinnedModules} from './pinned_modules.mjs';

const root = realpathSync(mkdtempSync(join(tmpdir(), 'javascript-eslint-')));
try {
    const {load, pins} = pinnedModules(resolve(process.argv[2]), root);
    const {esLint, javascript, javascriptLanguage, jsxLanguage, tsxLanguage, typescriptLanguage} =
        await load('@codemirror/lang-javascript');
    const {EditorState} = await load('@codemirror/state');
    const {ensureSyntaxTree} = await load('@codemirror/language');
    const {forEachDiagnostic, setDiagnostics} = await load('@codemirror/lint');
    const {html} = await load('@codemirror/lang-html');
    const {markdown} = await load('@codemirror/lang-markdown');
    const {cssLanguage} = await load('@codemirror/lang-css');
    assert.equal(pins.get('@codemirror/lang-javascript').version, '6.2.5');
    for (const language of [jsxLanguage, typescriptLanguage, tsxLanguage]) {
        assert.equal(language.data, javascriptLanguage.data);
    }
    const languages = {
        none: [], js: javascript(), jsx: jsxLanguage, ts: typescriptLanguage, tsx: tsxLanguage,
        html: html(), markdown: markdown({
            codeLanguages: name => ({js: javascriptLanguage, tsx: tsxLanguage, css: cssLanguage})[name],
        }),
    };
    const ruleEntries = [
        ['recommended', {meta: {docs: {recommended: true}}}],
        ['optional', {meta: {docs: {recommended: false}}}],
        ['undocumented', {meta: {}}],
        ['also-recommended', {meta: {docs: {recommended: true}}}],
    ];
    const recommended = ruleEntries.filter(([, rule]) => rule.meta.docs?.recommended).map(([id]) => id);
    const finding = (message, line, column, extra = {}) => ({message, line, column, severity: 2, ...extra});
    const cases = [
        {mode: 'none', doc: 'const ignored = 1;', findings: []},
        {mode: 'js', doc: '', findings: [[finding('empty document', 1, 1)]]},
        {mode: 'js', doc: 'α😀\rβ\r\nγ\n', rebase: {from: 0, insert: '🐾'}, findings: [[
            finding('through three line endings', 1, 2, {
                endLine: 3, endColumn: 2, ruleId: 'rule', severity: 1, fix: {range: [1, 6], text: 'Q'},
            }),
            finding('point at end of first line', 1, 4, {ruleId: '', severity: 0}),
            finding('point at EOF', 4, 1, {endColumn: 3}),
        ]]},
        {mode: 'jsx', doc: '<p/>', config: {}, findings: [[finding('JSX', 1, 2)]]},
        {mode: 'ts', doc: 'let a: T;', config: {parserOptions: {project: false}, rules: {custom: 1}},
            findings: [[finding('TS', 1, 8, {endLine: 1, endColumn: 9})]]},
        {mode: 'tsx', doc: '<T/>', findings: [[finding('TSX', 1, 2)]]},
        {mode: 'html', doc: '<p>😀</p><script>😀 alpha;\r\nbeta;</script><style>x{}</style>' +
                '<script type="text/typescript">typed</script><script type="application/json">{}</script>',
            rebase: {from: 0, insert: '🐾'}, findings: [[
                finding('first line, fix before finding', 1, 4, {
                    endLine: 1, endColumn: 9, ruleId: 'first', severity: 1, fix: {range: [0, 2], text: 'x'},
                }),
                finding('column one collapses the range', 1, 9, {
                    endLine: 2, endColumn: 1, fix: {range: [9, 10], text: ' '},
                }),
                finding('next line, fix after start', 2, 1, {
                    endLine: 2, endColumn: 5, fix: {range: [11, 12], text: 'E'},
                }),
            ], [finding('typed script', 1, 1, {endLine: 1, endColumn: 6})],
                [finding('HTML JSON script uses a JS expression parser', 1, 1)]]},
        {mode: 'markdown', doc: '> ```js\r\n> first\r\n> second\r\n> ```\r\n\r\n```tsx\r\nlast\r\n```\r\n\r\n```css\r\nignored\r\n```\r\n',
            findings: [[finding('first quote segment', 1, 1)], [finding('second quote segment', 1, 2)],
                [finding('TSX in Markdown', 1, 2, {endLine: 1, endColumn: 4, fix: {range: [0, 4], text: 'next'}})]]},
        {mode: 'markdown', doc: 'Before 😀\r\n\r\n```js\r\nconst x = "😀";\rbad;\r```\nshouldNotLint;\n',
            findings: [[finding('after the lone CR', 2, 1, {
                endLine: 2, endColumn: 4, fix: {range: [16, 19], text: 'good'},
            })]]},
    ];
    const rows = cases.map(test => {
        let state = EditorState.create({doc: test.doc, extensions: languages[test.mode]});
        if (test.mode !== 'none') assert(ensureSyntaxTree(state, state.doc.length, 1000));
        const {byteAt} = projectedOffsets(test.doc);
        const regions = javascriptLanguage.findRegions(state).map(({from, to}) => [byteAt(from), byteAt(to)]);
        assert.equal(regions.length, test.findings.length, test.mode);
        let ruleLookups = 0;
        const calls = [];
        const lint = esLint({
            getRules() { ruleLookups++; return new Map(ruleEntries); },
            verify(input, config) {
                calls.push({input, config});
                return test.findings[calls.length - 1];
            },
        }, test.config);
        const diagnostics = lint({state});
        const payload = diagnostics.map(({from, to, severity, source, message, markClass, actions}) => ({
            range: [byteAt(from), byteAt(to)], severity, source, message, markClass: markClass ?? null,
            actions: (actions ?? []).map(({name}) => name),
        }));
        state = state.update(setDiagnostics(state, diagnostics)).state;
        let raw = test.doc;
        if (test.rebase) {
            const tx = state.update({changes: test.rebase});
            raw = projectInput(raw, tx);
            state = tx.state;
        }
        const fixes = [];
        const current = projectedOffsets(raw);
        forEachDiagnostic(state, (diagnostic, from) => {
            for (const action of diagnostic.actions ?? []) {
                action.apply({dispatch(spec) {
                    const {from, to, insert} = spec.changes;
                    fixes.push({message: diagnostic.message, range: [current.byteAt(from), current.byteAt(to)],
                        insert, scroll: spec.scrollIntoView, after: projectInput(raw, state.update(spec))});
                }}, from);
            }
        });
        assert.equal(fixes.length, diagnostics.filter(diagnostic => diagnostic.actions).length);
        assert.equal(calls.length, regions.length);
        return {...test, regions, recommended, ruleLookups, calls, payload, fixes};
    });
    const actual = rows.map(row => JSON.stringify(row)).join('\n') + '\n';
    if (process.argv.includes('--emit')) process.stdout.write(actual);
    else assert.equal(actual, readFileSync(new URL('./javascript_eslint.jsonl', import.meta.url), 'utf8'));
} finally {
    rmSync(root, {recursive: true, force: true});
}
