// Run with the integrity-checked upstream browser import map.
import {EditorState, Transaction} from '@codemirror/state';
import {EditorView} from '@codemirror/view';
import {
    acceptCompletion, autocompletion, completionStatus, currentCompletions, pickedCompletion,
    selectedCompletion, snippetCompletion, startCompletion,
} from '@codemirror/autocomplete';

const wait = ms => new Promise(resolve => setTimeout(resolve, ms));
async function until(test) {
    for (let i = 0; i < 200; i++) {
        if (test()) return;
        await wait(10);
    }
    throw Error('Completion did not reach the expected state');
}
const bytes = text => new TextEncoder().encode(text).length;
const query = cx => [cx.state.doc.toString(), bytes(cx.state.sliceDoc(0, cx.pos)), cx.explicit];
const labels = view => currentCompletions(view.state).map(option => option.displayLabel ?? option.label);
const read = view => [completionStatus(view.state), labels(view), selectedCompletion(view.state)?.label ?? null];
const rows = [];
const dateNow = Date.now;
let now = 0;
Date.now = () => now;
try {
    let late, oldContext, abortCount = 0;
    const pick = {label: 'foo', apply: 'foo.'};
    const observation = {case: 'chain', primary: [], secondary: [], aborted: false, reused: [], picks: []};
    const view = new EditorView({
        doc: 'é foo', selection: {anchor: 5}, parent: document.body,
        extensions: [autocompletion({
            activateOnTyping: false, activateOnTypingDelay: 25, updateSyncTime: 10,
            activateOnCompletion: item => item === pick,
            override: [cx => {
                observation.primary.push(query(cx));
                return cx.explicit ? {
                    from: 2, options: [pick],
                    validFor: () => { observation.reused.push('validFor'); return true; },
                    map: result => { observation.reused.push('map'); return result; },
                    update: result => { observation.reused.push('update'); return result; },
                } : {from: cx.pos, options: [{label: 'bar'}]};
            }, cx => {
                observation.secondary.push(query(cx));
                if (!cx.explicit) return null;
                oldContext = cx;
                cx.addEventListener('abort', () => { abortCount++; });
                return new Promise(resolve => { late = resolve; });
            }],
        }), EditorView.updateListener.of(update => {
            for (const tr of update.transactions) {
                const item = tr.annotation(pickedCompletion);
                if (item) observation.picks.push([item.label, tr.annotation(Transaction.userEvent)]);
            }
        })],
    });
    startCompletion(view);
    await until(() => labels(view).length === 1);
    now = 74;
    observation.early = acceptCompletion(view);
    now = 75;
    observation.accepted = acceptCompletion(view);
    observation.pending = read(view);
    observation.aborted = oldContext.aborted && abortCount === 1;
    observation.acceptedWhilePending = acceptCompletion(view);
    late({from: 2, options: [{label: 'foo late'}]});
    await until(() => completionStatus(view.state) === 'active');
    observation.ready = read(view);
    // The new list retains the previous interaction timestamp.
    observation.nextAccepted = acceptCompletion(view);
    observation.final = [view.state.doc.toString(), ...read(view)];
    rows.push(observation);
    view.destroy();

    for (const mode of ['event_only', 'annotation_only', 'subtype', 'snippet', 'commit', 'pointer', 'no_dispatch', 'read_only']) {
        now = 0;
        const calls = [];
        const item = mode === 'snippet' ? snippetCompletion('${field}', {label: 'foo'})
            : {label: 'foo', apply: 'foo.', commitCharacters: [';']};
        const view = new EditorView({
            doc: 'é foo', selection: {anchor: 5}, parent: document.body,
            extensions: [EditorState.readOnly.of(mode === 'read_only'), autocompletion({
                activateOnTyping: false, activateOnTypingDelay: 25,
                activateOnCompletion: () => true,
                override: [cx => {
                    calls.push(query(cx));
                    return cx.explicit ? {from: 2, options: [item]} : null;
                }],
            })],
            dispatchTransactions(trs, view) {
                if (mode !== 'no_dispatch' || !trs.some(tr => tr.isUserEvent('input.complete'))) view.update(trs);
            },
        });
        startCompletion(view);
        await until(() => labels(view).length === 1);
        let early;
        if (['event_only', 'annotation_only', 'subtype'].includes(mode)) {
            view.dispatch({changes: {from: 5, insert: '.'}, selection: {anchor: 6},
                userEvent: mode === 'annotation_only' ? 'input.custom' : 'input.complete.subtype',
                annotations: mode === 'event_only' ? undefined : pickedCompletion.of(item)});
        } else if (mode === 'commit') {
            // Commit characters precede insertion of the key and do not wait for the delay.
            view.contentDOM.dispatchEvent(new KeyboardEvent('keydown', {key: ';', bubbles: true}));
        } else if (mode === 'pointer') {
            early = acceptCompletion(view);
            const entry = document.querySelector('.cm-tooltip-autocomplete li');
            if (!entry) throw Error('Expected a visible completion');
            entry.dispatchEvent(new MouseEvent('mousedown', {bubbles: true, cancelable: true}));
        } else {
            now = 75;
            acceptCompletion(view);
        }
        const pending = completionStatus(view.state) === 'pending';
        if (pending) await until(() => completionStatus(view.state) === null);
        rows.push({case: mode, pending, calls, doc: view.state.doc.toString(), ...(mode === 'pointer' ? {early} : {})});
        view.destroy();
    }

    const lists = [
        [{label: 'same', sortText: 'z', displayLabel: 'first same'},
         {label: 'alpha', sortText: 'm'}, {label: 'boosted', sortText: 'a', boost: 5},
         {label: 'fixed', sortText: 'z', section: {name: 'Section', rank: 1}}],
        [{label: 'same', sortText: 'a', displayLabel: 'last same', info: 'separate'},
         {label: 'value', sortText: 'z', displayLabel: 'plain value'},
         {label: 'value', sortText: 'z', displayLabel: 'rich value', info: 'preferred'},
         {label: 'beta', sortText: 'm'}],
        [{label: 'pin1', sortText: 'a'}, {label: 'pin2', sortText: 'z'}],
    ];
    const ranked = new EditorView({parent: document.body, extensions: [autocompletion({
        compareCompletions: (a, b) => b.sortText.localeCompare(a.sortText),
        override: lists.map((options, i) => () => ({from: 0, filter: i !== 2, options})),
    })]});
    startCompletion(ranked);
    await until(() => completionStatus(ranked.state) === 'active');
    rows.push({case: 'compare', labels: labels(ranked)});
    ranked.destroy();
} finally {
    Date.now = dateNow;
}
globalThis.nativeSupport = rows.map(row => JSON.stringify(row));
