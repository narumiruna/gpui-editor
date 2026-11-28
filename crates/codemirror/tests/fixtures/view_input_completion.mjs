import {EditorSelection, EditorState, Prec, StateEffect, StateField, Transaction} from '@codemirror/state';
import {history, redo, redoDepth, undo, undoDepth} from '@codemirror/commands';
import {EditorView, ViewPlugin} from '@codemirror/view';

const marker = StateEffect.define({map: (pos, changes) => changes.mapPos(pos, 1)});
const extensionEvent = StateEffect.define();
const event = tr => tr.annotation(Transaction.userEvent);
const isInput = tr => tr.isUserEvent('input.type') || tr.isUserEvent('input.complete');
const snapshot = state => ({doc: state.doc.toString(), head: state.selection.main.head});
const changes = set => {
    const result = [];
    set.iterChanges((from, to, _fromB, _toB, insert) => result.push([from, to, insert.toString()]));
    return result;
};
const step = tr => ({
    start: snapshot(tr.startState), end: snapshot(tr.state), changes: changes(tr.changes),
    event: event(tr), markers: tr.effects.filter(e => e.is(marker)).map(e => e.value),
    extensions: tr.effects.filter(e => e.is(extensionEvent)).map(e => e.value),
});
const pause = () => new Promise(resolve => setTimeout(resolve, 50));

async function run(mode) {
    const log = {handlers: [], filters: [], extenders: [], fields: [], dispatches: [], observers: []};
    let defaultTransaction;
    const first = EditorView.inputHandler.of((view, from, to, text, getDefault) => {
        log.handlers.push(['peek', from, to, text]);
        defaultTransaction = getDefault();
        if (defaultTransaction !== getDefault() || defaultTransaction.startState !== view.state ||
            defaultTransaction.state.doc.toString() !== 'aXYZb') throw Error('incorrect lazy default');
        return false;
    });
    const second = EditorView.inputHandler.of((view, from, to, text, getDefault) => {
        log.handlers.push(['complete', from, to, text]);
        const base = getDefault();
        if (base !== defaultTransaction) throw Error('default was rebuilt');
        if (mode === 'pass') return false;
        const at = base.state.selection.main.head;
        const completion = base.state.update({
            changes: [{from: 0, insert: '!'}, {from: at, insert: ')'}], selection: {anchor: at + 1},
            effects: marker.of(at + 1), userEvent: 'input.complete',
        });
        view.dispatch([base, completion]);
        return true;
    });
    const last = EditorView.inputHandler.of((_view, from, to, text) => {
        if (mode !== 'pass') throw Error('handler ran after completion');
        log.handlers.push(['last', from, to, text]);
        return false;
    });
    const field = StateField.define({
        create: () => 0,
        update(value, tr) {
            if (isInput(tr)) log.fields.push([event(tr), tr.startState.doc.toString(), tr.newDoc.toString()]);
            return value + 1;
        },
    });
    const observe = (source, update) => {
        if (!update.transactions.some(isInput)) return;
        log.observers.push({
            source, start: snapshot(update.startState), end: snapshot(update.state), changes: changes(update.changes),
            selection: update.selectionSet, steps: update.transactions.map(step),
        });
    };
    const view = new EditorView({
        doc: 'ab', selection: {anchor: 1}, parent: document.body,
        extensions: [
            history(), second, Prec.high(first), Prec.low(last), field,
            EditorState.changeFilter.of(tr => mode !== 'filtered' || !tr.isUserEvent('input.complete')),
            EditorState.transactionFilter.of(tr => {
                if (!isInput(tr)) return tr;
                log.filters.push([event(tr), tr.startState.doc.toString(), tr.newDoc.toString()]);
                if (!tr.isUserEvent('input.type')) return tr;
                const {from, to} = tr.startState.selection.main;
                // Construct the replacement without filtering or extending it within this filter.
                return Transaction.create(tr.startState, tr.startState.changes({from, to, insert: 'XYZ'}),
                    EditorSelection.single(from + 3), [marker.of(from + 3)], tr.annotations, tr.scrollIntoView);
            }),
            EditorState.transactionExtender.of(tr => {
                if (!isInput(tr)) return null;
                log.extenders.push([event(tr), tr.startState.doc.toString(), tr.newDoc.toString()]);
                return {effects: extensionEvent.of(event(tr))};
            }),
            ViewPlugin.fromClass(class { update(update) { observe('plugin', update); } }),
            EditorView.updateListener.of(update => observe('listener', update)),
        ],
        dispatchTransactions(trs, target) {
            if (trs.some(isInput)) {
                if (trs[0] !== defaultTransaction) throw Error('dispatch rebuilt the default');
                log.dispatches.push(trs.map(step));
            }
            target.update(trs);
        },
    });
    const trust = [];
    view.contentDOM.addEventListener('input', e => trust.push([e.isTrusted, e.inputType]));
    view.focus();
    await pause();
    if (view.contentDOM !== document.activeElement || !document.execCommand('insertText', false, 'x')) throw Error('no browser edit');
    await pause();
    if (trust.length !== 1 || !trust[0][0] || log.dispatches.length !== 1 || log.observers.length !== 2) throw Error('unexpected dispatch or notification count');
    const result = {mode, trust, ...log, end: snapshot(view.state), undoDepth: undoDepth(view.state), undo: [], redo: []};
    while (undo(view)) result.undo.push({...snapshot(view.state), depth: undoDepth(view.state)});
    while (redo(view)) result.redo.push({...snapshot(view.state), depth: redoDepth(view.state)});
    view.destroy();
    return result;
}

const output = [];
for (const mode of ['complete', 'filtered', 'pass']) output.push(JSON.stringify(await run(mode)));
globalThis.nativeSupport = output;
