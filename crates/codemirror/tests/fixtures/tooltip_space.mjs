// Run with capture_browser_fixture.mjs, package @codemirror/view and the verified archives.
import {Compartment, EditorState} from '@codemirror/state';
import {EditorView, repositionTooltips, showTooltip, tooltips} from '@codemirror/view';

const slot = new Compartment();
const trace = [];
const dynamic = tooltips({tooltipSpace(view) {
    const head = view.state.selection.main.head;
    const width = Math.round(view.dom.getBoundingClientRect().width);
    const box = {left: 8 + head, top: 0, right: width - 10, bottom: 180};
    trace.push({name: 'dynamic', box});
    return box;
}});
const fallback = tooltips({tooltipSpace() {
    const box = {left: 4, top: 1, right: 90, bottom: 120};
    trace.push({name: 'fallback', box});
    return box;
}});
const view = new EditorView({doc: 'abc', parent: document.body, extensions: [
    slot.of(dynamic), fallback,
    showTooltip.of({pos: 0, create() {
        const dom = document.createElement('div');
        dom.textContent = 'tooltip';
        dom.style.cssText = 'width:40px;height:20px';
        return {dom};
    }}),
]});
const rows = [];
for (const [at, width, enabled] of [[0, 160, true], [1, 180, true], [2, 200, false], [2, 220, true]]) {
    view.dom.style.width = `${width}px`;
    view.dispatch({selection: {anchor: at}, effects: slot.reconfigure(enabled ? dynamic : [])});
    trace.length = 0;
    repositionTooltips(view);
    view.measure();
    if (!trace.length) throw Error('tooltip geometry was not measured');
    rows.push({at, width, enabled, ...trace.at(-1)});
}
view.destroy();
globalThis.nativeSupport = rows.map(row => JSON.stringify(row));
