// Capture with capture_browser_fixture.mjs and @codemirror/search,@codemirror/theme-one-dark.
import {Compartment} from '@codemirror/state';
import {openSearchPanel, search} from '@codemirror/search';
import {oneDarkTheme} from '@codemirror/theme-one-dark';
import {EditorView, showDialog} from '@codemirror/view';

function color(value) {
    const match = /^rgb\((\d+), (\d+), (\d+)\)$/.exec(value);
    if (!match) throw new Error(`expected opaque RGB: ${value}`);
    return match.slice(1).map(part => Number(part).toString(16).padStart(2, '0')).join('');
}
function chrome(element) {
    const style = getComputedStyle(element);
    const bottom = Number.parseFloat(style.borderBottomWidth);
    if (['Top', 'Left', 'Right'].some(edge => Number.parseFloat(style[`border${edge}Width`]))
        || (bottom && style.borderBottomStyle !== 'solid')) {
        throw new Error('unexpected panel border');
    }
    return {
        fill: style.backgroundColor === 'rgba(0, 0, 0, 0)' ? null : color(style.backgroundColor),
        text: color(style.color),
        border: bottom ? color(style.borderBottomColor) : null,
        bottom,
    };
}

const slot = new Compartment();
const override = EditorView.theme({'&': {color: '#304050'}});
const view = new EditorView({
    doc: 'needle\nnext', parent: document.body, extensions: [search({top: true}), slot.of([])],
});
const rows = [];
try {
    openSearchPanel(view);
    // Use the stock dialog constructor with Go to line's options and top placement.
    // The stock Go to line command opens at the bottom.
    showDialog(view, {
        top: true, class: 'cm-goto-line', label: 'Go to line',
        input: {type: 'text', name: 'line', value: '1'}, focus: true, submitLabel: 'go',
    });
    const group = view.dom.querySelector('.cm-panels-top');
    const [find, go] = group.children;
    const line = go.querySelector('input[name=line]');
    if (group.children.length !== 2 || !find.matches('.cm-search') || !go.matches('.cm-dialog.cm-goto-line')) {
        throw new Error('expected the live Search and Go to line panels in order');
    }
    for (const [mode, extension] of [
        ['native', []], ['theme', oneDarkTheme], ['override', [override, oneDarkTheme]], ['removed', []],
    ]) {
        view.dispatch({effects: slot.reconfigure(extension)});
        if (view.dom.querySelector('.cm-panels-top') !== group || group.children[0] !== find
            || group.children[1] !== go || document.activeElement !== line) {
            throw new Error(`panel identity or focus changed: ${mode}`);
        }
        const {top, bottom} = group.getBoundingClientRect();
        const findRect = find.getBoundingClientRect();
        const goRect = go.getBoundingClientRect();
        if (top !== findRect.top || findRect.bottom !== goRect.top || bottom !== view.scrollDOM.getBoundingClientRect().top) {
            throw new Error(`panels are not contiguous with the editor body: ${mode}`);
        }
        const control = (panel, selector) => {
            const element = panel.querySelector(selector);
            if (!element || element.disabled) throw new Error(`missing enabled control: ${selector}`);
            return color(getComputedStyle(element).color);
        };
        rows.push({
            mode, root: color(getComputedStyle(view.dom).color), group: chrome(group),
            panels: [chrome(find), chrome(go)], edge: bottom - goRect.bottom,
            findButton: control(find, 'button.cm-button[name=next]'),
            findField: control(find, 'input.cm-textfield[name=search]'),
            goButton: control(go, 'button.cm-button[type=submit]'),
            goField: control(go, 'input.cm-textfield[name=line]'),
        });
    }
} finally {
    view.destroy();
}
globalThis.nativeSupport = rows.map(row => JSON.stringify(row));
