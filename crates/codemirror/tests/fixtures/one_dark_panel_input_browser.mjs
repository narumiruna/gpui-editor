// Capture with capture_browser_fixture.mjs and @codemirror/search,@codemirror/theme-one-dark.
import {Compartment} from '@codemirror/state';
import {EditorView} from '@codemirror/view';
import {gotoLine, openSearchPanel, search, SearchQuery, setSearchQuery} from '@codemirror/search';
import {oneDarkHighlightStyle, oneDarkTheme} from '@codemirror/theme-one-dark';
import {syntaxHighlighting} from '@codemirror/language';

const slot = new Compartment();
const view = new EditorView({doc: 'needle\nnext', extensions: [search(), slot.of([])], parent: document.body});
const rows = [];
const hex = color => {
    const channels = /^rgb\((\d+), (\d+), (\d+)\)$/.exec(color);
    if (!channels) throw new Error(`unexpected color: ${color}`);
    return channels.slice(1).map(value => Number(value).toString(16).padStart(2, '0')).join('');
};
try {
    openSearchPanel(view);
    view.dispatch({effects: setSearchQuery.of(new SearchQuery({search: 'needle', replace: ' 🦀 ' }))});
    gotoLine(view);
    for (const [mode, extension] of [
        ['uninstalled', []], ['theme', oneDarkTheme],
        ['highlightOnly', syntaxHighlighting(oneDarkHighlightStyle)],
    ]) {
        view.dispatch({effects: slot.reconfigure(extension)});
        for (const [field, name] of [['find', 'search'], ['replace', 'replace'], ['goto', 'line']]) {
            const input = view.dom.querySelector(`input[name=${name}]`);
            if (!input?.matches('.cm-textfield')) throw new Error(`missing field: ${field}`);
            const group = input.closest('.cm-panels');
            const style = getComputedStyle(input);
            const surface = style.backgroundColor === 'rgba(0, 0, 0, 0)'
                ? getComputedStyle(group).backgroundColor : style.backgroundColor;
            rows.push({mode, field, surface: hex(surface), text: hex(style.color),
                border: hex(style.borderTopColor), width: style.borderTopWidth,
                placement: group.matches('.cm-panels-top') ? 'top' : 'bottom', value: input.value});
        }
    }
} finally {
    view.destroy();
}
globalThis.nativeSupport = rows.map(row => JSON.stringify(row));
