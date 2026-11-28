// Run with Node and the directory of SHA-512-pinned npm archives.
import assert from 'node:assert/strict';
import {mkdirSync, mkdtempSync, realpathSync, rmSync, symlinkSync} from 'node:fs';
import {tmpdir} from 'node:os';
import {join, resolve} from 'node:path';
import {projectInput, projectedOffsets} from './editor_line_offsets.mjs';
import {pinnedModules} from './pinned_modules.mjs';

const handles = {159: 'second', 151: 'second'};
const selected = {
    closedBy: {159: ['TupleExpression', 'ArrayExpression']},
    openedBy: {151: ['ParenthesizedExpression']},
    handles,
};
const args = 'fn f() { g(¦(¦1¦2¦)¦, ¦[¦"¦😀¦\r\n¦字¦"¦]¦); }\r\n';
const cases = [
    ['explicit mates and selected handles', args, selected, {brackets: '<>', maxScanDistance: 1}],
    ['mismatch uses the mate handle', args, {...selected, closedBy: {159: ['TupleExpression']}}],
    ['missing mate handle keeps a match', args, {...selected, handles: {...handles, 151: 'skip'}}],
    ['skipped marked start can scan text', 'fn f() { "¦[¦😀¦]¦"; }', {
        closedBy: {35: ['Null']}, handles: {35: 'skip'},
    }, {afterCursor: false, maxScanDistance: 3}],
    ['empty lists shadow stock roles', 'fn f¦(¦_: u8¦)¦ {\r\n  "😀";\r\n}', {
        closedBy: {48: []}, openedBy: {50: []},
    }, {brackets: '<>', maxScanDistance: 1}],
    ['mates use names but properties use exact rows', 'fn f() { let c = |¦x¦, ¦y¦| y; }', {
        closedBy: {157: ['Parameter']}, openedBy: {158: ['Parameter']},
    }],
    ['a selector can read the verified root', '¦fn f() {}\r\n¦', {
        closedBy: {8: []}, handles: {8: 'first'},
    }],
];

assert.equal(process.argv.length, 3, 'usage: node rust_configured_brackets.mjs <pinned archive directory>');
const root = mkdtempSync(join(realpathSync(tmpdir()), 'rust-configured-brackets.'));
const oldPath = process.env.PATH;
try {
    const bin = join(root, 'bin');
    mkdirSync(bin);
    symlinkSync('/usr/bin/tar', join(bin, 'tar'));
    process.env.PATH = bin; // No downloader. Missing archives fail the run.
    const {load, installed, pins} = pinnedModules(resolve(process.argv[2]), root);
    const {rust, rustLanguage: stock} = await load('@codemirror/lang-rust');
    const {parser} = await load('@lezer/rust');
    const {NodeProp} = await load('@lezer/common');
    const {Compartment, EditorState} = await load('@codemirror/state');
    const {ViewPlugin} = await load('@codemirror/view');
    const {bracketMatching, bracketMatchingHandle, forceParsing, matchBrackets, syntaxTree} = await load('@codemirror/language');
    const selectors = {
        first: node => node.firstChild,
        second: node => node.firstChild?.nextSibling,
        skip: () => null,
    };
    const stockRoles = p => p.nodeSet.types.flatMap(type => {
        assert.equal(type.prop(bracketMatchingHandle), undefined);
        const opened = type.prop(NodeProp.openedBy), closed = type.prop(NodeProp.closedBy);
        return opened || closed ? [[type.id, type.name, opened ?? null, closed ?? null]] : [];
    });
    const rows = stockRoles(stock.parser);
    assert.deepEqual(rows, stockRoles(parser));
    assert.deepEqual(rows, [[10, ']', ['['], null], [41, '}', ['{'], null], [42, '{', null, ['}']],
        [48, '(', null, [')']], [50, ')', ['('], null], [85, '[', null, [']']]]);
    assert.equal(rust().language, stock);
    const modules = [...installed.keys()].sort().map(name => [name, pins.get(name).version, pins.get(name).integrity]);
    console.log(JSON.stringify({case: 'pins', modules, rows}));

    const configure = props => stock.configure({props: [
        NodeProp.closedBy.add(type => props.closedBy?.[type.id]),
        NodeProp.openedBy.add(type => props.openedBy?.[type.id]),
        bracketMatchingHandle.add(type => selectors[props.handles?.[type.id]]),
    ]});
    const done = (state, source) => {
        const view = {state, dispatch(spec) { this.state = this.state.update(spec).state; }};
        const {units} = projectedOffsets(source);
        assert.equal(state.doc.length, units);
        assert(forceParsing(view, units, 5000));
        assert.equal(syntaxTree(view.state).length, units);
        return view.state;
    };
    const capture = options => {
        let result;
        const matching = bracketMatching(options);
        const plugin = matching.flat(Infinity).find(item => item instanceof ViewPlugin);
        assert.ok(plugin);
        const captureSlot = new Compartment();
        const captured = bracketMatching({...options, renderMatch(hit) { result = hit; return []; }});
        const captureExtension = captureSlot.of(matching);
        return {captureExtension, query(state, source) {
            const {byteAt} = projectedOffsets(source);
            const span = range => range ? [byteAt(range.from), byteAt(range.to)] : null;
            const hit = value => value ? [span(value.start), span(value.end), value.matched] : null;
            const head = state.selection.main.head;
            const directional = [-1, 1].map(dir => hit(matchBrackets(state, head, dir, options)));
            result = null;
            plugin.create({state: state.update({effects: captureSlot.reconfigure(captured)}).state});
            const caret = hit(result);
            const marks = [];
            for (const it = plugin.create({state}).decorations.iter(); it.value; it.next()) {
                marks.push([...span(it), it.value.spec.class]);
            }
            return {head: byteAt(head), directional, caret, marks};
        }};
    };
    for (const [label, annotated, props, options = {}] of cases) {
        const parts = annotated.split('¦'), source = parts.join('');
        const {captureExtension, query} = capture(options);
        const make = language => done(EditorState.create({doc: source, extensions: [language, captureExtension]}), source);
        const base = make(configure(props));
        let anchor = 0;
        const queries = parts.slice(0, -1).map(part => {
            anchor += projectedOffsets(part).units;
            return query(base.update({selection: {anchor}}).state, source);
        });
        if (label === 'mates use names but properties use exact rows') {
            const types = [source.indexOf('|x') + 1, source.indexOf('y|')].map(at => syntaxTree(base).resolveInner(at, 1).parent.type);
            assert.deepEqual(types.map(type => [type.id, type.name]), [[157, 'Parameter'], [158, 'Parameter']]);
        }
        console.log(JSON.stringify({case: 'query', label, source, props, options, queries}));
    }
    {
        const source = 'const X: &str = "字";\r\nfn f<T>(_: T) {}';
        const props = {closedBy: {2: []}, openedBy: {3: []}};
        const options = {brackets: '<>'};
        const {captureExtension, query} = capture(options);
        const slot = new Compartment();
        const initial = done(EditorState.create({doc: source, extensions: [slot.of(configure(props)), captureExtension]}), source);
        const selected = initial.update({selection: {anchor: projectedOffsets(source.slice(0, source.indexOf('<'))).units}}).state;
        assert.equal(syntaxTree(selected), syntaxTree(initial));
        const from = source.indexOf('字'), insert = '🦀🦀';
        const tx = selected.update({changes: {from, to: from + 1, insert}});
        const editedSource = projectInput(source, tx);
        const edited = done(tx.state, editedSource);
        const removed = done(edited.update({effects: slot.reconfigure(stock)}).state, editedSource);
        for (const [state, expected] of [[selected, []], [edited, []], [removed, undefined]]) {
            const type = syntaxTree(state).resolveInner(state.selection.main.head, 1).type;
            assert.equal(type.id, 2);
            assert.deepEqual(type.prop(NodeProp.closedBy), expected);
        }
        console.log(JSON.stringify({case: 'retained', source, props, options, edit: {
            from: Buffer.byteLength(source.slice(0, from)), to: Buffer.byteLength(source.slice(0, from + 1)), insert,
        }, selected: query(selected, source), edited: query(edited, editedSource), removed: query(removed, editedSource),
        retainedSelected: query(selected, source), retainedEdited: query(edited, editedSource)}));
    }
    const {markdown} = await load('@codemirror/lang-markdown');
    for (const quoted of [false, true]) {
        const prefix = quoted ? '> ' : '';
        const label = quoted ? 'quoted fallback' : 'named contiguous';
        const info = quoted ? 'other' : 'rr';
        const annotated = `前 😀\r\n\r\n${prefix}\`\`\`${info}\r\n${prefix}fn λ() { g(¦(¦1¦2¦)¦,\r\n${prefix}  ¦[¦"¦字😀¦"¦]¦); }\r\n${prefix}\`\`\`\r\n`;
        const parts = annotated.split('¦'), source = parts.join('');
        const options = {brackets: '<>', maxScanDistance: 1};
        const {captureExtension, query} = capture(options);
        const language = configure(selected);
        const mount = markdown({codeLanguages: name => name === 'rr' ? language : null, defaultCodeLanguage: quoted ? language : undefined, addKeymap: false});
        const base = done(EditorState.create({doc: source, extensions: [mount, captureExtension]}), source);
        let anchor = 0;
        const queries = parts.slice(0, -1).map(part => {
            anchor += projectedOffsets(part).units;
            return query(base.update({selection: {anchor}}).state, source);
        });
        assert(queries.some(item => item.caret?.[2] && item.marks.length === 2));
        console.log(JSON.stringify({case: 'markdown', label, source, props: selected, options, queries}));
    }
} finally {
    process.env.PATH = oldPath;
    rmSync(root, {recursive: true, force: true});
}
