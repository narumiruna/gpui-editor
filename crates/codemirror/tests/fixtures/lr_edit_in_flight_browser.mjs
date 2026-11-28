// Capture with capture_browser_fixture.mjs and the pinned lang-json and lang-lezer.
import {EditorView} from '@codemirror/view';
import {Language, ensureSyntaxTree, syntaxTree, syntaxTreeAvailable} from '@codemirror/language';
import {json} from '@codemirror/lang-json';
import {lezer} from '@codemirror/lang-lezer';

const records = [];
const now = () => 4_000_000;
Date.now = now;
for (const [grammar, source, extension, insert, limit] of [
    ['json', '[\n' + '1,2,3,4,5,6,7,8,9,\n'.repeat(800) + '0\n]', json(), ' ', 2501],
    ['lezer', '@top Main {\n' + '  "hi"\n'.repeat(1500) + '}', lezer(), '\n', 1201],
]) {
    const parent = document.body.appendChild(document.createElement('div'));
    const view = new EditorView({doc: source, parent, extensions: [extension, EditorView.theme({
        '&': {height: '140px'}, '.cm-scroller': {overflow: 'auto'},
    })]});
    const record = (event, extra = {}) => {
        const tree = syntaxTree(view.state);
        const context = view.state.field(Language.state).context;
        records.push(JSON.stringify({grammar, event, viewport: view.viewport.to,
            through: context.treeLen, pub: tree.length,
            inFlight: context.parse?.parsedPos ?? null, ...extra}));
    };
    try {
        const retained = view.state, original = syntaxTree(retained);
        record('install');
        const requested = 8000;
        let calls = 0;
        Date.now = () => now() + (++calls > limit ? 11 : 0);
        const ensured = ensureSyntaxTree(view.state, requested, 10);
        Date.now = now;
        const progress = view.state.field(Language.state).context.parse?.parsedPos ?? -1;
        if (ensured || progress <= 3000 || progress >= requested) throw Error(`unexpected progress: ${grammar} ${progress}`);
        record('pending', {requested, available: syntaxTreeAvailable(view.state, progress)});
        view.dispatch({changes: {from: 0, insert}});
        record('edit', {insert, retained: syntaxTree(retained) === original,
            available: syntaxTreeAvailable(view.state, progress + insert.length)});
    } finally {
        view.destroy();
        parent.remove();
    }
}
globalThis.nativeSupport = records;
