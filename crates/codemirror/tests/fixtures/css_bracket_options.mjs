// Replay with Node and the directory of SHA-512-pinned archives. Add --emit to regenerate.
import assert from 'node:assert/strict';
import {mkdtempSync, readFileSync, realpathSync, rmSync} from 'node:fs';
import {tmpdir} from 'node:os';
import {join, resolve} from 'node:path';
import {projectedOffsets} from './editor_line_offsets.mjs';
import {pinnedModules} from './pinned_modules.mjs';

const sheet = 'stylesheet', styles = 'styles';
const forward = '/*(😀\r\n\r)*/';
const backward = '/*(\r\n\r😀)*/';
const alternate = '/*(«x»\r\n)*/';
const cases = [
    [sheet, 'grammar pairs override conflicting mates', 'a[foo] {\r\ncolor: f(1)\r}', {brackets: '[)'}],
    [styles, 'grammar pairs exceed the plain cap', '--v: f(\r\n😀\r)', {brackets: '«»', maxScanDistance: 1}],
    [styles, 'a grammar token also has the configured opposite side', '--v: f(1)', {brackets: ')('}],
    [styles, 'unfinished marked delimiter ignores a comment', '--v: f( /* ) */ 1;', {brackets: '«»', maxScanDistance: 1}],
    [styles, 'a custom pair replaces default pairs in strings', 'content: "(x)«😀»"', {brackets: '«»', maxScanDistance: 1}],
    [sheet, 'a custom mate in a string', 'a { content: "(x]"; }', {brackets: '(]'}],
    [styles, 'named punctuation uses siblings', '--v: foo(1,2,3)', {brackets: ',)', maxScanDistance: 1}],
    [sheet, 'comments and strings have different types', '/*«*/a {content:"»"}', {brackets: '«»'}],
    [styles, 'separate string nodes share one type', `--a: "«"; --b: '»'`, {brackets: '«»'}],
    [sheet, 'query and declaration property names have different terms', String.raw`a{p\(:0} @container style(p\):1) {}`, {brackets: '()'}],
    [sheet, 'retain the term in a recovered header', String.raw`@container style(p\(:1) {} a{p\):0;`, {brackets: '()'}],
    [styles, 'retain the term in a recovered value', String.raw`--v: if(style(p\(:1):0); p\):0; --x: f(`, {brackets: '()'}],
    [sheet, 'pseudo-class argument lists have different terms', ':nth-child(1 2){}:not(.a,\t.b){}', {brackets: ' \t'}],
    [styles, 'call and if argument lists have different terms', '--a: f(a b); --b: if(media:0;\tother:1)', {brackets: ' \t'}],
    [sheet, 'query and value colors have different terms', 'a{--v:#abc} @media (#def > 0){}', {brackets: 'ad'}],
    [styles, 'ordinary declarations share one property name term', String.raw`p\(:0;p\):1`, {brackets: '()'}],
    [sheet, 'forward scan accepts the exact cap', forward, {maxScanDistance: 5}],
    [sheet, 'forward scan stops before the next logical line', forward, {maxScanDistance: 4}],
    [styles, 'backward scan accepts the exact cap', backward, {maxScanDistance: 5}],
    [styles, 'backward scan stops before the previous logical line', backward, {maxScanDistance: 4}],
    [sheet, 'try right after cutoff', alternate, {brackets: '()«»', maxScanDistance: 1}],
    [sheet, 'before-only mode does not try right after cutoff', alternate, {brackets: '()«»', maxScanDistance: 1, afterCursor: false}],
    [styles, 'a mismatch on the left wins', '/*(«x»]*/', {brackets: '()[]«»'}],
    [styles, 'an unmatched delimiter on the left wins', '/*)«x»*/', {brackets: '()«»'}],
    [styles, 'a same-line match can exceed the cap', '/*(abcdef)*/', {maxScanDistance: 1}],
    [styles, 'a same-line miss returns unmatched at EOF', '/*(abcdef*/', {maxScanDistance: 1}],
    [sheet, 'empty options use stock pairs and distance', forward, {brackets: '', maxScanDistance: 0}],
];
const scratch = mkdtempSync(join(realpathSync(tmpdir()), 'gpui-css-bracket-options.'));
try {
    const {load} = pinnedModules(resolve(process.argv[2]), scratch);
    const {css, cssLanguage} = await load('@codemirror/lang-css');
    const {bracketMatching, bracketMatchingHandle, forceParsing} = await load('@codemirror/language');
    const {EditorState} = await load('@codemirror/state');
    const {EditorView, ViewPlugin} = await load('@codemirror/view');
    const {NodeProp} = await load('@lezer/common');
    const inline = cssLanguage.configure({top: 'Styles'});
    const props = key => cssLanguage.parser.nodeSet.types.flatMap(type => {
        const value = type.prop(key);
        return value ? [[type.id, type.name, ...value]] : [];
    });
    assert.deepEqual(props(NodeProp.openedBy), [[23, ')', '('], [30, ']', '['], [33, '}', '{']]);
    assert.deepEqual(props(NodeProp.closedBy), [[24, '(', ')'], [31, '[', ']'], [34, '{', '}']]);
    assert.deepEqual(props(bracketMatchingHandle), []);
    const rows = cases.map(([mode, name, source, options]) => {
        const {units, byteAt} = projectedOffsets(source);
        const support = mode === styles ? inline : css();
        const create = config => {
            const matching = bracketMatching(config);
            const plugin = matching.flat(Infinity).find(extension => extension instanceof ViewPlugin);
            assert(plugin);
            const view = {
                state: EditorState.create({doc: source, extensions: [support, matching]}),
                dispatch(spec) { this.state = this.state.update(spec).state; },
            };
            assert(forceParsing(view, units, 5000));
            return anchor => {
                const state = view.state.update({selection: {anchor}}).state;
                const instance = plugin.create({state});
                assert(state.facet(EditorView.decorations).some(get => typeof get === 'function' &&
                    get({plugin(p) { return p === plugin ? instance : null; }}) === instance.decorations));
                return instance;
            };
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
    else assert.equal(actual, readFileSync(new URL('./css_bracket_options.jsonl', import.meta.url), 'utf8'));
} finally {
    rmSync(scratch, {recursive: true, force: true});
}
