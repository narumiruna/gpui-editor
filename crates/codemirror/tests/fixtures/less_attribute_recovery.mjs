// From the repository root, with Node 22.14+:
// node crates/codemirror/tests/fixtures/less_attribute_recovery.mjs <archive-root> \
//   > crates/codemirror/tests/fixtures/less_attribute_recovery.jsonl
import assert from 'node:assert/strict';
import {mkdtempSync, realpathSync, rmSync} from 'node:fs';
import {tmpdir} from 'node:os';
import {join, resolve} from 'node:path';
import {projectedOffsets} from './editor_line_offsets.mjs';
import {pinnedModules} from './pinned_modules.mjs';

const root = mkdtempSync(join(realpathSync(tmpdir()), 'gpui-less-attribute.'));
try {
    const modules = pinnedModules(resolve(process.argv[2]), root);
    const {less, lessCompletionSource} = await modules.load('@codemirror/lang-less');
    const {EditorState} = await modules.load('@codemirror/state');
    const {ensureSyntaxTree, getIndentation, IndentContext} = await modules.load('@codemirror/language');
    const {CompletionContext} = await modules.load('@codemirror/autocomplete');
    const {highlightTree, tagHighlighter, tags} = await modules.load('@lezer/highlight');
    const highlighter = tagHighlighter([
        ...Object.entries(tags).filter(([, tag]) => tag.id != null).map(([cls, tag]) => ({tag, class: cls})),
        {tag: tags.constant(tags.className), class: 'className.constant'},
        {tag: tags.special(tags.variableName), class: 'variableName.special'},
    ]);
    const support = less();
    const create = doc => EditorState.create({doc, extensions: support});

    function snapshot(state, source) {
        const {units, byteAt} = projectedOffsets(source);
        const doc = state.doc.toString();
        assert.equal(doc, source.replace(/\r\n?/g, '\n'));
        const tree = ensureSyntaxTree(state, units, 5000);
        assert.ok(tree);
        const nodes = [], highlights = [];
        let depth = 0;
        tree.iterate({enter: n => nodes.push([n.name, byteAt(n.from), byteAt(n.to), depth++]),
                      leave: () => { depth--; }});
        highlightTree(tree, highlighter, (from, to, style) => highlights.push([byteAt(from), byteAt(to), style]));
        const indents = [...doc.matchAll(/\n(?= *(?:[\w-]+: @ba;|}))/g)].map(({index}) => {
            const pos = index + 1;
            return [byteAt(pos), getIndentation(state, pos), ...[false, true].map(simulateDoubleBreak =>
                getIndentation(new IndentContext(state, {simulateBreak: pos, simulateDoubleBreak}), pos))];
        });
        const completions = [...doc.matchAll(/(?<=: )@ba(?=;)/g)].flatMap(({index}) =>
            [index, index + 3].map(pos => [byteAt(pos), ...[false, true].map(explicit => {
                const result = lessCompletionSource(new CompletionContext(state, pos, explicit));
                if (!result) return null;
                // The probes use static values and variable names, not DOM properties.
                assert.ok(result.options.length && result.options.every(option => option.type !== 'property'));
                let hash = 2166136261;
                for (const {label, type, apply} of result.options) {
                    for (const byte of Buffer.from(`${label}\0${type}\0${apply ?? ''}\n`)) {
                        hash = Math.imul(hash ^ byte, 16777619);
                    }
                }
                return [byteAt(result.from), result.to === undefined ? null : byteAt(result.to),
                        result.options.length, hash >>> 0];
            })]));
        return {nodes, highlights, indents, completions};
    }

    for (const [name, marked] of [
        ['delete the closer before a rule block',
         '@base: 1;\n.a[data-x="x"¦] {\n  background: @ba;\n}\n.b {\n  color: @ba;\n}'],
        ['nested child and chained selectors with quote and block comment',
         ['// 😀', '@base: 1;', '.shell {',
          String.raw`  &:hover > .a[role][data-x="x\" { ] // y"¦] /* [ ] { */ {`,
          '    width: @ba;', '  }', '  color: @ba;', '}'].join('\r\n')],
        ['nested attribute with an unquoted URL literal',
         '@base: 1;\n.shell {\n  .a[data-x=url(https://x/[{a])¦] {\n    width: @ba;\n  }\n}'],
        ['presence selector followed by a line comment',
         '@base: 1;\n.a[data-x¦] // [ " ] {\n{\n  color: @ba;\n}'],
        ['valid nested subscript brackets and selector',
         '@base: 1;\n.shell {\n  .a[data-x=@theme[key]] {\n    width: @ba;\n  }\n}'],
        ['closed selector with a missing right operand in its block',
         '@base: 1;\n.a[data-x="x"] {\n  width: (@base +);\n  color: @ba;\n}'],
    ]) {
        const cut = marked.indexOf('¦');
        const source = marked.replace('¦', '');
        const base = create(source);
        const before = snapshot(base, source);
        if (cut === -1) {
            console.log(JSON.stringify({name, source, before}));
            continue;
        }
        assert.equal(source[cut], ']');
        const {units: from, byteAt} = projectedOffsets(source.slice(0, cut));
        const after = source.slice(0, cut) + source.slice(cut + 1);
        const editedState = base.update({changes: {from, to: from + 1}}).state;
        const edited = snapshot(editedState, after);
        assert.deepEqual(edited, snapshot(create(after), after), name);
        const errors = edited.nodes.filter(n => n[0] === '⚠');
        assert.equal(errors.length, 1, name);
        const [, at, end] = errors[0];
        assert.equal(at, end, name);
        assert.equal(Buffer.from(after)[at], 123, name);
        const restored = editedState.update({changes: {from, insert: ']'}}).state;
        assert.deepEqual(snapshot(restored, source), before, name);
        console.log(JSON.stringify({name, source, before, change: [byteAt(from), byteAt(from) + 1], edited}));
    }
} finally {
    rmSync(root, {recursive: true, force: true});
}
