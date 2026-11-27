// Run in Chromium with an integrity-checked browser import map for the pinned packages.
// Capture globalThis.nativeSupport as JSONL.
import {basicSetup} from 'codemirror';
import {redoDepth, undoDepth} from '@codemirror/commands';
import {foldEffect, foldService, unfoldEffect} from '@codemirror/language';
import {EditorSelection, EditorState, Prec, RangeSet} from '@codemirror/state';
import {
    Decoration, EditorView, GutterMarker, MatchDecorator, gutter, gutterLineClass,
    highlightTrailingWhitespace, highlightWhitespace, lineNumbers,
} from '@codemirror/view';

const source = '😀head\r{\r  猫\r\n}\r\rempty\none\rtwo\rthree\rfour\r';
const encoder = new TextEncoder();
const offsets = [0];
let bytes = 0;
for (const part of source.match(/\r\n|[^]/gu)) {
    if (part.length === 2 && part !== '\r\n') offsets.push(null);
    offsets.push(bytes += encoder.encode(part).length);
}
const byteAt = pos => offsets[pos];
const doc = EditorState.create({doc: source}).doc;
const selection = EditorSelection.create([
    EditorSelection.range(doc.line(1).to, doc.line(2).from),
    EditorSelection.cursor(doc.line(5).from),
    EditorSelection.cursor(doc.line(6).from),
    EditorSelection.cursor(doc.line(6).to),
    EditorSelection.cursor(doc.line(doc.lines).from),
], 2);
class Label extends GutterMarker {
    toDOM() { return document.createTextNode('custom'); }
}
class ClassMarker extends GutterMarker {
    constructor(elementClass) { super(); this.elementClass = elementClass; }
}
const shared = name => gutterLineClass.of(RangeSet.of(new ClassMarker(name).range(doc.line(6).from)));
const duplicate = new Label();
const updates = [];
const view = new EditorView({
    state: EditorState.create({doc: source, selection, extensions: [
        EditorState.allowMultipleSelections.of(true),
        gutter({class: 'cm-foldGutter', renderEmptyElements: true,
            lineMarker: (_view, line) => line.from === 0 ? duplicate : null}),
        lineNumbers({formatNumber: num => `n${num}`}),
        shared('before-active'),
        Prec.low(shared('after-active')),
        foldService.of((_state, from) => from === doc.line(2).from ?
            {from: doc.line(2).to, to: doc.line(4).from} : null),
        basicSetup,
    ]}),
    parent: document.body,
    dispatchTransactions(transactions, view) {
        for (const tr of transactions) {
            updates.push({selectionSet: !!tr.selection, docChanged: tr.docChanged, scroll: tr.scrollIntoView,
                effects: tr.effects.map(effect => {
                    if (effect.is(EditorView.announce)) return ['announce', effect.value];
                    const kind = effect.is(foldEffect) ? 'fold' : effect.is(unfoldEffect) ? 'unfold' : 'other';
                    return [kind, byteAt(effect.value?.from), byteAt(effect.value?.to)];
                })});
        }
        view.update(transactions);
    },
});
view.measure();
const active = [];
for (const provider of view.state.facet(EditorView.decorations)) {
    const set = typeof provider === 'function' ? provider(view) : provider;
    for (let iter = set.iter(); iter.value; iter.next()) {
        if (iter.value.spec.class === 'cm-activeLine') active.push(byteAt(iter.from));
    }
}
const slots = [...view.dom.querySelectorAll('.cm-gutters-before > .cm-gutter')];
const row = el => [el.className, el.textContent];
const gutters = slots.map(slot => {
    const children = [...slot.children];
    const spacer = children.find(el => el.style.visibility === 'hidden');
    return {class: slot.className, spacer: spacer ? row(spacer) : null,
        rows: children.filter(el => el !== spacer).map(row)};
});
const click = [];
for (const title of ['Fold line', 'Unfold line']) {
    const marker = [...slots[2].querySelectorAll('span')]
        .find(el => el.title === title && !el.parentNode.style.visibility);
    updates.length = 0;
    const event = new MouseEvent('click', {bubbles: true, cancelable: true});
    marker.dispatchEvent(event);
    view.measure();
    click.push({handled: event.defaultPrevented, updates: updates.slice(), undo: undoDepth(view.state), redo: redoDepth(view.state)});
}
const lines = Array.from({length: doc.lines}, (_, i) => {
    const {from, to, text} = doc.line(i + 1);
    return [byteAt(from), byteAt(to), text];
});
globalThis.nativeSupport = [JSON.stringify({source, lines, selection: selection.ranges.map(range =>
    [byteAt(range.anchor), byteAt(range.head)]), main: selection.mainIndex, active, gutters, click})];
view.destroy();

for (const [text, separator] of [
    ['a \rb \n\u200b😀\t', null],
    ['a \rb \n\u200b😀\t', '\n'],
    ['a \r\u200b😀 \r\nb\t', null],
]) {
    const matched = new MatchDecorator({regexp: /^[ab]/g, decoration: Decoration.mark({class: 'anchor'})});
    const editor = new EditorView({doc: text, parent: document.body, extensions: [
        basicSetup, highlightWhitespace(), highlightTrailingWhitespace(),
        separator === null ? [] : EditorState.lineSeparator.of(separator),
    ]});
    const positions = [0];
    let end = 0;
    for (const part of (separator === null ? text.match(/\r\n|[^]/gu) : [...text])) {
        if (part.length === 2 && part !== '\r\n') positions.push(null);
        positions.push(end += encoder.encode(part).length);
    }
    const whitespace = [], trailing = [], special = [], anchors = [];
    for (const provider of editor.state.facet(EditorView.decorations)) {
        const set = typeof provider === 'function' ? provider(editor) : provider;
        for (let iter = set.iter(); iter.value; iter.next()) {
            const {class: name, widget} = iter.value.spec;
            const range = [positions[iter.from], positions[iter.to]];
            if (name === 'cm-highlightSpace' || name === 'cm-highlightTab') whitespace.push([...range, name]);
            if (name === 'cm-trailingSpace') trailing.push([...range, name]);
            if (typeof widget?.code === 'number') special.push([...range, widget.code]);
        }
    }
    for (let iter = matched.createDeco(editor).iter(); iter.value; iter.next()) {
        anchors.push([positions[iter.from], positions[iter.to]]);
    }
    globalThis.nativeSupport.push(JSON.stringify({text, separator, whitespace, trailing, special, anchors}));
    editor.destroy();
}
