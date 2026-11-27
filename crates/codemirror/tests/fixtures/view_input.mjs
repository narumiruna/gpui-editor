import {EditorSelection, EditorState, Prec, StateEffect, Transaction} from '@codemirror/state';
import {history, undo, undoDepth} from '@codemirror/commands';
import {EditorView} from '@codemirror/view';

const flag = StateEffect.define();
const ranges = selection => selection.ranges.map(({anchor, head}) => [anchor, head]);
const describe = tr => ({
    changes: (() => {
        const result = [];
        tr.changes.iterChanges((from, to, _fromB, _toB, insert) => result.push([from, to, insert.toString()]));
        return result;
    })(),
    selection: tr.selection ? ranges(tr.selection) : null,
    doc: tr.newDoc.toString(),
    ranges: ranges(tr.newSelection),
    main: tr.newSelection.mainIndex,
    event: tr.annotation(Transaction.userEvent) ?? null,
    scroll: tr.scrollIntoView,
    effects: tr.effects.map(effect => effect.is(flag) ? effect.value : 'unexpected effect'),
});
const pause = () => new Promise(resolve => requestAnimationFrame(() => requestAnimationFrame(resolve)));
const cases = [
    {
        name: 'ordered fallthrough and cached default across handlers and dispatch',
        doc: 'a🦀b cd', ranges: [[3, 1], [6, 6]],
        handlers: [['regular', 'peek'], ['last', 'pass', 'low'], ['first', 'peek', 'high'], ['regular', 'peek']],
        ops: [{mode: 'selection', text: 'é'}, {mode: 'selection', text: '🙂'}],
    },
    {
        name: 'first handled accepts borrowed default',
        doc: 'ab', ranges: [[1, 1]],
        handlers: [['first', 'peek'], ['accept', 'accept'], ['later', 'unexpected']],
        ops: [{mode: 'selection', text: 'z'}],
    },
    {
        name: 'first handled consumes without default or dispatch',
        doc: 'ab', ranges: [[1, 1]],
        handlers: [['later', 'unexpected'], ['consume', 'consume', 'high']],
        ops: [{mode: 'selection', text: 'z'}],
    },
    {
        name: 'first handled supplies an effectful alternative',
        doc: 'Xxy', ranges: [[2, 1]],
        handlers: [['pass', 'pass', 'high'], ['custom', 'custom'], ['later', 'unexpected', 'low']],
        ops: [{mode: 'selection', text: 'c', undo: true}],
    },
    {
        name: 'explicit backspace affects the primary edit only',
        doc: 'a b🦀c', ranges: [[1, 1], [5, 5]], main: 1,
        handlers: [['regular', 'peek']],
        ops: [{mode: 'explicit', range: [3, 5], selection: [3, 3], text: '', command: 'delete'}],
    },
    {
        name: 'multiline insertion without handlers',
        doc: 'αz', ranges: [[1, 1]], handlers: [],
        ops: [{mode: 'selection', text: '1\n2'}],
    },
];

const output = [];
for (const spec of cases) {
    let observation, remembered;
    const handlers = new Map();
    const extensions = [EditorState.allowMultipleSelections.of(true), history({newGroupDelay: 0})];
    for (const [name, action, precedence] of spec.handlers) {
        if (!handlers.has(name)) {
            const handler = EditorView.inputHandler.of((view, from, to, text, getDefault) => {
                observation.calls.push([name, from, to, text, view.state.doc.toString(), ranges(view.state.selection)]);
                if (action === 'unexpected') throw Error('handler called after the first handled result');
                if (action === 'custom') {
                    view.dispatch({changes: {from, to, insert: text.toUpperCase()}, selection: {anchor: from + text.length},
                        effects: flag.of('accepted'), userEvent: 'input.override'});
                    return true;
                }
                if (action === 'consume') return true;
                if (action === 'pass') return false;
                const tr = getDefault();
                if (tr !== getDefault() || remembered && remembered !== tr || tr.startState !== view.state) {
                    throw Error('default was not cached against the original state');
                }
                remembered = tr;
                observation.previews.push(describe(tr));
                if (action === 'accept') view.dispatch(tr);
                return action === 'accept';
            });
            handlers.set(name, precedence ? Prec[precedence](handler) : handler);
        }
        extensions.push(handlers.get(name));
    }
    const view = new EditorView({
        doc: spec.doc,
        selection: EditorSelection.create(spec.ranges.map(([anchor, head]) => EditorSelection.range(anchor, head)), spec.main ?? 0),
        parent: document.body,
        extensions,
        dispatchTransactions(trs, view) {
            for (const tr of trs) {
                if (observation) {
                    if (remembered && tr.isUserEvent('input.type') && tr !== remembered) throw Error('dispatch did not reuse default');
                    observation.dispatches.push(describe(tr));
                }
            }
            view.update(trs);
        },
    });
    let inputEvents = [];
    view.contentDOM.addEventListener('input', event => inputEvents.push([event.isTrusted, event.inputType]));
    view.focus();
    await pause();
    if (document.activeElement !== view.contentDOM) throw Error('editor does not have focus');
    const results = [];
    for (const op of spec.ops) {
        observation = {calls: [], previews: [], dispatches: []};
        remembered = undefined;
        inputEvents = [];
        if (!document.execCommand(op.command ?? 'insertText', false, op.text)) throw Error('browser did not edit content');
        await pause();
        if (!inputEvents.length || !inputEvents.every(([trusted]) => trusted)) throw Error('expected trusted input');
        Object.assign(observation, {
            trustedInput: inputEvents.map(([, type]) => type),
            doc: view.state.doc.toString(),
            ranges: ranges(view.state.selection),
            main: view.state.selection.mainIndex,
            undoDepth: undoDepth(view.state),
        });
        const result = observation;
        observation = null;
        if (op.undo) {
            if (!undo(view)) throw Error('expected one undoable edit');
            result.undo = {doc: view.state.doc.toString(), ranges: ranges(view.state.selection), depth: undoDepth(view.state)};
        }
        results.push(result);
    }
    view.destroy();
    output.push(JSON.stringify({...spec, results}));
}
globalThis.nativeSupport = output;
