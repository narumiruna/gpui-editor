// Capture with capture_browser_fixture.mjs and @codemirror/theme-one-dark.
import {Compartment} from '@codemirror/state';
import {oneDarkTheme} from '@codemirror/theme-one-dark';
import {EditorView} from '@codemirror/view';

const source = '{"title": "draft"}';
const theme = new Compartment();
const view = new EditorView({doc: source, parent: document.body, extensions: [theme.of([])]});
const rootTheme = props => EditorView.theme({'&': props});
const host = rootTheme({backgroundColor: '#102030'});
const background = node => {
    const css = getComputedStyle(node).backgroundColor;
    const [, r, g, b, alpha = 1] = /^rgba?\((\d+), (\d+), (\d+)(?:, ([\d.]+))?\)$/.exec(css);
    return [r, g, b, Math.round(alpha * 255)].map(value => Number(value).toString(16).padStart(2, '0')).join('');
};
const rows = [];
try {
    for (const [name, layers] of [
        ['bare', []],
        ['oneDark', [oneDarkTheme]],
        ['hostFirst', [host, oneDarkTheme]],
        ['themeFirst', [oneDarkTheme, host]],
        ['hostOnly', [host]],
        ['translucentHost', [rootTheme({backgroundColor: '#10203080'})]],
        ['foregroundOnly', [rootTheme({color: '#304050'})]],
        ['removed', []],
    ]) {
        view.dispatch({effects: theme.reconfigure(layers)});
        const line = view.contentDOM.querySelector('.cm-line');
        if (!line || line.textContent !== source || !view.contentDOM.isContentEditable
            || line.parentElement !== view.contentDOM || view.contentDOM.parentElement !== view.scrollDOM
            || view.scrollDOM.parentElement !== view.dom) throw new Error('Expected an editable source line');
        rows.push({name, source, root: background(view.dom), scroller: background(view.scrollDOM),
            content: background(view.contentDOM), line: background(line)});
    }
} finally {
    view.destroy();
}
globalThis.nativeSupport = rows.map(row => JSON.stringify(row));
