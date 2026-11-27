// Run with capture_browser_fixture.mjs and the verified @codemirror/lint archives.
import {linter, nextDiagnostic, previousDiagnostic, setDiagnostics} from '@codemirror/lint';
import {EditorState} from '@codemirror/state';
import {EditorView, hasHoverTooltips} from '@codemirror/view';

const source = '```text\r\n𐐀 abc x\r\nother\r\n```\r\n';
const normalized = source.replaceAll('\r\n', '\n');
const encoder = new TextEncoder();
const utf16 = text => normalized.indexOf(text);
const bytes = position => encoder.encode(normalized.slice(0, position).replaceAll('\n', '\r\n')).length;
const first = utf16('abc');
const second = utf16('other');
const diagnostics = [
    {from: first, to: first + 3, message: 'range', severity: 'warning'},
    {from: first, to: first + 2, message: 'overlap', severity: 'error'},
    {from: second, to: second, message: 'point', severity: 'error'},
];
const view = new EditorView({parent: document.body, state: EditorState.create({doc: source,
    extensions: [linter(null, {tooltipFilter: items => [...items].reverse()})],
})});
view.dispatch(setDiagnostics(view.state, diagnostics));
const steps = [];
function record(name, command) {
    view.measure();
    const {anchor, head} = view.state.selection.main;
    steps.push({name, command, selection: [bytes(anchor), bytes(head)],
        tooltip: hasHoverTooltips(view.state)
            ? [...view.dom.querySelectorAll('.cm-tooltip-lint .cm-diagnosticText')].map(item => item.textContent)
            : null});
}
record('next', nextDiagnostic(view));
view.dispatch({selection: {anchor: first + 1}});
record('inside');
view.dispatch({selection: {anchor: first + 4}});
record('outside');
view.dispatch({selection: {anchor: first + 1}});
record('return');
record('next-from-overlap', nextDiagnostic(view));
record('wrap-back-to-point', previousDiagnostic(view));
view.dispatch(setDiagnostics(view.state, diagnostics));
record('publication');
view.dispatch({selection: {anchor: 0}});
record('again', nextDiagnostic(view));
view.dispatch({changes: {from: utf16(' x'), insert: ' '}});
record('edit');
view.destroy();
globalThis.nativeSupport = [JSON.stringify({source, steps})];
