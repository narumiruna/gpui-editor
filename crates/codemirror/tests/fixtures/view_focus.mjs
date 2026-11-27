import {EditorState, Compartment, Prec, StateEffect, Transaction} from '@codemirror/state';
import {EditorView, ViewPlugin} from '@codemirror/view';
import {history, invertedEffects, undo, undoDepth} from '@codemirror/commands';

const flag = StateEffect.define();
const text = StateEffect.define();
const point = StateEffect.define({map: (pos, changes) => changes.mapPos(pos, 1)});
const extra = StateEffect.define();
const effects = list => list.map(effect => effect.is(flag) ? ['flag', effect.value]
    : effect.is(text) ? ['text', effect.value]
    : effect.is(point) ? ['point', effect.value]
    : effect.is(extra) ? ['extra', effect.value]
    : ['config']);
const pause = () => new Promise(resolve => setTimeout(resolve, 60));
const outside = document.body.appendChild(document.createElement('button'));
const rows = [];

function mount(log, extensions) {
    const record = kind => update => {
        if (update.focusChanged || update.transactions.length) {
            log.push([kind, update.startState.doc.toString(), update.state.doc.toString(),
                update.focusChanged, update.view.hasFocus, update.transactions.length,
                effects(update.transactions.flatMap(tr => tr.effects))]);
        }
    };
    return new EditorView({
        doc: 'ab',
        parent: document.body,
        extensions: [...extensions, ViewPlugin.define(() => ({update: record('plugin')})),
            EditorView.updateListener.of(record('listener'))],
        dispatchTransactions(trs, view) {
            log.push(['dispatch', trs.length, effects(trs.flatMap(tr => tr.effects))]);
            view.update(trs);
        },
    });
}

function provider(log, name, makeEffect = () => null) {
    return EditorView.focusChangeEffect.of((state, focused) => {
        log.push(['provider', name, state.doc.toString(), state.selection.main.head, focused]);
        return makeEffect(state, focused);
    });
}

async function capture(name, extensions, exercise) {
    outside.focus();
    const log = [];
    const view = mount(log, extensions(log));
    await exercise(view, log);
    rows.push(JSON.stringify({case: name, log}));
    view.destroy();
}

await capture('idle', log => {
    const named = provider(log, 'text', (state, focused) => text.of(`${state.doc}:${focused}`));
    return [named, provider(log, 'skip'),
        Prec.highest(provider(log, 'flag', (_, focused) => flag.of(focused))), named,
        Prec.low(provider(log, 'point', state => point.of(state.selection.main.head)))];
}, async view => {
    view.focus();
    await pause();
    view.focus();
    await pause();
    outside.focus();
    await pause();
});

await capture('no_effect', log => [provider(log, 'skip')], async view => {
    view.focus();
    outside.focus();
    await pause();
    view.focus();
    await pause();
    outside.focus();
    await pause();
});

for (const batch of [false, true]) {
    const compartment = new Compartment();
    let next;
    await capture(batch ? 'batch' : 'composed', log => {
        next = provider(log, 'new', state => text.of(state.doc.toString()));
        return [compartment.of(provider(log, 'old', () => text.of('old')))];
    }, async view => {
        const first = {changes: {from: 1, insert: 'X'},
            effects: [point.of(2), compartment.reconfigure(next)]};
        const second = {changes: {from: 0, insert: '!'}, selection: {anchor: 3}, sequential: true};
        view.focus();
        if (batch) {
            const tr = view.state.update(first);
            view.dispatch([tr, tr.state.update(second)]);
        } else {
            view.dispatch(first, second);
        }
        await pause();
    });
}

await capture('composed_no_effect', log => [provider(log, 'skip')], async view => {
    view.focus();
    view.dispatch({changes: {from: 2, insert: 'X'}},
        {changes: {from: 3, insert: '!'}, sequential: true});
    await pause();
});

await capture('stale', log => [provider(log, 'text', state => text.of(state.doc.toString()))],
    async view => {
        view.focus();
        view.dispatch({changes: {from: 2, insert: 'X'}});
        view.dispatch({changes: {from: 3, insert: '!'}});
        await pause();
    });

await capture('filtered', log => [
    provider(log, 'flag', (_, focused) => flag.of(focused)),
    EditorState.transactionFilter.of(tr => {
        log.push(['filter', effects(tr.effects)]);
        return tr;
    }),
    EditorState.transactionExtender.of(tr => {
        log.push(['extender', effects(tr.effects)]);
        return {effects: extra.of('accepted')};
    }),
], async view => {
    view.focus();
    await pause();
});

await capture('removed_effect', log => [
    provider(log, 'flag', (_, focused) => flag.of(focused)),
    EditorState.transactionFilter.of(tr => ({annotations: tr.annotations})),
], async view => {
    view.focus();
    await pause();
});

await capture('history', log => [
    history(), provider(log, 'flag', (_, focused) => flag.of(focused)),
    invertedEffects.of(tr => tr.effects.flatMap(effect => effect.is(flag) ? [flag.of(!effect.value)] : [])),
    EditorState.transactionFilter.of(tr => tr.effects.some(effect => effect.is(flag) && !effect.value)
        ? {effects: tr.effects, annotations: [...tr.annotations, Transaction.addToHistory.of(false)]}
        : tr),
], async (view, log) => {
    view.focus();
    await pause();
    log.push(['undo_depth', undoDepth(view.state)]);
    outside.focus();
    await pause();
    log.push(['undo_depth', undoDepth(view.state)]);
    undo(view);
    log.push(['undo_depth', undoDepth(view.state)]);
});

globalThis.nativeSupport = rows;
