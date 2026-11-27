// Run with capture_browser_fixture.mjs and the SHA-512-verified archives.
import {basicSetup} from 'codemirror';
import {highlightSelectionMatches} from '@codemirror/search';
import {Compartment, EditorState} from '@codemirror/state';
import {EditorView} from '@codemirror/view';

const stockSource = 'α cat scatter cat cat_ cat';
const cssSource = 'alpha-beta\n\n```css\np { alpha: alpha-beta; content: alpha }\n```\n\nalpha';
const cssStart = cssSource.indexOf('p {');
const cssWord = cssSource.indexOf('alpha:', cssStart);
const encoder = new TextEncoder();
const cases = [
    {setup: true, config: [], range: [2, 5]},
    {setup: false, config: [], range: [2, 5]},
    {setup: true, config: [{wholeWords: true}], range: [2, 5]},
    {name: 'whole words', setup: false, config: [{wholeWords: true}], range: [2, 5]},
    {setup: true, config: [{wholeWords: true}, {wholeWords: false}], range: [2, 5]},
    {setup: true, config: [{maxMatches: 2}, {maxMatches: 3}], range: [2, 5]},
    {name: 'cap', setup: false, config: [{maxMatches: 1}, {maxMatches: 5}], range: [2, 5]},
    {name: 'minimum length', setup: false, config: [{minSelectionLength: 4}, {minSelectionLength: 5}], range: [2, 5]},
    {setup: false, config: [{minSelectionLength: 4}, {minSelectionLength: 3}], range: [2, 5]},
    {setup: true, config: [{highlightWordAroundCursor: false}, {highlightWordAroundCursor: true}], range: [2, 2]},
    {name: 'cursor word', setup: false, config: [{highlightWordAroundCursor: false}, {highlightWordAroundCursor: true}], range: [2, 2]},
    {setup: false, config: [{highlightWordAroundCursor: false}], range: [2, 2]},
    {name: 'positional word characters', setup: false, config: [{wholeWords: true}], range: [cssWord, cssWord + 5],
        source: cssSource, wordScope: [cssStart, cssSource.indexOf('\n```', cssStart), '-']},
];
const rows = [];
const regions = editor => {
    const byte = pos => encoder.encode(editor.state.sliceDoc(0, pos)).length;
    const marks = [];
    for (const provide of editor.state.facet(EditorView.decorations)) {
        const set = typeof provide === 'function' ? provide(editor) : provide;
        for (let iter = set.iter(); iter.value; iter.next()) {
            if (iter.value.spec.class?.startsWith('cm-selectionMatch')) {
                marks.push([byte(iter.from), byte(iter.to), iter.value.spec.class]);
            }
        }
    }
    return marks;
};
for (const {name, setup, config, range, source = stockSource, wordScope} of cases) {
    const slot = new Compartment();
    const host = document.createElement('div');
    const outside = document.createElement('button');
    document.body.append(host, outside);
    let editor;
    const byte = pos => encoder.encode(source.slice(0, pos)).length;
    const byteRange = range.map(byte);
    const selection = {anchor: range[0], head: range[1]};
    const language = wordScope ? EditorState.languageData.of((_, pos) =>
        wordScope[0] <= pos && pos <= wordScope[1] ? [{wordChars: wordScope[2]}] : []) : [];
    try {
        editor = new EditorView({parent: host, state: EditorState.create({doc: source, selection,
            extensions: [setup ? basicSetup : [], language, slot.of(config.map(highlightSelectionMatches))]})});
        if (name) editor.focus();
        const before = regions(editor);
        let unfocused, stock;
        if (name) {
            if (!editor.hasFocus) throw new Error('expected editor focus');
            outside.focus();
            if (editor.hasFocus) throw new Error('expected outside focus');
            unfocused = regions(editor);
            editor.dispatch({selection, effects: slot.reconfigure(highlightSelectionMatches())});
            stock = regions(editor);
        }
        editor.dispatch({selection, effects: slot.reconfigure([])});
        const after = regions(editor);
        rows.push({name, source, setup, config, selection: byteRange, before, unfocused, stock, after,
            wordScope: wordScope && [byte(wordScope[0]), byte(wordScope[1]), wordScope[2]]});
    } catch (error) {
        if (!String(error).includes('wholeWords')) throw error;
        rows.push({source, setup, config, selection: byteRange, conflict: true});
    } finally {
        editor?.destroy();
        host.remove();
        outside.remove();
    }
}
globalThis.nativeSupport = rows.map(row => JSON.stringify(row));
