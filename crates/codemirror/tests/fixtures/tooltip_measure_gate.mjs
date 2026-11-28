// Run with capture_browser_fixture.mjs and the pinned @codemirror/view archives.
import {Compartment} from '@codemirror/state';
import {EditorView, repositionTooltips, showTooltip, tooltips} from '@codemirror/view';

const slot = new Compartment();
let anchorReads = 0, spaceReads = 0, clipped = false;
const tip = {
    pos: 0,
    create() {
        const dom = document.createElement('div');
        dom.className = 'test-tooltip';
        dom.style.cssText = 'width:40px;height:20px';
        return {dom, getCoords() {
            anchorReads++;
            return {left: 60, right: 60, top: clipped ? 4 : 60, bottom: clipped ? 20 : 76};
        }};
    },
};
const view = new EditorView({doc: 'abc', parent: document.body, extensions: [
    EditorView.theme({'&': {position: 'absolute', left: '32px', width: '160px', height: '96px'}}),
    tooltips({tooltipSpace() {
        spaceReads++;
        return {left: 0, top: 0, right: 240, bottom: 180};
    }}),
    slot.of(showTooltip.of(tip)),
]});
const rows = [];
let wasActive = true;
for (const [step, active, inView, clipAnchor] of [
    ['visible', true, true, false],
    ['clipped anchor', true, true, true],
    ['offscreen editor', true, false, false],
    ['back in view', true, true, false],
    ['no tooltip', false, true, false],
    ['reopened', true, true, false],
]) {
    clipped = clipAnchor;
    view.dom.style.top = inView ? '32px' : '-1000px';
    if (active !== wasActive) {
        view.dispatch({effects: slot.reconfigure(showTooltip.of(active ? tip : null))});
        wasActive = active;
    }
    // Flush measurements queued by the transition before the explicit request.
    view.measure();
    if (view.inView !== inView) throw Error(`unexpected view visibility: ${step}`);
    anchorReads = spaceReads = 0;
    repositionTooltips(view);
    view.measure();
    const box = view.dom.querySelector('.test-tooltip');
    rows.push({step, active, inView, clipAnchor, anchorReads, spaceReads,
        visible: box != null && box.style.top !== '-10000px'});
}
view.destroy();
globalThis.nativeSupport = rows.map(row => JSON.stringify(row));
