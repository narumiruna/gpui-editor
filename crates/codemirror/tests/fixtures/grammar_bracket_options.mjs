// Run with Node and the directory of SHA-512-pinned archives.
import assert from 'node:assert/strict';
import {mkdtempSync, realpathSync, rmSync} from 'node:fs';
import {join, resolve} from 'node:path';
import {tmpdir} from 'node:os';
import {projectedOffsets} from './editor_line_offsets.mjs';
import {pinnedModules} from './pinned_modules.mjs';

const bmp = {brackets: '«»', maxScanDistance: 1};
const mixed = ';; (😀\r\n;; \r;; )\r\n(module)';
const cases = [
    ['wast', 'custom pairs in strings and a marked parent', '(module (func "(x)«😀»"))', bmp],
    ['wast', 'same type across comments', ';; «\r\n;; »\r\n(module)', {brackets: '«»'}],
    ['wast', 'different types do not pair', ';; «\r\n(module "»")', {brackets: '«»'}],
    ['wast', 'scan stops before the next chunk', mixed, {maxScanDistance: 7}],
    ['wast', 'forward scan at exact limit', mixed, {maxScanDistance: 8}],
    ['wast', 'backward scan at exact limit', mixed, {maxScanDistance: 9}],
    ['wast', 'only marked pairs ignore the limit', '(module\r\n  (func)\r)', bmp],
    ['wast', 'can try the right bracket after cutoff', ';; («x»\r\n;; )', {brackets: '()«»', maxScanDistance: 1}],
    ['wast', 'before-only cannot try the right bracket', ';; («x»\r\n;; )', {brackets: '()«»', maxScanDistance: 1, afterCursor: false}],
    ['wast', 'grammar direction can have an opposite configured name', '(module)', {brackets: ')('}],
    ['wast', 'custom mate wins in a block comment', '(; (x] ;)', {brackets: '(]'}],
    ['wast', 'prior unmatched bracket wins', ';; )«x»', {brackets: '()«»'}],
    ['lezer', 'configured pairs in strings and marked braces', '@top T { "(x)«😀»" }', bmp],
    ['lezer', 'configured same type across comments', '// «\r\n// »\r\n@top T { R }\r\nR { "x" }', {brackets: '«»'}],
    ['lezer', 'different types do not pair', '// «\r\n@top T { "»" }', {brackets: '«»'}],
    ['lezer', 'structured braces ignore options and limit', '@top T {\r\n  ("x")\r}', bmp],
    ['lezer', 'literal is not the same type as character set', '@top T { "«" $[»] }', {brackets: '«»'}],
    ['lezer', 'matching punctuation on same line overshoots the cap', '/*(abcdefgh)*/', {maxScanDistance: 1}],
    ['lezer', 'logical CRLF and emoji count at exact cap', '/*(😀\r\n\r)*/', {maxScanDistance: 5}],
    ['lezer', 'logical CRLF and emoji cutoff', '/*(😀\r\n\r)*/', {maxScanDistance: 4}],
    ['lezer', 'retry right after cutoff', '/*(«x»\r\n)*/', {brackets: '()«»', maxScanDistance: 1}],
    ['lezer', 'prior mismatch wins over right pair', '/*(«x»]*/', {brackets: '()[]«»'}],
    ['lezer', 'named punctuation as a configured delimiter', '@top T { A | B }\r\nA { "a" }\r\nB { "b" }', {brackets: '|}'}],
    ['lezer', 'before-only with reversed named grammar brackets', '@top T { "x" }', {brackets: '}{', afterCursor: false}],
    ['xml', 'host tag handles ignore cap and missing alternatives', '<root>😀\r\n<node/>\r</root>', bmp],
    ['xml', 'alternate pairs in attributes and text', '<root a="(x)«q»">«😀»</root>', bmp],
    ['xml', 'attributes and text use different syntax types', '<root a="«">»</root>', {brackets: '«»'}],
    ['xml', 'CDATA content can match configured text pairs', '<root><![CDATA[«😀»]]></root>', bmp],
    ['xml', 'comment scan stops before next logical line', '<root><!--(😀\r\n\r)--></root>', {maxScanDistance: 4}],
    ['xml', 'comment scan matches at the exact limit', '<root><!--(😀\r\n\r)--></root>', {maxScanDistance: 5}],
    ['xml', 'retry right after cutoff outside markup', '<root>(«x»\r\n)</root>', {brackets: '()«»', maxScanDistance: 1}],
    ['xml', 'before-only applies to tag handles and custom text pairs', '<root>«😀»</root>', {brackets: '«»', afterCursor: false}],
    ['xml', 'markup punctuation keeps grammar precedence', '<root><node/></root>', {brackets: '<)'}],
    ['xml', 'case-sensitive tags stay unmatched', '<root><Item></item></root>', bmp],
    ['cpp', 'code delimiters skip literal and comment characters', 'int main() { log(/* ) */ ")"); }', {}],
    ['cpp', 'raw literal delimiter is not a code delimiter', 'int n = log(R"tag(😀 [)] )tag");', {}],
    ['c', 'array and initializer punctuation matches in C', 'int data[2] = {1, 2};', bmp],
    ['cpp', 'configured angle names match nested template arguments', 'using X = Box<Box<int>>;', {brackets: '<>'}],
    ['cpp', 'default pairs do not include template angles', 'using X = Box<int>;', {}],
    ['cpp', 'configured bracket stays within its syntax type', 'const char *s = u8"«😀»"; /* « */', bmp],
    ['cpp', 'same comment type matches with logical UTF-16 limit', '/*(😀\r\n\r)*/ int n = 0;', {maxScanDistance: 5}],
    ['cpp', 'comment scan cuts off before another chunk', '/*(😀\r\n\r)*/ int n = 0;', {maxScanDistance: 4}],
    ['cpp', 'logical cutoff permits a right probe', '/*(«x»\r\n)*/', {brackets: '()«»', maxScanDistance: 1}],
    ['cpp', 'before-only omits the right probe after cutoff', '/*(«x»\r\n)*/', {brackets: '()«»', maxScanDistance: 1, afterCursor: false}],
    ['c', 'one-character names participate in marked depth', 'int f(int a,int b,int c);', {brackets: ',)'}],
    ['cpp', 'ordinary comparison token is not a template token', 'bool n = a < b > c;', {brackets: '<>'}],
];

const root = mkdtempSync(join(realpathSync(tmpdir()), 'gpui-grammar-brackets.'));
try {
    const {load} = pinnedModules(resolve(process.argv[2]), root);
    const {wast, wastLanguage} = await load('@codemirror/lang-wast');
    const {lezer, lezerLanguage} = await load('@codemirror/lang-lezer');
    const {xml} = await load('@codemirror/lang-xml');
    const {cpp, cppLanguage} = await load('@codemirror/lang-cpp');
    const {NodeProp} = await load('@lezer/common');
    const {bracketMatching, bracketMatchingHandle, forceParsing} = await load('@codemirror/language');
    const {EditorState} = await load('@codemirror/state');
    const {ViewPlugin} = await load('@codemirror/view');
    for (const [provider, pairs] of [[wastLanguage, '()'], [lezerLanguage, '()[]{}'], [cppLanguage, '()[]{}']]) {
        const types = provider.parser.nodeSet.types;
        assert(types.every(type => !type.prop(bracketMatchingHandle)));
        assert.deepEqual(types.filter(type => type.prop(NodeProp.closedBy)).map(type => [type.name, ...type.prop(NodeProp.closedBy)]).sort(),
            [...pairs.matchAll(/../g)].map(([pair]) => [...pair]).sort());
        assert.deepEqual(types.filter(type => type.prop(NodeProp.openedBy)).map(type => [type.name, ...type.prop(NodeProp.openedBy)]).sort(),
            [...pairs.matchAll(/../g)].map(([pair]) => [...pair].reverse()).sort());
    }
    const language = {wast, lezer, xml, cpp, c: cpp};
    for (const [mode, name, source, options] of cases) {
        const {units, byteAt} = projectedOffsets(source);
        let rendered;
        const create = (opts) => {
            const matching = bracketMatching(opts);
            const plugin = matching.flat(Infinity).find(extension => extension instanceof ViewPlugin);
            assert(plugin);
            const view = {state: EditorState.create({doc: source, extensions: [language[mode](), matching]}), dispatch(spec) {
                this.state = this.state.update(spec).state;
            }};
            assert(forceParsing(view, units, 5000));
            return anchor => plugin.create({state: view.state.update({selection: {anchor}}).state});
        };
        const readHit = create({...options, renderMatch(hit) {
            assert.equal(rendered, null);
            rendered = hit;
            return [];
        }});
        const stock = create(options);
        const range = ({from, to}) => [byteAt(from), byteAt(to)];
        let anchor = 0;
        const checks = [];
        for (const part of ['', ...source.match(/\r\n|[^]/gu)]) {
            anchor += part === '\r\n' ? 1 : part.length;
            rendered = null;
            readHit(anchor);
            const hit = rendered && [range(rendered.start), rendered.end ? range(rendered.end) : null, rendered.matched];
            const marks = [];
            for (const iter = stock(anchor).decorations.iter(); iter.value; iter.next()) {
                const kind = iter.value.spec.class;
                assert(kind === 'cm-matchingBracket' || kind === 'cm-nonmatchingBracket');
                marks.push([...range(iter), kind === 'cm-matchingBracket']);
            }
            const expected = hit ? [hit[0], hit[1]].filter(Boolean).map(span => [...span, hit[2]]) : [];
            assert.deepEqual(marks, expected.sort((a, b) => a[0] - b[0]), `${name} at ${anchor}`);
            checks.push([byteAt(anchor), hit]);
        }
        console.log(JSON.stringify({mode, name, source, options, checks}));
    }
} finally {
    rmSync(root, {recursive: true, force: true});
}
