// Run against the integrity-checked browser import map for the pinned packages.
import {Compartment} from '@codemirror/state';
import {EditorView, showPanel} from '@codemirror/view';
import {closeSearchPanel, getSearchQuery, openSearchPanel, search, searchPanelOpen} from '@codemirror/search';

const slot = new Compartment();
const view = new EditorView({doc: '猫 kitten\n猫', selection: {anchor: 0, head: 1},
    parent: document.body, extensions: [slot.of([])]});
const rows = [];
function record(step) {
    const panel = view.dom.querySelector('.cm-search');
    const renderedPlacement = panel ? (panel.closest('.cm-panels-top') ? 'top' : 'bottom') : null;
    // The constructor supplies a live placement getter. The existing DOM
    // may stay in its old container until the showPanel facet changes.
    const declared = view.state.facet(showPanel).find(Boolean);
    const placement = declared ? (declared(view).top ? 'top' : 'bottom') : null;
    rows.push({step, open: searchPanelOpen(view.state), placement, renderedPlacement,
        query: getSearchQuery(view.state).search, doc: view.state.doc.toString()});
}
record('before');
openSearchPanel(view);
record('open_default');
view.dispatch({effects: slot.reconfigure(search({top: true}))});
record('configure_top_while_open');
closeSearchPanel(view);
record('close_bottom');
openSearchPanel(view);
record('open_top');
view.dispatch({effects: slot.reconfigure(search({top: false}))});
record('configure_bottom_while_open');
closeSearchPanel(view);
record('close_top');
openSearchPanel(view);
record('reopen_bottom');
view.destroy();
globalThis.nativeSupport = rows.map(row => JSON.stringify(row));
