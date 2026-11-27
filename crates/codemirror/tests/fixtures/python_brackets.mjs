// Replay with Node 22.14 or later and the directory of SHA-512-pinned archives.
import assert from 'node:assert/strict';
import {mkdtempSync, realpathSync, rmSync} from 'node:fs';
import {tmpdir} from 'node:os';
import {join, resolve} from 'node:path';
import {projectedOffsets, projectInput} from './editor_line_offsets.mjs';
import {pinnedModules} from './pinned_modules.mjs';

const scratch = mkdtempSync(join(realpathSync(tmpdir()), 'gpui-python-brackets.'));
try {
    const {load, pins} = pinnedModules(resolve(process.argv[2]), scratch);
    const {python, pythonLanguage} = await load('@codemirror/lang-python');
    const {parser} = await load('@lezer/python');
    const {bracketMatching, bracketMatchingHandle, forceParsing, syntaxTree} = await load('@codemirror/language');
    const {EditorState} = await load('@codemirror/state');
    const {ViewPlugin} = await load('@codemirror/view');
    const {NodeProp} = await load('@lezer/common');
    const props = parser => parser.nodeSet.types.flatMap(type => {
        assert.equal(type.prop(bracketMatchingHandle), undefined, type.name);
        const open = type.prop(NodeProp.openedBy), close = type.prop(NodeProp.closedBy);
        return open || close ? [[type.id, type.name, open ?? null, close ?? null]] : [];
    });
    for (const name of ['@codemirror/lang-python', '@lezer/python', '@codemirror/language']) {
        const {version, integrity} = pins.get(name);
        console.error(`${name} ${version}: verified ${integrity}`);
    }
    const expectedProps = [
        [23, ')', ['('], null], [24, '(', null, [')']],
        [54, ']', ['['], null], [55, '[', null, [']']],
        [58, '}', ['{'], null], [59, '{', null, ['}']],
    ];
    for (const grammar of [parser, pythonLanguage.parser]) {
        assert.deepEqual(props(grammar), expectedProps);
        // The external f-string replacement token matches by name, not prop.
        assert.deepEqual(grammar.nodeSet.types.filter(type => type.name === '{').map(type => type.id), [3, 59]);
    }
    console.error(`delimiter NodeProps: ${JSON.stringify(expectedProps)}; unmarked f-string opener: [3,"{"]`);

    let rendered;
    const matching = bracketMatching({renderMatch(hit) {
        assert.equal(rendered, null);
        rendered = hit;
        return [];
    }});
    const defaults = bracketMatching();
    const plugin = defaults.flat(Infinity).find(extension => extension instanceof ViewPlugin);
    assert(plugin);
    assert(matching.flat(Infinity).includes(plugin));
    const create = (source, extension = matching) => EditorState.create({doc: source, extensions: [python(), extension]});
    function parsed(state, units) {
        const view = {state, dispatch(spec) { this.state = this.state.update(spec).state; }};
        assert(forceParsing(view, units, 5000));
        assert.equal(syntaxTree(view.state).length, units);
        return view.state;
    }
    function snapshot(state, source, anchors) {
        const {units, byteAt} = projectedOffsets(source);
        assert.equal(state.doc.toString(), source.replace(/\r\n?/g, '\n'));
        state = parsed(state, units);
        let recovered = false;
        syntaxTree(state).iterate({enter(node) { recovered ||= node.type.isError; }});
        const defaultState = parsed(create(source, defaults), units);
        const plainState = EditorState.create({doc: source, extensions: matching});
        const span = ({from, to}) => [byteAt(from), byteAt(to)];
        const hits = [];
        const plain = [];
        let anchor = 0;
        const render = state => {
            rendered = null;
            plugin.create({state: state.update({selection: {anchor}}).state});
            return rendered && [span(rendered.start), rendered.end ? span(rendered.end) : null, rendered.matched];
        };
        const probe = () => {
            const hit = render(state);
            const raw = render(plainState);
            const {decorations} = plugin.create({state: defaultState.update({selection: {anchor}}).state});
            const marks = [];
            for (let iter = decorations.iter(); iter.value; iter.next()) {
                marks.push([...span(iter), iter.value.spec.class]);
            }
            const expected = hit ? [hit[0], hit[1]].filter(Boolean).sort((a, b) => a[0] - b[0])
                .map(range => [...range, hit[2] ? 'cm-matchingBracket' : 'cm-nonmatchingBracket']) : [];
            assert.deepEqual(marks, expected);
            if (hit !== null || raw !== null) hits.push([byteAt(anchor), hit]);
            if (JSON.stringify(hit) !== JSON.stringify(raw)) plain.push([byteAt(anchor), raw]);
        };
        if (anchors) {
            for (anchor of anchors) probe();
        } else {
            probe();
            for (const [part] of source.matchAll(/\r\n|[^]/gu)) {
                anchor += part === '\r\n' ? 1 : part.length;
                probe();
            }
        }
        return [state, {recovered, hits, plain}];
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

    replay('call with a comment and literal closer', '# 界😀\r\nrun(\r\n    # )\r\n    "é)",\r\n)');
    replay('grouping, indexing and comprehensions', 'def run(xs: list[int]):\n    return ({x: [x] for x in xs}, {x for x in xs}, (xs[0]))\n');
    replay('plain, raw, bytes and triple strings', 'x = "([)]"\r\ny = r"["\rz = b"]"\nq = """{😀\r\n界}"""');
    replay('same syntax type across tokens', 'x = "(" ")"\r\n# [😀\r\n# ]\r# (]');
    replay('f-string literals and replacements', 'x = f"([)] {{escaped}} {obj[(1, 2)]!r:>{width}.{precision}} {f\'{{{other}}}\'}"');
    replay('literal brackets around f-string expressions', 'x = f"界😀 ) {call(\')\')} ( [ {obj[0]} ] {{text}}"\r\n');
    replay('raw multiline f-string', 'value = rf"""界😀[\r\n{x(1)}\r]\r\n{{text}}"""\n');
    replay('bracket inside a string escape', 'x = "\\[😀]"\r\n');
    replay('damage and repair after Unicode', '# 界😀\r\nvalue = "é"\r\nresult = run(1)\r', [
        ['remove the call closer', 'run(1)', 'run(1'],
        ['shift and repair the call', 'run(1', 'run("🦀", [1])'],
        ['replace call closer with wrong bracket', '[1])', '[1]]'],
        ['repair the wrong bracket', '[1]]', '[1])'],
    ]);
    replay('native prefix recovery cannot see later grouping', 'first = (1)\r\nbroken = )\r\nlater = [2]\r\n');

    // The line chunk and logical break reach 10,000 UTF-16 units with 4,999 emoji.
    for (const separator of ['\r\n', '\r']) {
        for (const count of [4999, 5000]) {
            const source = `# (${'😀'.repeat(count)}${separator}# )`;
            const end = Buffer.byteLength(source);
            const [, {hits}] = snapshot(create(source), source, [2]);
            assert.deepEqual(hits, count === 4999 ? [[2, [[2, 3], [end - 1, end], true]]] : []);
        }
    }
    const structural = `(${' '.repeat(10_001)}\r\n1)`;
    const end = Buffer.byteLength(structural);
    const [, {hits}] = snapshot(create(structural), structural, [0]);
    assert.deepEqual(hits, [[0, [[0, 1], [end - 1, end], true]]]);
} finally {
    rmSync(scratch, {recursive: true, force: true});
}
