// Capture with capture_browser_fixture.mjs and the codemirror package.
import {basicSetup, minimalSetup, EditorView} from 'codemirror';
import {openSearchPanel, search, SearchQuery, setSearchQuery} from '@codemirror/search';
import {undoDepth} from '@codemirror/commands';

const source = '😀 hit here\r\nThen hit.';
const offsets = [0];
const encoder = new TextEncoder();
let bytes = 0;
for (const [piece] of source.matchAll(/\r\n|[^]/gu)) {
    if (piece.length === 2 && piece !== '\r\n') offsets.push(null);
    offsets.push(bytes += encoder.encode(piece).length);
}
const output = [];
for (const [preset, setup] of [['basic', basicSetup], ['minimal', minimalSetup]]) {
    for (const scope of ['panel', 'editor']) {
        const announcements = [];
        const view = new EditorView({doc: source, parent: document.body, extensions: [setup, search(),
            EditorView.updateListener.of(update => {
                for (const tr of update.transactions)
                    for (const effect of tr.effects)
                        if (effect.is(EditorView.announce)) announcements.push(effect.value);
            }),
        ]});
        openSearchPanel(view);
        view.dispatch({effects: setSearchQuery.of(new SearchQuery({search: 'hit'}))});
        const field = view.dom.querySelector('.cm-search input[name=search]');
        const target = scope === 'panel' ? field : view.contentDOM;
        target.focus();
        const before = view.state.doc;
        const steps = [false, false, false, true].map(shiftKey => {
            const event = new KeyboardEvent('keydown', {key: 'F3', code: 'F3', keyCode: 114,
                shiftKey, bubbles: true, cancelable: true});
            target.dispatchEvent(event);
            const {from, to} = view.state.selection.main;
            return {key: shiftKey ? 'shift-f3' : 'f3', handled: event.defaultPrevented,
                selection: [offsets[from], offsets[to]], announcements: announcements.splice(0),
                focused: document.activeElement === target};
        });
        output.push(JSON.stringify({preset, scope, source, steps, unchanged: view.state.doc === before,
            undoDepth: undoDepth(view.state)}));
        view.destroy();
    }
}
globalThis.nativeSupport = output;
