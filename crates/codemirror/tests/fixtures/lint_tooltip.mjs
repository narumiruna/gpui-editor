// Run with capture_browser_fixture.mjs, package @codemirror/lint and the verified archives.
import {diagnosticCount, linter, setDiagnostics} from '@codemirror/lint';
import {Compartment, EditorState} from '@codemirror/state';
import {EditorView, activateHover, closeHoverTooltips, hasHoverTooltips} from '@codemirror/view';

const source = 'é abc\r\nxy';
const offsets = [0, 2, 3, 4, 5, 6, 8, 9, 10];
const specs = [
    {from: 2, to: 5, severity: 'error', message: 'outer'},
    {from: 3, to: 4, severity: 'warning', message: 'inner'},
    {from: 4, to: 4, severity: 'info', message: 'point'},
    {from: 6, to: 8, severity: 'hint', message: 'tail'},
    {from: 0, to: 1, severity: 'error', message: 'invisible'},
];
const calls = [];
const add = (ds, state) => {
    calls.push(['add', ds.map(d => d.message), state.doc.length]);
    return ds.map(d => ({...d, message: `${d.message}!`}));
};
const reorder = (ds, state) => {
    calls.push(['reverse', ds.map(d => d.message), state.doc.length]);
    return ds.slice().reverse();
};
const clear = (ds, state) => {
    calls.push(['clear', ds.map(d => d.message), state.doc.length]);
    return [];
};
const filters = {add, reverse: reorder, clear};
const slot = new Compartment();
const editor = new EditorView({parent: document.body, state: EditorState.create({doc: source,
    extensions: [linter(null, {markerFilter: ds => ds.filter(d => d.message !== 'invisible')}), slot.of([])],
})});
editor.dispatch(setDiagnostics(editor.state, specs));
const rows = [];
for (const configuration of [[], ['add', 'add', 'reverse', 'add'], ['clear']]) {
    editor.dispatch({effects: slot.reconfigure(configuration.map(name => linter(null, {tooltipFilter: filters[name]})))});
    const outcomes = [];
    for (const [at, side] of [[1, -1], [2, -1], [2, 1], [3, -1], [3, 1], [4, -1], [4, 1], [5, -1], [5, 1], [6, 1], [8, -1]]) {
        editor.dispatch({effects: closeHoverTooltips});
        calls.length = 0;
        activateHover(editor, at, side);
        editor.measure();
        outcomes.push({at: offsets[at], side, active: hasHoverTooltips(editor.state),
            messages: [...editor.dom.querySelectorAll('.cm-tooltip-lint .cm-diagnosticText')].map(node => node.textContent),
            calls: calls.slice()});
    }
    rows.push({source, configuration, markerCount: diagnosticCount(editor.state), outcomes});
}
editor.destroy();
globalThis.nativeSupport = rows.map(row => JSON.stringify(row));
