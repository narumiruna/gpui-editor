// Run with Node and the directory of SHA-512-pinned npm archives.
import assert from 'node:assert/strict';
import {mkdtempSync, realpathSync, rmSync} from 'node:fs';
import {tmpdir} from 'node:os';
import {join, resolve} from 'node:path';
import {projectedOffsets, projectInput} from './editor_line_offsets.mjs';
import {pinnedModules} from './pinned_modules.mjs';

assert.equal(process.argv.length, 3, 'usage: node json_state.mjs <pinned archive directory>');
const scratch = mkdtempSync(join(realpathSync(tmpdir()), 'gpui-json-state-oracle.'));
try {
    const {load} = pinnedModules(resolve(process.argv[2]), scratch);
    const {json} = await load('@codemirror/lang-json');
    const {Compartment, EditorState, Transaction} = await load('@codemirror/state');
    const {bracketMatching, foldable, getIndentation, syntaxTree} = await load('@codemirror/language');
    const {history, insertNewlineAndIndent, isolateHistory, redo, undo} = await load('@codemirror/commands');
    const {closeBrackets, insertBracket} = await load('@codemirror/autocomplete');
    const {ViewPlugin} = await load('@codemirror/view');
    const {diagnosticCount} = await load('@codemirror/lint');
    const emit = (name, value) => console.log(JSON.stringify({name, ...value}));
    const makeTarget = spec => {
        const target = {state: EditorState.create(spec), dispatch(tr) { target.state = tr.state; }};
        return target;
    };

    // The state lifecycle mirrors the audit. Keep original line breaks and byte offsets.
    const original = '{\r\n  "😀": [\r    1\n  ]\n}';
    const language = new Compartment();
    const filter = EditorState.changeFilter.of(() => [5, 9]); // Protect the quoted property name.
    const target = makeTarget({doc: original, extensions: [language.of(json()), filter, history()]});
    const snapshot = (name, raw, state = target.state) => {
        assert.equal(state.doc.toString(), raw.replace(/\r\n?/g, '\n'));
        const {byteAt} = projectedOffsets(raw);
        const line = state.doc.line(1);
        const indentAt = state.doc.line(2).from;
        const fold = foldable(state, line.from, line.to);
        emit(name, {source: raw, head: byteAt(state.selection.main.head), indentAt: byteAt(indentAt),
            indent: getIndentation(state, indentAt), lineEnd: byteAt(line.to),
            fold: fold ? [byteAt(fold.from), byteAt(fold.to)] : null,
            brackets: state.languageDataAt('closeBrackets', 0).map(data => data.brackets),
            diagnostics: diagnosticCount(state)});
    };
    const edit = changes => target.dispatch(target.state.update({changes, annotations: [
        isolateHistory.of('full'), Transaction.userEvent.of('input.type'),
    ]}));
    const old = target.state;
    snapshot('initial', original);
    target.dispatch(old.update({selection: {anchor: 1}}));
    assert.equal(syntaxTree(old), syntaxTree(target.state));
    snapshot('selection', original);
    const {byteAt} = projectedOffsets(original);
    emit('protected', {range: [byteAt(5), byteAt(9)]});
    edit([{from: 0, insert: ' '}, {from: 5, to: 9, insert: 'ignored'}]);
    const shifted = ` ${original}`;
    snapshot('edited', shifted);
    snapshot('retained', original, old);
    edit({from: target.state.doc.length - 1, to: target.state.doc.length});
    snapshot('broken', shifted.slice(0, -1));
    edit({from: target.state.doc.length, insert: '}'});
    snapshot('repaired', shifted);
    assert(undo(target));
    snapshot('undo', shifted.slice(0, -1));
    assert(redo(target));
    snapshot('redo', shifted);
    target.dispatch(target.state.update({effects: language.reconfigure([])}));
    snapshot('detached', shifted);
    edit({from: 0, insert: ' '});
    snapshot('detached edit', ` ${shifted}`);
    target.dispatch(target.state.update({effects: language.reconfigure(json())}));
    snapshot('reattached', ` ${shifted}`);

    const enter = makeTarget({doc: '{}', selection: {anchor: 1}, extensions: json()});
    assert(insertNewlineAndIndent(enter));
    emit('enter', {source: '{}', head: 1, result: enter.state.doc.toString(), selection: enter.state.selection.main.head});

    const source = '{"a":"}"}';
    const matching = bracketMatching();
    const plugin = matching.flat(Infinity).find(extension => extension instanceof ViewPlugin);
    assert(plugin);
    const state = EditorState.create({doc: source, selection: {anchor: 1}, extensions: [json(), matching]});
    const marks = [];
    for (const iter = plugin.create({state}).decorations.iter(); iter.value; iter.next()) {
        assert.equal(iter.value.spec.class, 'cm-matchingBracket');
        marks.push([iter.from, iter.to]);
    }
    emit('match', {source, head: 1, marks});

    for (const [name, source, anchor] of [
        ['string opener', '"a"', 0],
        ['property opener', '{"😀": 0, "a": 1}', 10],
        ['inside a string', '" "', 1],
    ]) {
        const input = '"';
        const state = EditorState.create({doc: source, selection: {anchor}, extensions: [json(), closeBrackets()]});
        const tr = insertBracket(state, input) ?? state.update(state.replaceSelection(input));
        const result = projectInput(source, tr);
        emit(name, {source, input, head: projectedOffsets(source).byteAt(anchor),
            result, selection: projectedOffsets(result).byteAt(tr.newSelection.main.head)});
    }
} finally {
    rmSync(scratch, {recursive: true, force: true});
}
