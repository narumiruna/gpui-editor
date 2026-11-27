// Run in a browser with an import map for the verified npm archives:
// @codemirror/search 6.7.3, @codemirror/state 6.7.6, @codemirror/view 6.44.0.
// Capture globalThis.nativeSupport as JSONL.
import {EditorSelection, EditorState, StateEffect, Text} from '@codemirror/state';
import {EditorView} from '@codemirror/view';
import {findNext, findPrevious, replaceAll, replaceNext, search, SearchQuery, setSearchQuery} from '@codemirror/search';

const commands = {findNext, findPrevious, replaceAll, replaceNext};
const custom = StateEffect.define();
const encoder = new TextEncoder();
const byte = (doc, pos) => encoder.encode(doc.sliceString(0, pos)).length;
const range = (doc, sel) => [byte(doc, sel.anchor), byte(doc, sel.head)];
const cases = [
    {id: 'default', callback: null},
    {id: 'next'},
    {id: 'previous-with-custom-effect', command: 'findPrevious', callback: 'custom'},
    {id: 'replace-selects-first', command: 'replaceNext', selection: [0, 0]},
    {id: 'replace-with-longer-text', command: 'replaceNext', proposed: '😀 a🦀bc hit'},
    {id: 'reject-replacement', command: 'replaceNext', proposed: '😀 a🦀bc hit', filter: 'reject'},
    {id: 'drop-scroll-effect', filter: 'drop'},
    {id: 'no-match', doc: '😀 miss', selection: [0, 0]},
    {id: 'no-following-match', command: 'replaceNext', doc: '😀 hit'},
    {id: 'replace-all', command: 'replaceAll'},
    {id: 'read-only', command: 'replaceNext', readOnly: true},
];

globalThis.nativeSupport = cases.map(spec => {
    const {id, command = 'findNext', callback = 'scroll', doc = '😀 hit hit',
        selection = [3, 6], proposed = doc, filter = null, readOnly = false} = spec;
    const replacement = 'a🦀bc';
    const proposedDoc = Text.of(proposed.split('\n'));
    const calls = [];
    const config = callback ? {
        scrollToMatch(sel, view) {
            const target = range(proposedDoc, sel);
            const {doc, selection} = view.state;
            calls.push([doc.toString(), range(doc, selection.main), target]);
            return callback === 'custom' ? custom.of(target) :
                EditorView.scrollIntoView(sel, {x: 'start', y: 'center', xMargin: 12, yMargin: 24});
        },
    } : undefined;
    const extensions = [search(config), EditorState.readOnly.of(readOnly)];
    if (filter === 'reject') extensions.push(EditorState.changeFilter.of(tr => !tr.docChanged));
    if (filter === 'drop') extensions.push(EditorState.transactionFilter.of(tr => {
        if (!tr.isUserEvent('select.search')) return tr;
        return {selection: tr.selection, userEvent: 'select.search',
            effects: tr.effects.filter(effect => effect.is(EditorView.announce))};
    }));

    let update = null;
    const view = new EditorView({doc, selection: EditorSelection.single(...selection), extensions,
        parent: document.body, dispatchTransactions(trs, view) {
            view.update(trs);
            const [tr] = trs;
            const event = tr.isUserEvent('select.search') ? 'select.search' :
                tr.isUserEvent('input.replace.all') ? 'input.replace.all' :
                tr.isUserEvent('input.replace') ? 'input.replace' : null;
            if (!event) return;
            const changes = [];
            tr.changes.iterChanges((from, to, _from, _to, insert) => {
                changes.push([byte(tr.startState.doc, from), byte(tr.startState.doc, to), insert.toString()]);
            });
            update = {doc: tr.newDoc.toString(), selection: tr.selection ? range(tr.newDoc, tr.selection.main) : null,
                changes, effects: tr.effects.map(effect => {
                    if (effect.is(EditorView.announce)) return ['announce', effect.value];
                    if (effect.is(custom)) return ['custom', effect.value];
                    const {range: target, x, y, xMargin, yMargin} = effect.value;
                    return ['scroll', range(tr.newDoc, target), x, y, xMargin, yMargin];
                }), event, scrollFlag: tr.scrollIntoView};
        }});
    const before = range(view.state.doc, view.state.selection.main);
    view.dispatch({effects: setSearchQuery.of(new SearchQuery({search: 'hit', replace: replacement}))});
    const handled = commands[command](view);
    view.destroy();
    return JSON.stringify({id, command, callback, doc, before, replacement, filter, readOnly,
        expected: {handled, calls, update}});
});
