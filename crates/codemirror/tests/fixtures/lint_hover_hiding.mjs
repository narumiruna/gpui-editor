// Run with capture_browser_fixture.mjs, package @codemirror/lint and the verified archives.
import {linter, setDiagnostics} from '@codemirror/lint';
import {EditorView, activateHover, showTooltip} from '@codemirror/view';

const source = 'aa\nbad\nzz';
const diagnostic = {from: 3, to: 6, severity: 'error', message: 'text'};
const rows = [];
for (const config of [[], ['keep'], ['keep', 'defer'], ['defer', 'keep'], ['hide', 'keep']]) {
    for (const operation of ['prefix', 'touch', 'publish', 'delete']) {
        const calls = [];
        const extensions = [linter(null), ...config.map(name => linter(null, {hideOn(tr, from, to) {
            calls.push([name, from, to, tr.startState.doc.toString(), tr.docChanged]);
            return name === 'defer' ? null : name === 'hide';
        }}))];
        const editor = new EditorView({doc: source, extensions, parent: document.body});
        editor.dispatch(setDiagnostics(editor.state, [diagnostic]));
        activateHover(editor, 4, 1);
        const tip = () => editor.state.facet(showTooltip).filter(Boolean).map(t => [t.pos, t.end]);
        if (JSON.stringify(tip()) !== JSON.stringify([[3, 6]])) throw Error('missing initial lint hover');
        calls.length = 0;
        if (operation === 'prefix') editor.dispatch({changes: {from: 0, insert: 'X'}});
        if (operation === 'touch') editor.dispatch({changes: {from: 3, insert: 'X'}});
        if (operation === 'publish') editor.dispatch(setDiagnostics(editor.state, [diagnostic]));
        if (operation === 'delete') editor.dispatch({changes: {from: 2, to: 4}});
        rows.push({source, config, operation, tooltip: tip(), calls});
        editor.destroy();
    }
}
globalThis.nativeSupport = rows.map(row => JSON.stringify(row));
