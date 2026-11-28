// From the repository root, with Node 22.14+:
// node crates/codemirror/tests/fixtures/less_operand_recovery.mjs <archive-root> \
//   > crates/codemirror/tests/fixtures/less_operand_recovery.jsonl
import assert from 'node:assert/strict';
import {mkdtempSync, realpathSync, rmSync} from 'node:fs';
import {createRequire} from 'node:module';
import {tmpdir} from 'node:os';
import {join, resolve} from 'node:path';
import {projectedOffsets} from './editor_line_offsets.mjs';
import {pinnedModules} from './pinned_modules.mjs';

const root = mkdtempSync(join(realpathSync(tmpdir()), 'gpui-less-operand.'));
try {
    const modules = pinnedModules(resolve(process.argv[2] ?? join(root, 'archives')), root);
    const pkg = '@codemirror/lang-less';
    const exports = await modules.load(pkg);
    assert.deepEqual(Object.keys(exports), ['less', 'lessCompletionSource', 'lessLanguage']);
    assert.deepEqual(Object.keys(createRequire(join(root, 'probe.cjs'))(pkg)).sort(), Object.keys(exports));
    const {less, lessCompletionSource, lessLanguage} = exports;
    const {EditorState} = await modules.load('@codemirror/state');
    const {LRLanguage, LanguageSupport, IndentContext, ensureSyntaxTree, foldable, getIndentation,
           language, syntaxTree} = await modules.load('@codemirror/language');
    const {CompletionContext} = await modules.load('@codemirror/autocomplete');
    const support = less();
    assert.ok(support instanceof LanguageSupport);
    assert.ok(lessLanguage instanceof LRLanguage);
    assert.equal(support.language, lessLanguage);
    assert.equal(lessLanguage.allowsNesting, false);
    const create = doc => EditorState.create({doc, extensions: support});
    const bare = EditorState.create({doc: '@base: 2px;', extensions: lessLanguage.extension});
    assert.equal(bare.facet(language), lessLanguage);
    assert.deepEqual(bare.languageDataAt('autocomplete', 0), []);
    const state = create('@base: 2px;');
    assert.equal(state.facet(language), lessLanguage);
    assert.deepEqual(state.languageDataAt('autocomplete', 0), [lessCompletionSource]);
    const prefixes = ['}', '\t}', '\ufeff}', '\u0085}', ' } ', '.a }', '};'];
    const [onInput] = state.languageDataAt('indentOnInput', 0);
    console.log(JSON.stringify({
        package: pkg, version: modules.pins.get(pkg).version, exports: Object.keys(exports),
        language: lessLanguage.name, comments: state.languageDataAt('commentTokens', 0)[0],
        wordChars: state.languageDataAt('wordChars', 0)[0],
        indentOnInput: prefixes.map(text => [text, onInput.test(text)]),
    }));

    function marked(input) {
        let source = '', at = 0;
        const probes = [];
        for (const match of input.matchAll(/[§¶]/g)) {
            source += input.slice(at, match.index);
            probes.push([match[0], projectedOffsets(source).units]);
            at = match.index + 1;
        }
        return {source: source + input.slice(at), probes};
    }

    function snapshot(state, {source, probes}) {
        const {units, byteAt} = projectedOffsets(source);
        assert.equal(state.doc.toString(), source.replace(/\r\n?/g, '\n'));
        const tree = ensureSyntaxTree(state, units, 5000);
        assert.ok(tree);
        assert.equal(syntaxTree(state), tree);
        const nodes = [];
        let depth = 0;
        tree.iterate({enter: n => nodes.push([n.name, byteAt(n.from), byteAt(n.to), depth++]),
                      leave: () => { depth--; }});
        const folds = [];
        for (let i = 1; i <= state.doc.lines; i++) {
            const {from, to} = state.doc.line(i);
            const fold = foldable(state, from, to);
            folds.push([byteAt(from), byteAt(to), fold ? [byteAt(fold.from), byteAt(fold.to)] : null]);
        }
        const indents = probes.filter(([marker]) => marker === '§').map(([, pos]) => [
            byteAt(pos), getIndentation(state, pos), ...[false, true].map(simulateDoubleBreak =>
                getIndentation(new IndentContext(state, {simulateBreak: pos, simulateDoubleBreak}), pos)),
        ]);
        const completions = probes.map(([, pos]) => [byteAt(pos), ...[false, true].map(explicit => {
            const sources = state.languageDataAt('autocomplete', pos);
            assert.deepEqual(sources, [lessCompletionSource]);
            const result = sources[0](new CompletionContext(state, pos, explicit));
            if (!result) return null;
            // FNV-1a over ordered labels, types, and insert text. The selected
            // probes use the static keyword and variable lists, not DOM properties.
            assert.ok(result.options.length && result.options.every(option => option.type !== 'property'));
            let hash = 2166136261;
            for (const {label, type, apply} of result.options) {
                for (const byte of Buffer.from(`${label}\0${type}\0${apply ?? ''}\n`)) {
                    hash = Math.imul(hash ^ byte, 16777619);
                }
            }
            return [byteAt(result.from), result.to === undefined ? null : byteAt(result.to),
                    result.options.length, hash >>> 0];
        })]);
        return {nodes, folds, indents, completions};
    }

    const expression = '// 😀\r\n@base: 2px;\r\n\t.a {\r\n\t\twidth: (@base + 1px\r§);\n\t\tcolor: @ba¶;\n\t}';
    const initial = marked(expression);
    let editedState = create(initial.source);
    function replay(name, input, change) {
        const expected = snapshot(editedState, input);
        assert.deepEqual(expected, snapshot(create(input.source), input), name);
        console.log(JSON.stringify({name, source: input.source, change, ...expected}));
    }
    replay('complete arithmetic with mixed line endings', initial);
    const before = initial.source.slice(0, initial.source.indexOf(' 1px'));
    const {units: from, byteAt} = projectedOffsets(before);
    const cut = byteAt(from);
    editedState = editedState.update({changes: {from, to: from + 4}}).state;
    replay('delete the right operand before a real closer', marked(expression.replace(' 1px', '')),
        [cut, cut + 4, '']);
    editedState = editedState.update({changes: {from, insert: ' 1px'}}).state;
    replay('restore the right operand', initial, [cut, cut, ' 1px']);

    for (const [name, input] of [
        ['argument of a Less function call', '@base: 2px;\n  .a {\n    width: fn(@base +§);\n    color: @ba¶;\n  }'],
        ['comparison in a guarded mixin', '@base: 2px;\n  .m(@x) when (@x > §) {\n    color: @ba¶;\n  }'],
        ['multiply in a parenthesized value', '  .a {\n    width: (@base *§);\n    opacity: 1;\n  }'],
        ['unfinished arithmetic and block at EOF', '  .a {\n    width: (@base +\n§'],
        ['missing closers without a missing operand', '  .a {\n    width: (@base + 1px\n§'],
    ]) {
        const data = marked(input);
        console.log(JSON.stringify({name, source: data.source, ...snapshot(create(data.source), data)}));
    }
} finally {
    rmSync(root, {recursive: true, force: true});
}
