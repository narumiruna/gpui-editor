// Run against the integrity-checked browser import map for the pinned packages.
import {Compartment, EditorState} from '@codemirror/state';
import {EditorView} from '@codemirror/view';
import {codeFolding, foldEffect, foldState, unfoldEffect} from '@codemirror/language';

const source = '猫 lead\n[old]\n';
const slot = new Compartment();
const calls = [];
const preparePlaceholder = (state, range) => {
    const doc = state.doc.toString();
    const prepared = state.sliceDoc(range.from, range.to);
    calls.push([doc, [byte(doc, range.from), byte(doc, range.to)], prepared]);
    return prepared;
};
const mode = prefix => codeFolding({preparePlaceholder, placeholderDOM(_view, _unfold, prepared) {
    const marker = document.createElement('span');
    marker.className = 'configured-fold';
    marker.textContent = prefix + ':' + (prepared ?? '-');
    return marker;
}});
const stock = codeFolding({placeholderText: 'hidden'});
const base = EditorState.phrases.of({'folded code': 'hidden source', unfold: 'show source'});
const view = new EditorView({doc: source, parent: document.body, extensions: [codeFolding(), base, slot.of(mode('first'))]});
function byte(doc, pos) { return new TextEncoder().encode(doc.slice(0, pos)).length; }
function byteRange(doc, range) { return [byte(doc, range.from), byte(doc, range.to)]; }
const rows = [];
function record(phase, spec = {}) {
    const doc = view.state.doc.toString();
    const current = view.state.field(foldState);
    const markers = [];
    for (const cursor = current.iter(); cursor.value; cursor.next()) {
        // Probe the current widget against current configuration, independent
        // of whether the DOM reconciler chose to retain a rendered widget.
        const marker = cursor.value.spec.widget.toDOM(view);
        markers.push([byte(doc, cursor.from), byte(doc, cursor.to), marker.textContent,
            marker.getAttribute('title'), marker.getAttribute('aria-label')]);
    }
    rows.push({phase, doc, ...spec, markers, calls: calls.splice(0)});
}
record('before');
const first = source.indexOf('old');
const initialFold = {from: first, to: first + 'fresh'.length};
view.dispatch({changes: {from: first, to: first + 3, insert: 'fresh'}, effects: foldEffect.of(initialFold)});
record('fold_after_edit', {edit: [byte(source, first), byte(source, first + 3), 'fresh'], fold: byteRange(view.state.doc.toString(), initialFold)});
view.dispatch({changes: {from: 0, insert: '💡'}});
record('mapped', {edit: [0, 0, '💡']});
const mapped = {from: first + 2, to: initialFold.to + 2};
view.dispatch({effects: foldEffect.of(mapped)});
record('duplicate', {fold: byteRange(view.state.doc.toString(), mapped)});
view.dispatch({effects: slot.reconfigure(mode('second'))});
record('renderer');
const lead = view.state.doc.toString().indexOf('lead');
const revealed = {from: lead, to: lead + 'lead'.length};
view.dispatch({effects: foldEffect.of(revealed), selection: {anchor: lead + 1}});
record('clear_before_paint', {fold: byteRange(view.state.doc.toString(), revealed), head: byte(view.state.doc.toString(), lead + 1)});
const json = view.state.toJSON({folds: foldState});
const restored = EditorState.fromJSON(json, {extensions: [codeFolding(), base, slot.of(mode('second'))]}, {folds: foldState});
view.setState(restored);
record('restored');
view.dispatch({effects: slot.reconfigure(stock)});
record('stock_label');
view.dispatch({effects: unfoldEffect.of(mapped)});
record('unfold', {fold: byteRange(view.state.doc.toString(), mapped)});
view.destroy();
globalThis.nativeSupport = rows.map(row => JSON.stringify(row));
