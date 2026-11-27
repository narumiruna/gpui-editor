// Replay with Node 22.14 or later and the directory of SHA-512-pinned archives.
import assert from 'node:assert/strict';
import {mkdtempSync, realpathSync, rmSync} from 'node:fs';
import {tmpdir} from 'node:os';
import {join, resolve} from 'node:path';
import {projectedOffsets, projectInput} from './editor_line_offsets.mjs';
import {pinnedModules} from './pinned_modules.mjs';

const scratch = mkdtempSync(join(realpathSync(tmpdir()), 'gpui-go-brackets.'));
try {
    const {load, pins} = pinnedModules(resolve(process.argv[2]), scratch);
    const {go, goLanguage} = await load('@codemirror/lang-go');
    const {parser} = await load('@lezer/go');
    const {bracketMatching, bracketMatchingHandle, forceParsing, syntaxTree} = await load('@codemirror/language');
    const {EditorState} = await load('@codemirror/state');
    const {ViewPlugin} = await load('@codemirror/view');
    const {NodeProp} = await load('@lezer/common');
    const props = parser => parser.nodeSet.types.flatMap(type => {
        assert.equal(type.prop(bracketMatchingHandle), undefined, type.name);
        const open = type.prop(NodeProp.openedBy), close = type.prop(NodeProp.closedBy);
        return open || close ? [[type.id, type.name, open ?? null, close ?? null]] : [];
    });
    const expectedProps = [
        [13, ')', ['('], null], [14, '(', null, [')']],
        [25, '}', ['{'], null], [26, '{', null, ['}']],
        [53, ']', ['['], null], [54, '[', null, [']']],
    ];
    assert.deepEqual(props(parser), expectedProps);
    assert.deepEqual(props(goLanguage.parser), expectedProps);
    for (const name of ['@codemirror/lang-go', '@lezer/go', '@codemirror/language']) {
        const {version, integrity} = pins.get(name);
        console.error(`${name} ${version}: verified ${integrity}`);
    }
    console.error(`delimiter NodeProps: ${JSON.stringify(expectedProps)}`);

    let rendered;
    const matching = bracketMatching({renderMatch(hit) {
        assert.equal(rendered, null);
        rendered = hit;
        return [];
    }});
    const plugin = matching.flat(Infinity).find(extension => extension instanceof ViewPlugin);
    assert(plugin);
    const extensions = [go(), matching];
    const create = source => EditorState.create({doc: source, extensions});
    function snapshot(state, source) {
        const {units, byteAt} = projectedOffsets(source);
        assert.equal(state.doc.toString(), source.replace(/\r\n?/g, '\n'));
        const view = {state, dispatch(spec) { this.state = this.state.update(spec).state; }};
        assert(forceParsing(view, units, 5000));
        const tree = syntaxTree(view.state);
        assert.equal(tree.length, units);
        let recovered = false;
        tree.iterate({enter(node) { recovered ||= node.type.isError; }});
        const span = ({from, to}) => [byteAt(from), byteAt(to)];
        const hits = [];
        let anchor = 0;
        const probe = () => {
            rendered = null;
            plugin.create({state: view.state.update({selection: {anchor}}).state});
            hits.push([byteAt(anchor), rendered && [span(rendered.start), rendered.end ? span(rendered.end) : null, rendered.matched]]);
        };
        probe();
        for (const [part] of source.matchAll(/\r\n|[^]/gu)) {
            anchor += part === '\r\n' ? 1 : part.length;
            probe();
        }
        return [view.state, {recovered, hits}];
    }
    function replay(name, source, edits = []) {
        let [state, expected] = snapshot(create(source), source);
        console.log(JSON.stringify({name, source, ...expected}));
        for (const [name, remove, insert] of edits) {
            const start = source.indexOf(remove);
            assert(start !== -1 && source.indexOf(remove, start + 1) === -1, remove);
            const before = projectedOffsets(source.slice(0, start)).units;
            const after = before + projectedOffsets(remove).units;
            const {byteAt} = projectedOffsets(source);
            const change = [byteAt(before), byteAt(after), insert];
            const transaction = state.update({changes: {from: before, to: after, insert}});
            source = projectInput(source, transaction);
            [state, expected] = snapshot(transaction.state, source);
            assert.deepEqual(expected, snapshot(create(source), source)[1], name);
            console.log(JSON.stringify({name, change, ...expected}));
        }
    }

    replay('code, string and comment closers', 'package p\r\nfunc f() { println(/* 🦀 ) */ "界)"); }\r');
    replay('nested syntax and label', 'func f[T any](x T) { a := []T{x}; a = a[(0):]; f[T](a[0]); }');
    replay('string, rune, raw string and block comment', 'var s = "([)]"\r\nvar a, b = \'{\', \'}\'\rvar r = `([😀\r\n\r])`\n/* {界\r} */');
    replay('same type across separate tokens', 'var a = "("; var b = `)`\r// [😀\r\n/* ] */\r// ]');
    replay('unterminated block comment', 'func f() {\r\n/* start [😀\r  ]');
    replay('wrong call closer', 'func f() { use(1]; }');
    replay('recovery keeps later declarations', 'func f(param int) {\n    use({ foo ]  } )\n    var next = 3\n    return next\n}\n');
    replay('damage and repair after a Unicode prefix', 'package p\r\nvar 名 = "😀"\rfunc f() { use(名) }\r\n', [
        ['remove the call closer', 'use(名)', 'use(名'],
        ['repair the call', 'use(名', 'use(名)'],
        ['remove the block closer', ') }', ') '],
        ['repair the block and shift its ranges', '名) ', '名, "🦀") }'],
    ]);
} finally {
    rmSync(scratch, {recursive: true, force: true});
}
