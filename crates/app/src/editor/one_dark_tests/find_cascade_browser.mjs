// Replay with capture_browser_fixture.mjs and @codemirror/theme-one-dark,@codemirror/search.
import {EditorSelection, EditorState} from '@codemirror/state';
import {EditorView, drawSelection} from '@codemirror/view';
import {SearchQuery, openSearchPanel, search, setSearchQuery} from '@codemirror/search';
import {oneDarkTheme} from '@codemirror/theme-one-dark';

const source = 'Find hit, hit, hit, hits.';
const hits = [...source.matchAll(/hit/g)].map(({index}) => [index, index + 3]);
const view = new EditorView({
    parent: document.body, doc: source,
    selection: EditorSelection.create([
        EditorSelection.range(...hits[0]),
        EditorSelection.range(hits[1][1], hits[1][0]),
        EditorSelection.range(hits[3][0] + 1, hits[3][1] + 1),
    ], 1),
    extensions: [EditorState.allowMultipleSelections.of(true), oneDarkTheme, drawSelection(), search()],
});
openSearchPanel(view);
view.dispatch({effects: setSearchQuery.of(new SearchQuery({search: 'hit'}))});

function hex(css) {
    const match = /^rgba?\((\d+), (\d+), (\d+)(?:, ([\d.]+))?\)$/.exec(css);
    if (!match) throw new Error(`Not an RGB color: ${css}`);
    return [...match.slice(1, 4).map(Number), Math.round(Number(match[4] ?? 1) * 255)]
        .map(part => part.toString(16).padStart(2, '0')).join('');
}

const rows = [];
view.focus();
for (const blur of [false, true]) {
    if (blur) view.contentDOM.blur();
    view.measure();
    const selectionLayer = view.scrollDOM.querySelector('.cm-selectionLayer');
    const selections = [...selectionLayer.querySelectorAll('.cm-selectionBackground')];
    const marks = [...view.contentDOM.querySelectorAll('.cm-searchMatch')];
    if (marks.length !== 4 || selections.length !== 3) throw new Error('Missing paint boxes');
    const find = marks.at(-1).getBoundingClientRect();
    const selected = selections.at(-1).getBoundingClientRect();
    const findStyle = getComputedStyle(marks.at(-1));
    const exactSelections = selections.slice(0, 2).map((selection, index) => {
        const box = selection.getBoundingClientRect();
        const hit = marks[index].getBoundingClientRect();
        return {
            // Selection boxes can round the measured text edges independently.
            aligned: Math.abs(box.left - hit.left) <= 1 && Math.abs(box.right - hit.right) <= 1
                && hit.top < box.bottom && box.top < hit.bottom,
            background: hex(getComputedStyle(selection).backgroundColor),
        };
    });
    rows.push({
        focused: view.dom.classList.contains('cm-focused'),
        selectionLayerZ: getComputedStyle(selectionLayer).zIndex,
        contentZ: getComputedStyle(view.contentDOM).zIndex,
        scrollerZ: getComputedStyle(view.scrollDOM).zIndex,
        exactSelections,
        partialOverlap: find.left < selected.left && selected.left < find.right
            && find.right < selected.right && find.top < selected.bottom && selected.top < find.bottom,
        selection: hex(getComputedStyle(selections.at(-1)).backgroundColor),
        searchMatch: hex(findStyle.backgroundColor),
        selectedSearchMatch: hex(getComputedStyle(marks[0]).backgroundColor),
        outline: hex(findStyle.outlineColor),
        outlineWidth: Number.parseFloat(findStyle.outlineWidth),
        selectedHits: marks.map(mark => mark.classList.contains('cm-searchMatch-selected')),
    });
}
view.destroy();
globalThis.nativeSupport = rows.map(row => JSON.stringify(row));
