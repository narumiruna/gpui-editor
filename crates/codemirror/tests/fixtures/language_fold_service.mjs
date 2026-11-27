// Run against the integrity-checked browser import map for the pinned packages.
import {Prec} from '@codemirror/state';
import {EditorView} from '@codemirror/view';
import {foldable, foldCode, foldedRanges, foldService} from '@codemirror/language';
import {json} from '@codemirror/lang-json';

const doc = '[\n  "猫",\n  1\n]\n';
const byte = pos => new TextEncoder().encode(doc.slice(0, pos)).length;
const rows = [];
for (const chosen of ['high', 'default', 'low', 'syntax']) {
    const calls = [];
    const register = (name, range) => foldService.of((state, start, end) => {
        if (state.doc.toString() !== doc) throw Error('Unexpected fold service state');
        calls.push([name, byte(start), byte(end)]);
        return chosen === name ? range : null;
    });
    const view = new EditorView({doc, parent: document.body, extensions: [
        json(), Prec.low(register('low', {from: 1, to: 6})),
        register('default', {from: 1, to: 8}),
        Prec.high(register('high', {from: 1, to: 10})),
        register('later', {from: 1, to: 12}),
    ]});
    view.measure();
    const range = foldable(view.state, 0, 1);
    const queryCalls = calls.splice(0);
    const handled = foldCode(view);
    const folded = [];
    for (let cursor = foldedRanges(view.state).iter(); cursor.value; cursor.next()) {
        folded.push([byte(cursor.from), byte(cursor.to)]);
    }
    rows.push({chosen, doc, range: range && [byte(range.from), byte(range.to)], queryCalls,
        handled, folded, commandCalls: calls});
    view.destroy();
}
globalThis.nativeSupport = rows.map(row => JSON.stringify(row));
