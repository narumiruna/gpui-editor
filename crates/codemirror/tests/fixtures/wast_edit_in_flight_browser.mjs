// Capture with capture_browser_fixture.mjs, Chrome, and SHA-512-pinned @codemirror/lang-wast.
import {EditorView} from '@codemirror/view';
import {Language, ensureSyntaxTree, syntaxTree, syntaxTreeAvailable} from '@codemirror/language';
import {wast} from '@codemirror/lang-wast';

const now = () => 4_000_000;
Date.now = now;
const parent = document.body.appendChild(document.createElement('div'));
const view = new EditorView({
    doc: '(func $f)\n'.repeat(1_200),
    parent,
    extensions: [wast(), EditorView.theme({
        '&': {height: '140px'},
        '.cm-scroller': {overflow: 'auto'},
    })],
});
const rows = [];
function record(event, extra = {}) {
    const {state, viewport} = view;
    const {context} = state.field(Language.state);
    const tree = syntaxTree(state);
    rows.push(JSON.stringify({
        event,
        vp: viewport.to,
        through: context.treeLen,
        pub: tree.length,
        inFlight: context.parse?.parsedPos ?? null,
        full: syntaxTreeAvailable(state),
        visible: syntaxTreeAvailable(state, viewport.to),
        error0: tree.topNode.firstChild?.type.isError ?? false,
        ...extra,
    }));
}
try {
    const retained = view.state;
    const original = syntaxTree(retained);
    record('install');
    const requested = 8_000;
    let calls = 0;
    // Stop ensure before it stages the requested prefix. No idle callback runs.
    Date.now = () => now() + (++calls > 2_401 ? 11 : 0);
    const ensured = ensureSyntaxTree(view.state, requested, 10);
    Date.now = now;
    const progress = view.state.field(Language.state).context.parse.parsedPos;
    record('pending', {requested, ensured: ensured !== null, available: syntaxTreeAvailable(view.state, progress)});

    view.dispatch({changes: {from: 0, insert: ')'}});
    record('edit', {retained: syntaxTree(retained) === original, available: syntaxTreeAvailable(view.state, progress + 1)});
} finally {
    view.destroy();
    parent.remove();
}
globalThis.nativeSupport = rows;
