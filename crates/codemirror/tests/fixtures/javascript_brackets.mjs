// Replay with node and the directory of SHA-512-pinned archives. Add --emit to regenerate.
import assert from 'node:assert/strict';
import {existsSync, mkdtempSync, readFileSync, realpathSync, rmSync} from 'node:fs';
import {tmpdir} from 'node:os';
import {join, resolve} from 'node:path';
import {projectedOffsets} from './editor_line_offsets.mjs';
import {pinnedModules} from './pinned_modules.mjs';

const cases = [
    ['js', 'call with string', 'log(")");'],
    ['js', 'call with template', 'log(`)`);'],
    ['js', 'call with block comment', 'log(/* ) */ 1);'],
    ['js', 'literal syntax classes', 'log("[😀)", "]", /[()]/, `([)]`, "\\("); /* [(]) */'],
    ['js', 'CRLF line comments', '// [𐐀(\r\nlog("]"); // )\r\n// ]'],
    ['js', 'lone CR line comments', '// [𐐀(\rlog("]"); // )\r// ]'],
    ['js', 'CRLF escaped string line', 'log("\\\r\n)");'],
    ['js', 'missing call delimiter', 'log(")"'],
    ['js', 'skipped mismatched delimiter', 'log(1];'],
    ['js', 'stray mismatched delimiters', '] } )'],
    ['js', 'missing nested closes', 'log([{k: 1'],
    ['js', 'unterminated template', 'log(`x(😀\r\n'],
    ['js', 'unterminated block comment', 'log(/* [😀\r\n'],
    ['js', 'template interpolation boundary', '`prefix(${log(")")})${[0]}`'],
    ['js', 'nested interpolations with original line breaks', '`😀\r\n${`${[0]}`}${\r(x)}`'],
    ['js', 'missing interpolation delimiter', '`α${(1)'],
    ['js', 'unmarked angle and slash characters', 'a < b && c > d; /[<>]/; "< > </ >";'],
    ['ts', 'type and value groups', 'let xs: Array<{v: [number]}> = [{v: [1]}]; log(")");'],
    ['ts', 'incomplete type and value', 'let xs: Array<{v: [number'],
    ['ts', 'type parameters, nested type arguments, and casts', 'interface X<T> { x: Foo<Bar<T>>; } f<X<T>>(); let x = <T>(1);'],
    ['ts', 'missing type parameter delimiter', 'function f<T('],
    ['ts', 'template type interpolation', 'type T = `(${string})`;'],
    ['jsx', 'attribute, children, and expression', '<A a="[😀)" b={() => ["]"]}> ) ] {log(")")} </A>'],
    ['jsx', 'fragment boundary', '<>{log(")")}</>'],
    ['jsx', 'missing JSX escape and tag', '<A>{log(")"'],
    ['jsx', 'mismatched names and self-closing punctuation', '<Outer><Inner key={p}/></Other   >'],
    ['jsx', 'incomplete close punctuation', '<Outer>text</Other'],
    ['jsx', 'angle and slash characters in attribute and text', '<A a="< />"> text / > </A>'],
    ['tsx', 'type parameters and JSX escapes', 'const C = <T,>(p: T) => <>{[p, "]"]}</>;'],
    ['tsx', 'broken JSX close and expression', 'const C = <A>{log(")"]}</B>;'],
    ['js', 'plain scan within UTF-16 limit', '/* ($\r\n) */', ['😀', 4995]],
    ['js', 'plain scan beyond UTF-16 limit', '/* ($\r\n) */', ['😀', 5005]],
    ['js', 'marked pair beyond plain scan limit', 'log(/* $\r\n) */1)', ['😀', 5005]],
];

const archives = resolve(process.argv[2]);
const root = realpathSync(mkdtempSync(join(tmpdir(), 'javascript-brackets-')));
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
    const annotations = parser.nodeSet.types.flatMap(type => {
        const openedBy = type.prop(NodeProp.openedBy), closedBy = type.prop(NodeProp.closedBy);
        return openedBy || closedBy ? [[type.id, type.name, openedBy ?? null, closedBy ?? null]] : [];
    });
    assert.deepEqual(annotations, [
        [23, '>', ['<'], null], [24, '<', null, ['>']],
        [38, 'InterpolationEnd', ['InterpolationStart'], null],
        [40, 'InterpolationStart', null, ['InterpolationEnd']],
        [50, '[', null, [']']], [56, ']', ['['], null],
        [60, '}', ['{'], null], [61, '{', null, ['}']],
        [73, ')', ['('], null], [74, '(', null, [')']],
        [161, 'JSXEndTag', ['JSXStartCloseTag'], null],
        [166, 'JSXStartCloseTag', null, ['JSXEndTag']], [169, '<', null, ['>']],
    ]);
    let rendered;
    const matching = bracketMatching({renderMatch(hit) {
        assert.equal(rendered, null);
        rendered = hit;
        return [];
    }});
    const plugin = matching.flat(Infinity).find(extension => extension instanceof ViewPlugin);
    assert(plugin);
    const rows = cases.map(([mode, name, source, repeat]) => {
        const raw = repeat ? source.replace('$', repeat[0].repeat(repeat[1])) : source;
        const {units, byteAt} = projectedOffsets(raw);
        const support = javascript({typescript: mode.startsWith('ts'), jsx: mode.endsWith('x')});
        assert(support.language.parser.nodeSet.types.every(type => !type.prop(bracketMatchingHandle)));
        const view = {state: EditorState.create({doc: raw, extensions: [support, matching]}), dispatch(spec) {
            this.state = this.state.update(spec).state;
        }};
        assert(forceParsing(view, units, 5000));
        const doc = view.state.doc.toString();
        const anchors = [0];
        let cursor = 0;
        for (const part of doc) {
            cursor += part.length;
            anchors.push(cursor);
        }
        const checks = [];
        for (const anchor of anchors) {
            if (repeat && !/[()[\]{}]/.test(doc.slice(Math.max(0, anchor - 1), anchor + 1))) continue;
            rendered = null;
            plugin.create({state: view.state.update({selection: {anchor}}).state});
            const span = ({from, to}) => [byteAt(from), byteAt(to)];
            const hit = rendered && [span(rendered.start), rendered.end ? span(rendered.end) : null, rendered.matched];
            checks.push([byteAt(anchor), hit]);
        }
        return JSON.stringify({mode, name, source, repeat, checks});
    });
    const actual = rows.join('\n') + '\n';
    if (process.argv.includes('--emit')) process.stdout.write(actual);
    else assert.equal(actual, readFileSync(new URL('./javascript_brackets.jsonl', import.meta.url), 'utf8'));
} finally {
    rmSync(root, {recursive: true, force: true});
}
