// Run in a headless browser with an import map for the verified npm archives:
// @codemirror/search 6.7.3, @codemirror/state 6.7.6, @codemirror/view 6.44.0.
// Capture globalThis.nativeSupport as JSONL.
import {Compartment, EditorState, Prec} from '@codemirror/state';
import {EditorView} from '@codemirror/view';
import {closeSearchPanel, getSearchQuery, openSearchPanel, search, SearchQuery, setSearchQuery} from '@codemirror/search';

const events = [];
let serial = 0;
function factory(name, top) {
    return view => {
        const id = `${name}${++serial}`;
        events.push(['create', id, getSearchQuery(view.state).search]);
        const dom = document.createElement('div');
        dom.dataset.searchPanel = id;
        return {
            dom, top,
            mount() { events.push(['mount', id]); },
            update(update) { events.push(['update', id, getSearchQuery(update.state).search]); },
            destroy() { events.push(['destroy', id]); },
        };
    };
}

const a = factory('a', false), b = factory('b', true);
const config = new Compartment();
const view = new EditorView({doc: 'cat dog cat', parent: document.body,
    extensions: [search(), config.of(search({createPanel: a, top: false}))]});
const rows = [];
function record(step) {
    const dom = view.dom.querySelector('[data-search-panel]');
    const placement = dom ? (dom.closest('.cm-panels-top') ? 'top' : 'bottom') : null;
    rows.push({step, placement, events: events.splice(0)});
}
function configure(createPanel, top) {
    view.dispatch({effects: config.reconfigure(search({createPanel, top}))});
}

record('before');
openSearchPanel(view);
record('open_a');
openSearchPanel(view);
record('open_again');
configure(a, false);
record('equivalent_a');
// The custom instance supplies its own `top`, not the search config's `top`.
configure(a, true);
record('change_top');
configure(b, true);
record('change_factory');
view.dispatch({effects: setSearchQuery.of(new SearchQuery({search: 'dog'}))});
record('set_query');
closeSearchPanel(view);
record('close');
openSearchPanel(view);
record('reopen_b');
view.destroy();
record('destroy_view');

function factoryConflict(factories) {
    try {
        EditorState.create({doc: 'source', extensions: factories});
        return false;
    } catch (error) {
        if (!String(error).includes('Config merge conflict for field createPanel')) throw error;
        return true;
    }
}
const duplicate = factoryConflict([search({createPanel: a}), search({createPanel: a})]);
const conflict = factoryConflict([search({createPanel: a}), Prec.high(search({createPanel: b}))]);
if (duplicate || !conflict) throw new Error('unexpected factory conflict policy');
rows.push({step: 'factory_config', duplicate, conflict});
globalThis.nativeSupport = rows.map(row => JSON.stringify(row));
