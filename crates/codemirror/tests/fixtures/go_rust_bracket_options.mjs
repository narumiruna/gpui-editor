// Run with Node and the directory of SHA-512-pinned archives. JSONL goes to stdout.
import assert from 'node:assert/strict';
import {mkdtempSync, realpathSync, rmSync} from 'node:fs';
import {tmpdir} from 'node:os';
import {join, resolve} from 'node:path';
import {projectedOffsets} from './editor_line_offsets.mjs';
import {pinnedModules} from './pinned_modules.mjs';

const bmp = {brackets: '«»'};
const markedOnly = {...bmp, maxScanDistance: 1};
const reverse = {brackets: ')('};
const mixed = {brackets: '()[]{}«»', maxScanDistance: 1};
const caps = '// ¦«😀\r\n// ¦»';
const cases = [
    ['go', 'marked parameters and nested pairs ignore custom list and cap', 'func f¦[T any]¦(x T¦) ¦{ g¦([...]T{x});\r\n¦}', markedOnly],
    ['go', 'configured comma pairs with a named sibling', 'func f(a int¦, b int¦) {}', {brackets: ',)'}],
    ['go', 'repeated comma increases sibling depth', 'func f(a int¦, b int¦, c int¦) {}', {brackets: ',)'}],
    ['go', 'opposite configured name does not replace grammar prop', 'func f¦(a int¦) {}', reverse],
    ['go', 'raw and interpreted strings use one term', 'var a = "¦«😀"; var b = `¦»`', bmp],
    ['go', 'line comment and raw string do not pair', '// ¦«😀\r\nvar b = `¦»`', bmp],
    ['go', 'term identity separates additive and multiplicative operators', 'var _ = 1 ¦+ 2 ¦* 3', {brackets: '+*'}],
    ['go', 'the same additive term pairs under distinct parents', 'var _ = ¦+1 ¦- 2', {brackets: '+-'}],
    ['go', 'exact UTF-16 cap forward', caps, {...bmp, maxScanDistance: 4}],
    ['go', 'UTF-16 cutoff forward', caps, {...bmp, maxScanDistance: 3}],
    ['go', 'retry after cutoff', '/*¦(¦«😀»¦\r\n)*/', mixed],
    ['go', 'before-only disables retry', '/*(¦«😀»¦\r\n)*/', {...mixed, afterCursor: false}],
    ['go', 'empty pair list and zero limit use defaults', 'func f¦() { f¦()¦; s := `¦[😀]¦` }', {brackets: '', maxScanDistance: 0, afterCursor: true}],
    ['go', 'bounded missing closer does not pair with string literal', 'func f() { f¦("¦)"', bmp],
    ['rust', 'marked params and blocks ignore custom list and cap', 'fn f¦(x: [u8; 1]¦) ¦{ g¦([x]);\r\n¦}', markedOnly],
    ['rust', 'angles only get matching from configured names', 'fn f¦<T¦>(x: Vec¦<Vec¦<T¦>¦>¦) {}', {brackets: '<>'}],
    ['rust', 'angle names are not default brackets', 'fn f¦<T¦>(x: Vec¦<T¦>) {}', {}],
    ['rust', 'configured pipe reaches a sibling by name', 'fn f() { let f = ¦|x¦, y¦| x; }', {brackets: '|,'}],
    ['rust', 'configured named siblings track depth', 'fn f(x: i32¦, y: i32¦, z: i32¦) {}', {brackets: ',)'}],
    ['rust', 'opposite configured name does not replace grammar prop', 'fn f¦(x: i32¦) {}', reverse],
    ['rust', 'raw and normal strings do not pair', 'fn f() { g("¦«😀", r#"¦»"#); }', bmp],
    ['rust', 'macro token groups and raw strings keep their own matches', 'fn f() { dbg!¦(¦[1]; r#"¦«😀»¦"#; br#"¦«»"#¦); }', markedOnly],
    ['rust', 'byte and normal strings share a term', 'fn f() { f(b"¦«", "¦»"); }', bmp],
    ['rust', 'term identity separates unary and binary operators', 'fn f() { let x = ¦-1 ¦+ 2; }', {brackets: '-+'}],
    ['rust', 'exact UTF-16 cap backward', caps, {...bmp, maxScanDistance: 5}],
    ['rust', 'UTF-16 cutoff backward', caps, {...bmp, maxScanDistance: 4}],
    ['rust', 'after-cursor and adjacent delimiters at Unicode boundaries', 'fn f() { g("😀¦«»¦«¦»字"); ¦[¦]; }', mixed],
    ['rust', 'before-only prevents right hit across Unicode boundary', 'fn f() { g("😀¦«»¦«¦»字"); ¦[¦]; }', {...mixed, afterCursor: false}],
    ['rust', 'explicit defaults for punctuation and raw tokens', 'fn f¦() { m!¦()¦; let x = r#"¦[😀]¦"#; }', {brackets: '()[]{}', maxScanDistance: 10000, afterCursor: true}],
    ['rust', 'bounded missing closer does not pair with raw string', 'fn f() { f¦(r#"¦)"#', bmp],
];

const props = {
    go: [[13, ')', ['('], null], [14, '(', null, [')']], [25, '}', ['{'], null],
         [26, '{', null, ['}']], [53, ']', ['['], null], [54, '[', null, [']']]],
    rust: [[10, ']', ['['], null], [41, '}', ['{'], null], [42, '{', null, ['}']],
           [48, '(', null, [')']], [50, ')', ['('], null], [85, '[', null, [']']]],
};

const scratch = mkdtempSync(join(realpathSync(tmpdir()), 'gpui-go-rust-brackets.'));
try {
    const {load} = pinnedModules(resolve(process.argv[2]), scratch);
    const {NodeProp} = await load('@lezer/common');
    const {bracketMatching, bracketMatchingHandle, forceParsing, matchBrackets, syntaxTree} = await load('@codemirror/language');
    const {EditorState} = await load('@codemirror/state');
    const {ViewPlugin} = await load('@codemirror/view');
    const supports = {};
    for (const mode of ['go', 'rust']) {
        const language = await load('@codemirror/lang-' + mode);
        const {parser} = await load('@lezer/' + mode);
        supports[mode] = language[mode];
        for (const p of [parser, language[mode + 'Language'].parser]) {
            const actual = p.nodeSet.types.flatMap(type => {
                assert.equal(type.prop(bracketMatchingHandle), undefined);
                const opened = type.prop(NodeProp.openedBy), closed = type.prop(NodeProp.closedBy);
                return opened || closed ? [[type.id, type.name, opened ?? null, closed ?? null]] : [];
            });
            assert.deepEqual(actual, props[mode]);
        }
    }
    for (const [mode, name, template, options] of cases) {
        const parts = template.split('¦');
        const source = parts.join('');
        const {units, byteAt} = projectedOffsets(source);
        const range = ({from, to}) => [byteAt(from), byteAt(to)];
        let rendered;
        const create = opts => {
            const matching = bracketMatching(opts);
            const plugin = matching.flat(Infinity).find(extension => extension instanceof ViewPlugin);
            assert(plugin);
            const view = {state: EditorState.create({doc: source, extensions: [supports[mode](), matching]}), dispatch(spec) {
                this.state = this.state.update(spec).state;
            }};
            assert(forceParsing(view, units, 5000));
            return [view.state, anchor => plugin.create({state: view.state.update({selection: {anchor}}).state})];
        };
        const [state, raw] = create({...options, renderMatch(hit) {
            assert.equal(rendered, null);
            rendered = hit;
            return [];
        }});
        const [, stock] = create(options);
        const snapshot = hit => hit ? [range(hit.start), hit.end ? range(hit.end) : null, hit.matched] : null;
        const checks = [];
        let prefix = '';
        for (const part of parts.slice(0, -1)) {
            prefix += part;
            const anchor = projectedOffsets(prefix).units;
            const hit = matchBrackets(state, anchor, -1, options)
                || (anchor > 0 && matchBrackets(state, anchor - 1, 1, options))
                || (options.afterCursor !== false && (matchBrackets(state, anchor, 1, options)
                    || (anchor < units && matchBrackets(state, anchor + 1, -1, options))));
            const expected = snapshot(hit);
            const label = `${mode}: ${name} at ${anchor}`;
            rendered = null;
            raw(anchor);
            assert.deepEqual(snapshot(rendered), expected, label);
            const marks = [];
            for (const iter = stock(anchor).decorations.iter(); iter.value; iter.next()) {
                const kind = iter.value.spec.class;
                assert(kind === 'cm-matchingBracket' || kind === 'cm-nonmatchingBracket');
                marks.push([...range(iter), kind === 'cm-matchingBracket']);
            }
            const spans = expected ? [expected[0], expected[1]].filter(Boolean).map(span => [...span, expected[2]]) : [];
            assert.deepEqual(marks, spans.sort((a, b) => a[0] - b[0]), label);
            checks.push([byteAt(anchor), expected]);
        }
        const tree = syntaxTree(state);
        const typeAt = at => tree.resolveInner(at, 1).type;
        switch (`${mode}/${name}`) {
            case 'go/term identity separates additive and multiplicative operators':
                assert.deepEqual([typeAt(10).id, typeAt(14).id], [82, 87]);
                assert.equal(typeAt(10).name, typeAt(14).name);
                break;
            case 'go/the same additive term pairs under distinct parents':
                assert.equal(typeAt(8), typeAt(11));
                break;
            case 'rust/term identity separates unary and binary operators':
                assert.deepEqual([typeAt(17).id, typeAt(20).id], [119, 126]);
                assert.equal(typeAt(17).name, typeAt(20).name);
                break;
        }
        console.log(JSON.stringify({mode, name, source, options, checks}));
    }
} finally {
    rmSync(scratch, {recursive: true, force: true});
}
