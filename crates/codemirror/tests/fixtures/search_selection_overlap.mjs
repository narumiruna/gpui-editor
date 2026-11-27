// Run with capture_browser_fixture.mjs and the verified @codemirror/search archives.
import {EditorState} from '@codemirror/state';
import {EditorView} from '@codemirror/view';
import {SearchQuery, highlightSelectionMatches, openSearchPanel, search, setSearchQuery} from '@codemirror/search';

const source = 'é cat catx cat scatter';
const textBytes = (pos) => new TextEncoder().encode(source.slice(0, pos)).length;
const result = [];
for (const [query, reversed] of [['atx', false], ['scat', false], ['nothing', false], ['atx', true], ['scat', true], ['at', false]]) {
    const extensions = reversed ? [highlightSelectionMatches(), search()] : [search(), highlightSelectionMatches()];
    const view = new EditorView({parent: document.body, state: EditorState.create({doc: source, extensions, selection: {anchor: 2, head: 5}})});
    openSearchPanel(view);
    view.dispatch({effects: setSearchQuery.of(new SearchQuery({search: query}))});
    view.measure();
    const ranges = selector => [...view.contentDOM.querySelectorAll(selector)].map(node => {
        const walker = document.createTreeWalker(node, NodeFilter.SHOW_TEXT);
        const start = walker.nextNode();
        let end = start;
        for (let next; next = walker.nextNode();) end = next;
        const from = view.posAtDOM(start, 0);
        const to = view.posAtDOM(end, end.length);
        return [textBytes(from), textBytes(to), getComputedStyle(node).backgroundColor];
    });
    result.push({source, query, reversed, selection: [textBytes(2), textBytes(5)], selectedTextMatches: ranges('.cm-selectionMatch'), find: ranges('.cm-searchMatch')});
    view.destroy();
}
globalThis.nativeSupport = result.map(row => JSON.stringify(row));
