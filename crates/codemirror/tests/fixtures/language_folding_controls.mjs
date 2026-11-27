// Run with the integrity-checked browser import map for the pinned packages.
import {EditorState} from '@codemirror/state';
import {EditorView} from '@codemirror/view';
import {
    codeFolding, foldService, foldGutter, foldEffect, unfoldEffect, foldedRanges,
    foldCode, unfoldCode, toggleFold, foldAll, unfoldAll, foldKeymap,
} from '@codemirror/language';

const doc = 'open\n猫 line\nend';
const region = {from: 4, to: 12};
const bytes = pos => new TextEncoder().encode(doc.slice(0, pos)).length;
const range = ({from, to}) => [bytes(from), bytes(to)];
const phrases = EditorState.phrases.of({
    'Folded lines': 'Collapsed', 'Unfolded lines': 'Expanded', to: 'through',
    'Fold line': 'Hide source', 'Unfold line': 'Show source',
});
const provider = foldService.of((_state, from) => from === 0 ? region : null);
const rows = [{kind: 'keymap', bindings: foldKeymap.map(binding => [binding.key, binding.mac ?? null, binding.run.name])}];
const commands = [
    ['fold', foldCode, false], ['unfold', unfoldCode, true],
    ['toggle_fold', toggleFold, false], ['toggle_unfold', toggleFold, true],
    ['fold_all', foldAll, false], ['unfold_all', unfoldAll, true],
];
function effects(transactions) {
    return transactions.flatMap(tr => tr.effects.flatMap(effect => {
        if (effect.is(foldEffect)) return [['fold', range(effect.value)]];
        if (effect.is(unfoldEffect)) return [['unfold', range(effect.value)]];
        if (effect.is(EditorView.announce)) return [['announce', effect.value]];
        return [];
    }));
}
for (const [name, run, seed] of commands) {
    const captured = [];
    const view = new EditorView({doc, parent: document.body, extensions: [codeFolding(), phrases, provider],
        dispatchTransactions(trs, view) { captured.push(...effects(trs)); view.update(trs); }});
    view.measure();
    if (seed) view.dispatch({effects: foldEffect.of(region)});
    captured.length = 0;
    rows.push({kind: 'command', name, doc, region: range(region), handled: run(view), effects: captured});
    view.destroy();
}

for (const custom of [false, true]) {
    const clicks = [], captured = [];
    let intercept = true;
    const markerDOM = custom ? open => {
        const el = document.createElement('span');
        el.className = 'custom-fold';
        el.textContent = open ? 'expanded' : 'collapsed';
        return el;
    } : null;
    const view = new EditorView({doc, parent: document.body, extensions: [phrases, provider, foldGutter({
        openText: 'collapse', closedText: 'expand', markerDOM,
        domEventHandlers: {click(_view, line) { clicks.push(range(line)); return intercept; }},
    })], dispatchTransactions(trs, view) { captured.push(...effects(trs)); view.update(trs); }});
    view.measure();
    function markers() {
        return [...view.dom.querySelectorAll('.cm-foldGutter span')].map(el => [el.textContent, el.getAttribute('title'), el.className]);
    }
    const before = markers(), steps = [];
    for (const name of ['intercept', 'fold', 'unfold']) {
        intercept = name === 'intercept';
        const active = [...view.dom.querySelectorAll('.cm-foldGutter span')].at(-1);
        active.dispatchEvent(new MouseEvent('click', {bubbles: true, cancelable: true}));
        const folds = [], cursor = foldedRanges(view.state).iter();
        for (; cursor.value; cursor.next()) folds.push(range(cursor));
        steps.push({name, line: clicks.at(-1), effects: captured.splice(0), folds, markers: markers()});
    }
    rows.push({kind: 'gutter', custom, doc, region: range(region), before, steps});
    view.destroy();
}
globalThis.nativeSupport = rows.map(row => JSON.stringify(row));
