// Run with Node and the directory of SHA-512-pinned npm archives.
import assert from 'node:assert/strict';
import {mkdtempSync, realpathSync, rmSync} from 'node:fs';
import {tmpdir} from 'node:os';
import {join, resolve} from 'node:path';
import {pinnedModules} from './pinned_modules.mjs';

// Map normalized UTF-16 positions to original UTF-8 byte boundaries.
function inputPositions(source) {
    const positions = [0];
    let bytes = 0;
    for (const [text] of source.matchAll(/\r\n|[^]/gu)) {
        bytes += Buffer.byteLength(text);
        if (text.length === 2 && text !== '\r\n') positions.push(null);
        positions.push(bytes);
    }
    return pos => {
        const byte = positions[pos];
        assert(Number.isSafeInteger(byte), 'expected an original byte boundary');
        return byte;
    };
}

assert.equal(process.argv.length, 3, 'usage: node wast_state.mjs <pinned archive directory>');
const scratch = mkdtempSync(join(realpathSync(tmpdir()), 'gpui-wast-state.'));
try {
    const {load} = pinnedModules(resolve(process.argv[2]), scratch);
    const {wast} = await load('@codemirror/lang-wast');
    const {EditorState, Prec} = await load('@codemirror/state');
    const {IndentContext, foldService, foldable, getIndentation, indentRange, indentService,
        indentUnit, syntaxTree, syntaxTreeAvailable} = await load('@codemirror/language');
    const {history, redo, redoSelection, undo, undoSelection} = await load('@codemirror/commands');
    const output = (name, value) => console.log(JSON.stringify({name, ...value}));
    const before = ';; 😀\r\n(module\r  (func)\n)\r(;\rnote\r;)\n';
    const source = '(func\r    nop\r  )';
    const from = before.replace(/\r\n/g, '\n').indexOf('(func)');
    const to = from + '(func)'.length;
    const after = before.replace('(func)', source);
    const target = {state: EditorState.create({doc: before, extensions: [wast(), history()]}),
        dispatch(tr) { target.state = tr.state; }};
    function snapshot(name, state, raw) {
        assert(syntaxTreeAvailable(state));
        assert.equal(state.doc.toString(), raw.replace(/\r\n|\r/g, '\n'));
        const byte = inputPositions(raw);
        const lines = [];
        for (let n = 1; n <= state.doc.lines; ++n) {
            const {from, to} = state.doc.line(n);
            const fold = foldable(state, from, to);
            lines.push([byte(from), byte(to), getIndentation(state, from),
                fold ? [byte(fold.from), byte(fold.to)] : null]);
        }
        output(name, {source: raw, head: byte(state.selection.main.head), lines,
            commentTokens: state.languageDataAt('commentTokens', 0),
            closeBrackets: state.languageDataAt('closeBrackets', 0)});
    }
    const old = target.state, oldTree = syntaxTree(old);
    snapshot('initial', old, before);
    const byte = inputPositions(before);
    output('change', {from: byte(from), to: byte(to), insert: source});
    target.dispatch(old.update({changes: {from, to, insert: source}, selection: {anchor: from + 1}}));
    const editedTree = syntaxTree(target.state);
    assert.notEqual(editedTree, oldTree);
    snapshot('edited', target.state, after);
    target.dispatch(target.state.update({selection: {anchor: 0}, userEvent: 'select'}));
    assert.equal(syntaxTree(target.state), editedTree);
    assert(undoSelection(target));
    assert.equal(syntaxTree(target.state), editedTree);
    snapshot('undoSelection', target.state, after);
    assert(redoSelection(target));
    assert.equal(syntaxTree(target.state), editedTree);
    snapshot('redoSelection', target.state, after);
    assert(undo(target));
    snapshot('undo', target.state, before);
    assert(redo(target));
    snapshot('redo', target.state, after);
    assert.equal(syntaxTree(old), oldTree);
    snapshot('retained', old, before);

    const raw = '\t(module\r\n\t  (func  )\r)\n';
    const config = [wast(), EditorState.tabSize.of(4), indentUnit.of('   ')];
    const state = EditorState.create({doc: raw, extensions: config});
    const pos = state.doc.toString().indexOf('  )');
    const root = state.doc.line(2).from;
    const mapped = inputPositions(raw);
    const overrideIndentation = at => at === root ? 9 : -1;
    for (const [name, options] of Object.entries({ordinary: {}, single: {simulateBreak: pos},
        double: {simulateBreak: pos, simulateDoubleBreak: true},
        overrideSingle: {simulateBreak: pos, overrideIndentation},
        overrideDouble: {simulateBreak: pos, simulateDoubleBreak: true, overrideIndentation}})) {
        const cx = new IndentContext(state, options);
        const line = bias => {
            const {from, text} = cx.lineAt(pos, bias);
            return [mapped(from), text];
        };
        output(name, {source: raw, pos: mapped(pos), root: mapped(root), unit: cx.unit,
            column: getIndentation(cx, pos), lineIndent: cx.lineIndent(root),
            before: line(-1), after: line(1)});
    }
    for (const [name, answer] of [['Defer', undefined], ['Column', 31], ['Inherit', null]]) {
        const calls = [];
        const state = EditorState.create({doc: raw, extensions: [...config,
            indentService.of(() => { calls.push('answer'); return answer; }),
            Prec.high(indentService.of(() => { calls.push('high'); return undefined; }))]});
        const cx = new IndentContext(state, {simulateBreak: pos, simulateDoubleBreak: true, overrideIndentation});
        output(name, {column: getIndentation(cx, pos), calls});
    }
    for (const [name, answer] of [['foldDefer', null], ['foldCustom', {from: 2, to: 5}]]) {
        const calls = [];
        const state = EditorState.create({doc: '(module\n(func)\n)', extensions: [wast(),
            foldService.of(() => { calls.push('answer'); return answer; }),
            Prec.high(foldService.of(() => { calls.push('high'); return null; }))]});
        const {from, to} = foldable(state, 0, state.doc.line(1).to);
        output(name, {fold: [from, to], calls});
    }
    const unindented = '(module\r(func\r\nnop\r )\r)';
    const rangeState = EditorState.create({doc: unindented, extensions: [wast()]});
    const rangeResult = rangeState.update({changes: indentRange(rangeState, 0, rangeState.doc.length)}).state;
    const breaks = [...unindented.matchAll(/\r\n|\r|\n/g)].map(([part]) => part);
    assert.equal(rangeResult.doc.lines, breaks.length + 1);
    let index = 0;
    output('indentRange', {source: unindented, result: rangeResult.doc.toString().replace(/\n/g, () => breaks[index++])});
} finally {
    rmSync(scratch, {recursive: true, force: true});
}
