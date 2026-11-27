// Run with Node and the directory of SHA-512-pinned archives. JSONL goes to stdout.
import assert from 'node:assert/strict';
import {mkdtempSync, realpathSync, rmSync} from 'node:fs';
import {tmpdir} from 'node:os';
import {join, resolve} from 'node:path';
import {projectedOffsets} from './editor_line_offsets.mjs';
import {pinnedModules} from './pinned_modules.mjs';

const custom = {brackets: '«»', maxScanDistance: 1};
const zero = {brackets: '', maxScanDistance: 0, afterCursor: true};
const explicit = {brackets: '()[]{}', maxScanDistance: 10000, afterCursor: true};
const comma = {brackets: ',)'};
const capped = {brackets: '()[]{}«»', maxScanDistance: 1};
const javaCaps = '// ¦«😀\r\n// ¦»¦';
const pythonCaps = '# ¦«😀\r\n# ¦»¦';
const javaDefaults = 'class C { int¦[¦] a = ¦{3¦}; String s = "😀¦[界]¦"; }¦\r\n// ¦(😀\r\n// ¦)¦';
const pythonDefaults = 'x = f"😀¦{f¦([1¦]¦)¦}¦"\r\n# ¦(😀\r\n# ¦]¦';
const cases = [
    ['java', 'marked braces and parentheses ignore custom pairs and limit', 'class C ¦{ void f¦(\r\nint x ¦) { log¦("(😀)"); }\r\n¦}', custom],
    ['java', 'unmarked array brackets are absent from custom pairs', 'class C { int¦[¦] a = new int¦[2¦]; }', custom],
    ['java', 'unmarked array names use configured siblings, not the scan limit', 'class C { int¦[¦] a = new int¦[\r\n123¦]; }', {brackets: '[]', maxScanDistance: 1}],
    ['java', 'reversing a pair changes the unmarked array role', 'class C { int¦[¦]¦ a; }', {brackets: ']['}],
    ['java', 'configured commas count depth before the marked closer', 'class C { void f(int x¦, int y¦, int z¦) {} }', comma],
    ['java', 'marked direction takes precedence over a reversed pair', 'class C { void f¦(int x¦)¦ {} }', {brackets: ')('}],
    ['java', 'configured angles in a literal and anonymous type syntax', 'class C¦<T¦> { String s = "¦<😀¦>"; }', {brackets: '<>'}],
    ['java', 'different arithmetic terms with one name', 'class C { int f() { return 1 ¦+ 2 ¦* 3; } }', {brackets: '+*'}],
    ['java', 'same arithmetic term under different parents', 'class C { int f() { return ¦-1 ¦+ 2; } }', {brackets: '-+'}],
    ['java', 'same literal term across nodes, not across a comment', 'class C { String x = "¦«😀"; /* ¦» */ String y = "¦»"; }', {brackets: '«»'}],
    ['java', 'forward UTF-16 boundary after CRLF', javaCaps, {brackets: '«»', maxScanDistance: 4}],
    ['java', 'backward UTF-16 boundary after CRLF', javaCaps, {brackets: '«»', maxScanDistance: 5}],
    ['java', 'UTF-16 cutoff in both directions', javaCaps, {brackets: '«»', maxScanDistance: 3}],
    ['java', 'retry after cutoff and left priority', '/*¦(¦«😀»¦\r\n)*/', capped],
    ['java', 'before-only prevents retry after cutoff', '/*¦(¦«😀»¦\r\n)*/', {...capped, afterCursor: false}],
    ['java', 'implicit defaults', javaDefaults, {}],
    ['java', 'explicit defaults', javaDefaults, explicit],
    ['java', 'empty brackets and zero limit use defaults', javaDefaults, zero],
    ['java', 'missing call and class closers do not match a literal', 'class C ¦{ void f() { log¦(¦"¦)"', custom],
    ['python', 'marked lists and parentheses ignore custom pairs and limit', 'x = ¦[call¦(\r\n"[😀]"¦),\r\n¦]', custom],
    ['python', 'unmarked f-string opener is excluded, but marked closers and nested opener remain', 'x = f"😀¦{value:¦{width¦}}¦"', custom],
    ['python', 'both f-string openers match by name with braces enabled', 'x = f"😀¦{value:¦{width¦}}¦"', {brackets: '{}', maxScanDistance: 1}],
    ['python', 'configured commas count depth before the marked closer', 'call(x¦, y¦, z¦)¦', comma],
    ['python', 'marked direction takes precedence over a reversed pair', 'call¦(x¦)¦', {brackets: ')('}],
    ['python', 'different arithmetic terms with one name', 'x = ¦-1 ¦+ 2', {brackets: '-+'}],
    ['python', 'same arithmetic term under different parents', 'x = 1 ¦+ 2 ¦- 3', {brackets: '+-'}],
    ['python', 'string prefixes share a term, not with comments', 'x = r"¦«😀" # ¦»\r\ny = b"¦»"', {brackets: '«»'}],
    ['python', 'both UTF-16 boundaries after CRLF', pythonCaps, {brackets: '«»', maxScanDistance: 4}],
    ['python', 'UTF-16 cutoff in both directions', pythonCaps, {brackets: '«»', maxScanDistance: 3}],
    ['python', 'retry after cutoff and left priority', 'x = """¦(¦«😀»¦\r\n)"""', capped],
    ['python', 'before-only prevents retry after cutoff', 'x = """¦(¦«😀»¦\r\n)"""', {...capped, afterCursor: false}],
    ['python', 'implicit defaults', pythonDefaults, {}],
    ['python', 'explicit defaults', pythonDefaults, explicit],
    ['python', 'empty brackets and zero limit use defaults', pythonDefaults, zero],
    ['python', 'missing call and group closers do not match a literal', 'x = ¦(call¦(¦"¦)"', custom],
];

const props = {
    java: [[10, ')', ['('], null], [11, '(', null, [')']],
           [52, '}', ['{'], null], [53, '{', null, ['}']]],
    python: [[23, ')', ['('], null], [24, '(', null, [')']],
             [54, ']', ['['], null], [55, '[', null, [']']],
             [58, '}', ['{'], null], [59, '{', null, ['}']]],
};

const scratch = mkdtempSync(join(realpathSync(tmpdir()), 'gpui-java-python-brackets.'));
try {
    const {load} = pinnedModules(resolve(process.argv[2]), scratch);
    const {NodeProp} = await load('@lezer/common');
    const {bracketMatching, bracketMatchingHandle, forceParsing, syntaxTree} = await load('@codemirror/language');
    const {EditorState} = await load('@codemirror/state');
    const {ViewPlugin} = await load('@codemirror/view');
    const supports = {};
    for (const mode of ['java', 'python']) {
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
        let hit;
        const [state, capture] = create({...options, renderMatch(match) {
            assert.equal(hit, null);
            hit = match;
            return [];
        }});
        const [, stock] = create(options);
        const checks = [];
        const anchors = [];
        let prefix = '';
        for (const part of parts.slice(0, -1)) {
            prefix += part;
            const anchor = projectedOffsets(prefix).units;
            anchors.push(anchor);
            hit = null;
            capture(anchor);
            const expected = hit ? [range(hit.start), hit.end ? range(hit.end) : null, hit.matched] : null;
            const marks = [];
            for (const iter = stock(anchor).decorations.iter(); iter.value; iter.next()) {
                const kind = iter.value.spec.class;
                assert(kind === 'cm-matchingBracket' || kind === 'cm-nonmatchingBracket');
                marks.push([...range(iter), kind === 'cm-matchingBracket']);
            }
            const spans = expected ? [expected[0], expected[1]].filter(Boolean).map(span => [...span, expected[2]]) : [];
            assert.deepEqual(marks, spans.sort((a, b) => a[0] - b[0]), `${mode}: ${name} at ${anchor}`);
            checks.push([byteAt(anchor), expected]);
        }
        const tree = syntaxTree(state);
        const types = anchors.map(at => tree.resolveInner(at, 1).type);
        const terms = types.map(type => [type.id, type.name]);
        switch (`${mode}/${name}`) {
            case 'java/unmarked array brackets are absent from custom pairs':
                assert.deepEqual(terms, [[31, '['], [29, ']'], [31, '['], [29, ']']]);
                break;
            case 'java/different arithmetic terms with one name':
                assert.deepEqual(terms, [[66, 'ArithOp'], [67, 'ArithOp']]);
                break;
            case 'java/same arithmetic term under different parents':
            case 'python/same arithmetic term under different parents':
                assert.equal(types[0], types[1]);
                break;
            case 'python/unmarked f-string opener is excluded, but marked closers and nested opener remain':
                assert.deepEqual(terms.slice(0, 3), [[3, '{'], [59, '{'], [58, '}']]);
                break;
            case 'python/different arithmetic terms with one name':
                assert.deepEqual(terms, [[19, 'ArithOp'], [13, 'ArithOp']]);
                break;
        }
        console.log(JSON.stringify({mode, name, source, options, checks}));
    }
} finally {
    rmSync(scratch, {recursive: true, force: true});
}
