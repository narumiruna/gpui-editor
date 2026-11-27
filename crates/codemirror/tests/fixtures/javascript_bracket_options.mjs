// Replay with Node and the directory of SHA-512-pinned archives. Add --emit to regenerate.
import assert from 'node:assert/strict';
import {existsSync, mkdtempSync, readFileSync, realpathSync, rmSync} from 'node:fs';
import {tmpdir} from 'node:os';
import {join, resolve} from 'node:path';
import {projectedOffsets} from './editor_line_offsets.mjs';
import {pinnedModules} from './pinned_modules.mjs';

const bmp = {brackets: '«»', maxScanDistance: 1};
const alternate = '/*(«x»\r\n)*/';
const forward = '/*(😀\r\n\r)*/';
const backward = '/*(\r\n\r😀)*/';
const cases = [
    ['js', 'replace plain pairs but keep grammar pairs', 'call("(x)", "«😀»")', bmp],
    ['js', 'custom mate matches within one syntax type', '/*(😀]*/', {brackets: '(]'}],
    ['js', 'same syntax type across separate comments', '// «\r\n// »', {brackets: '«»'}],
    ['js', 'text in comments and strings has different syntax', '/* « */ "»"', bmp],
    ['js', 'same-named comparison nodes have different types', 'a < b > c', {brackets: '<>'}],
    ['js', 'both characters of a comparison have one type', 'a >= b', {brackets: '>='}],
    ['js', 'configured one-character sibling precedes grammar mismatch', 'f(1, 2)', {brackets: ',)'}],
    ['js', 'a configured unpaired sibling counts as a delimiter', 'f(1, 2)', {brackets: ','}],
    ['js', 'grammar pair overrides a conflicting mate', 'f(0)', {brackets: '(]'}],
    ['js', 'a grammar token also has the configured opposite side', 'f(0)', {brackets: ')('}],
    ['js', 'grammar handle ignores the plain scan cap', '`😀${\r\n(0)\r}`', bmp],
    ['js', 'inside interpolation works before-only', '`😀${\r\n(0)\r}`', {...bmp, afterCursor: false}],
    ['js', 'try the adjacent bracket after cutoff', alternate, {...bmp, brackets: '()«»'}],
    ['js', 'before-only mode has no alternate after cutoff', alternate, {...bmp, brackets: '()«»', afterCursor: false}],
    ['js', 'a mismatch wins over the bracket on the right', '/*(«x»]*/', {...bmp, brackets: '()[]«»'}],
    ['js', 'an unmatched result wins over the bracket on the right', '/*)«x»*/', {brackets: '()«»'}],
    ['js', 'forward scan accepts the exact logical UTF-16 distance', forward, {maxScanDistance: 5}],
    ['js', 'forward scan cuts off before the next logical line', forward, {maxScanDistance: 4}],
    ['js', 'backward scan accepts the exact logical UTF-16 distance', backward, {maxScanDistance: 5}],
    ['js', 'backward scan cuts off before the previous logical line', backward, {maxScanDistance: 4}],
    ['js', 'same-line match can exceed the limit', '/*(abcdefghijklmn)*/', {maxScanDistance: 1}],
    ['js', 'same-line miss is unmatched at EOF', '/*(abcdefghijklmn*/', {maxScanDistance: 1}],
    ['js', 'empty pairs and zero distance use defaults', forward, {brackets: '', maxScanDistance: 0}],
    ['ts', 'type argument open is not grammar-marked', 'type T = Box<\r\nnumber\r>;', bmp],
    ['ts', 'configured type argument open matches by node name', 'type T = Box<number>;', {brackets: '<>', maxScanDistance: 1}],
    ['ts', 'type parameter and cast open have grammar props', 'function f<\r\nT\r>(t: T) { return <T>t; }', bmp],
    ['ts', 'compare and type argument angles use distinct types', 'a < b; let x: Box<T>; a > b;', {brackets: '<>'}],
    ['ts', 'template type interpolation has whole handles', 'type T = `😀${\r\nstring\r}`;', bmp],
    ['jsx', 'whole closing punctuation and unmarked self-close', 'const x = <A a="«😀»">\r\n<B/>\r</A>;', bmp],
    ['jsx', 'unmarked JSX punctuation can use a configured text pair', '<A/>', {brackets: '/>'}],
    ['jsx', 'punctuation ignores tag name mismatch', '<A></B>', {...bmp, afterCursor: false}],
    ['jsx', 'fragment matches its full closing punctuation', '<>{[0]}</>', bmp],
    ['tsx', 'type parameters, JSX and plain pairs coexist', 'const C = <T,>(p: T) => <A a="«😀»">\r\n{[p]}\r</A>;', bmp],
    ['tsx', 'broken JSX close and expression', 'const C = <A>{log(")"]}</B>;', {...bmp, brackets: '()]«»'}],
];

const archives = resolve(process.argv[2]);
const root = realpathSync(mkdtempSync(join(tmpdir(), 'javascript-bracket-options-')));
try {
    const {load, pins} = pinnedModules(archives, root);
    for (const name of [
        '@codemirror/autocomplete', '@codemirror/lang-javascript', '@codemirror/language',
        '@codemirror/lint', '@codemirror/state', '@codemirror/streamparser', '@codemirror/view',
        '@lezer/common', '@lezer/highlight', '@lezer/javascript', '@lezer/lr',
        '@marijn/find-cluster-break', 'crelt', 'style-mod', 'w3c-keyname',
    ]) {
        const archive = `${name.split('/').at(-1)}-${pins.get(name).version}.tgz`;
        assert(existsSync(join(archives, name.replace('/', '__'), archive)), `missing local archive: ${archive}`);
    }
    const {javascript} = await load('@codemirror/lang-javascript');
    const {parser} = await load('@lezer/javascript');
    const {NodeProp} = await load('@lezer/common');
    const {bracketMatching, bracketMatchingHandle, forceParsing} = await load('@codemirror/language');
    const {EditorState} = await load('@codemirror/state');
    const {ViewPlugin} = await load('@codemirror/view');
    const types = parser.nodeSet.types;
    const props = (key) => types.flatMap(type => {
        const names = type.prop(key);
        return names ? [[type.id, ...names]] : [];
    });
    assert.deepEqual(props(NodeProp.openedBy), [
        [23, '<'], [38, 'InterpolationStart'], [56, '['], [60, '{'], [73, '('], [161, 'JSXStartCloseTag'],
    ]);
    assert.deepEqual(props(NodeProp.closedBy), [
        [24, '>'], [40, 'InterpolationEnd'], [50, ']'], [61, '}'], [74, ')'], [166, 'JSXEndTag'], [169, '>'],
    ]);
    assert.deepEqual([4, 96, 150].map(id => types[id].name), ['JSXStartTag', '<', 'JSXSelfCloseEndTag']);

    const rows = cases.map(([mode, name, source, options]) => {
        const {units, byteAt} = projectedOffsets(source);
        const support = javascript({typescript: mode.startsWith('ts'), jsx: mode.endsWith('x')});
        assert(support.language.parser.nodeSet.types.every(type => !type.prop(bracketMatchingHandle)));
        const create = (config) => {
            const matching = bracketMatching(config);
            const plugin = matching.flat(Infinity).find(extension => extension instanceof ViewPlugin);
            assert(plugin);
            const view = {state: EditorState.create({doc: source, extensions: [support, matching]}), dispatch(spec) {
                this.state = this.state.update(spec).state;
            }};
            assert(forceParsing(view, units, 5000));
            return anchor => plugin.create({state: view.state.update({selection: {anchor}}).state});
        };
        let rendered;
        const readDirection = create({...options, renderMatch(hit) {
            assert.equal(rendered, null);
            rendered = hit;
            return [];
        }});
        const stock = create(options);
        const span = ({from, to}) => [byteAt(from), byteAt(to)];
        const checks = [];
        let anchor = 0;
        for (const part of ['', ...source.match(/\r\n|[^]/gu)]) {
            anchor += part === '\r\n' ? 1 : part.length;
            rendered = null;
            readDirection(anchor);
            const hit = rendered && [span(rendered.start), rendered.end ? span(rendered.end) : null, rendered.matched];
            const expected = hit ? [hit[0], hit[1]].filter(Boolean).map(range => [...range, hit[2]]) : [];
            expected.sort((a, b) => a[0] - b[0]);
            const marks = [];
            for (const iter = stock(anchor).decorations.iter(); iter.value; iter.next()) {
                const label = iter.value.spec.class;
                assert(label === 'cm-matchingBracket' || label === 'cm-nonmatchingBracket');
                marks.push([...span(iter), label === 'cm-matchingBracket']);
            }
            assert.deepEqual(marks, expected, `${mode}: ${name} at byte ${byteAt(anchor)}`);
            checks.push([byteAt(anchor), hit]);
        }
        return JSON.stringify({mode, name, source, options, checks});
    });
    const actual = rows.join('\n') + '\n';
    if (process.argv.includes('--emit')) process.stdout.write(actual);
    else assert.equal(actual, readFileSync(new URL('./javascript_bracket_options.jsonl', import.meta.url), 'utf8'));
} finally {
    rmSync(root, {recursive: true, force: true});
}
