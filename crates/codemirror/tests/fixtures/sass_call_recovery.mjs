// From the repository root, with Node 22.14+:
// node crates/codemirror/tests/fixtures/sass_call_recovery.mjs <archive-root> \
//   > crates/codemirror/tests/fixtures/sass_call_recovery.jsonl
import assert from 'node:assert/strict';
import {mkdtempSync, realpathSync, rmSync} from 'node:fs';
import {createRequire} from 'node:module';
import {tmpdir} from 'node:os';
import {join, resolve} from 'node:path';
import {projectedOffsets, projectInput} from './editor_line_offsets.mjs';
import {pinnedModules} from './pinned_modules.mjs';

const root = mkdtempSync(join(realpathSync(tmpdir()), 'gpui-sass-call.'));
try {
    const modules = pinnedModules(resolve(process.argv[2] ?? join(root, 'archives')), root);
    const pkg = '@codemirror/lang-sass';
    const exports = await modules.load(pkg);
    assert.deepEqual(Object.keys(exports), ['sass', 'sassCompletionSource', 'sassLanguage']);
    assert.deepEqual(Object.keys(createRequire(join(root, 'probe.cjs'))(pkg)).sort(), Object.keys(exports));
    const {sass, sassCompletionSource, sassLanguage} = exports;
    const {EditorState} = await modules.load('@codemirror/state');
    const {LRLanguage, LanguageSupport, ensureSyntaxTree, foldable, getIndentation,
           language} = await modules.load('@codemirror/language');
    const {CompletionContext} = await modules.load('@codemirror/autocomplete');
    const {highlightTree, tags} = await modules.load('@lezer/highlight');
    const tagNames = new Map(Object.entries(tags).filter(([, tag]) => typeof tag !== 'function')
        .map(([name, tag]) => [tag.id, name]));
    tagNames.set(tags.special(tags.variableName).id, 'variableName.special');
    const highlighter = {style(found) {
        return found.map(tag => {
            assert.ok(tagNames.has(tag.id), `unknown tag ${tag.id}`);
            return tagNames.get(tag.id);
        }).join(' ');
    }};
    // Supply a deterministic DOM snapshot before the first property query.
    const keys = ['zIndex', 'cssText', 'cssFloat', 'hostOnly', 'z-index', 'color'];
    globalThis.document = {body: {style: Object.fromEntries(keys.map(key => [key, '']))}};
    console.log(JSON.stringify({keys}));

    function replay(dialect, marked) {
        const indented = dialect === 'indented';
        const support = sass({indented});
        assert.ok(support instanceof LanguageSupport);
        assert.ok(sassLanguage instanceof LRLanguage);
        assert.equal(sass().language, sassLanguage);
        assert.equal(support.language === sassLanguage, !indented);
        const create = (doc, extensions = support) => EditorState.create({doc, extensions});
        const metadata = create('', support.language.extension);
        assert.equal(metadata.facet(language), support.language);
        assert.equal(support.language.name, 'sass');
        assert.deepEqual(metadata.languageDataAt('autocomplete', 0), []);
        assert.deepEqual(metadata.languageDataAt('commentTokens', 0), [{line: '//', block: {open: '/*', close: '*/'}}]);
        assert.deepEqual(metadata.languageDataAt('wordChars', 0), ['$-']);
        const [indentOnInput] = metadata.languageDataAt('indentOnInput', 0);
        assert.ok(indentOnInput.test('\t}'));
        assert.ok(!indentOnInput.test('} '));

        function snapshot(state, source, points) {
            const {units, byteAt} = projectedOffsets(source);
            const tree = ensureSyntaxTree(state, units, 5000);
            assert.ok(tree);
            const nodes = [], highlights = [], lines = [];
            let depth = 0;
            tree.iterate({enter: n => nodes.push([n.name, byteAt(n.from), byteAt(n.to), depth++]),
                          leave: () => { depth--; }});
            highlightTree(tree, highlighter, (from, to, cls) => highlights.push([byteAt(from), byteAt(to), cls]));
            for (let n = 1; n <= state.doc.lines; n++) {
                const {from, to} = state.doc.line(n);
                const fold = foldable(state, from, to);
                lines.push([getIndentation(state, from), fold ? [byteAt(fold.from), byteAt(fold.to)] : null]);
            }
            const completion = points.map(pos => [byteAt(pos), ...[false, true].map(explicit => {
                const sources = state.languageDataAt('autocomplete', pos);
                assert.deepEqual(sources, [sassCompletionSource]);
                const result = sources[0](new CompletionContext(state, pos, explicit));
                if (!result) return null;
                // FNV-1a over ordered labels, types, and insert text.
                let hash = 2166136261;
                for (const {label, type, apply} of result.options) {
                    for (const byte of Buffer.from(`${label}\0${type}\0${apply ?? ''}\n`)) {
                        hash = Math.imul(hash ^ byte, 16777619);
                    }
                }
                return [byteAt(result.from), result.to === undefined ? null : byteAt(result.to),
                    result.options.length, hash >>> 0, result.options[0]?.type === 'property'];
            })]);
            return {nodes, highlights, lines, completion};
        }

        let source = '';
        const points = [];
        for (const part of marked.split('§')) {
            if (source) points.push(projectedOffsets(source).units);
            source += part;
        }
        const close = projectedOffsets(source.slice(0, source.indexOf(')'))).units;
        let state = create(source);
        function row(name, change) {
            const expected = snapshot(state, source, points);
            const bare = create(source, support.language.extension);
            assert.equal(ensureSyntaxTree(bare, state.doc.length, 5000).toString(),
                ensureSyntaxTree(state, state.doc.length, 5000).toString());
            assert.deepEqual(expected, snapshot(create(source), source, points));
            const at = projectedOffsets(source).byteAt(close);
            assert.deepEqual(expected.nodes.filter(([node]) => node === '⚠'),
                name === 'remove closer' ? [['⚠', at, at, 6]] : []);
            console.log(JSON.stringify({dialect, name, source, change, ...expected}));
        }
        function edit(to, insert) {
            const update = state.update({changes: {from: close, to, insert}});
            const offset = projectedOffsets(source).byteAt(close);
            const end = projectedOffsets(source).byteAt(to);
            source = projectInput(source, update);
            state = update.state;
            for (let n = 0; n < points.length; n++) {
                if (points[n] > close) points[n] += insert.length - (to - close);
            }
            return [offset, end, insert];
        }
        row('complete call');
        row('remove closer', edit(close + 1, ''));
        row('restore closer', edit(close, ')'));
    }

    replay('scss', '// 😀\r\n$tone: red;\r\n.card {\r\n  width: fn(§1px /* value */);\r\n  dis§play: gr§id;\r\n  color: $to§ne;\r\n}');
    replay('indented', '// 😀\r\n$tone: red\r\n.card\r\n  width: fn(§1px)\r\n  dis§play: gr§id\r\n  color: $to§ne\r\n.next\r\n  hei§ght: 2px');
} finally {
    rmSync(root, {recursive: true, force: true});
}
