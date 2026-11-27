// Capture with capture_browser_fixture.mjs and @codemirror/theme-one-dark.
import {Compartment, Prec} from '@codemirror/state';
import {EditorView} from '@codemirror/view';
import {oneDarkTheme} from '@codemirror/theme-one-dark';

const root = new Compartment();
const view = new EditorView({doc: '一 🦀', selection: {anchor: 0, head: 1}, parent: document.body,
    extensions: [root.of([])]});
const surface = (props) => EditorView.theme({'&': props});
const host = surface({color: '#304050', backgroundColor: '#102030'});
const foreground = surface({color: '#304050'});
const background = surface({backgroundColor: '#102030'});
const rows = [];
const hex = css => {
    const [, r, g, b, alpha = 1] = /^rgba?\((\d+), (\d+), (\d+)(?:, ([\d.]+))?\)$/.exec(css);
    return [r, g, b, Math.round(alpha * 255)].map(value => Number(value).toString(16).padStart(2, '0')).join('');
};
try {
    for (const [name, layers] of [
        ['bare', []],
        ['hostOnly', [host]],
        ['viewHostFirst', [host, oneDarkTheme]],
        ['viewHostSecond', [oneDarkTheme, host]],
        ['onlyForegroundWins', [foreground, oneDarkTheme, background]],
        ['bothPropertiesWin', [background, foreground, oneDarkTheme]],
        ['higherPrecedence', [oneDarkTheme, Prec.high(host)]],
        ['lowerPrecedence', [Prec.low(host), oneDarkTheme]],
        ['foregroundOnly', [foreground]],
        ['removed', []],
    ]) {
        view.dispatch({effects: root.reconfigure(layers)});
        const css = getComputedStyle(view.dom);
        const {anchor, head} = view.state.selection.main;
        if (view.state.doc.toString() !== '一 🦀' || anchor !== 0 || head !== 1) {
            throw new Error('theme changed editing state');
        }
        rows.push({name, dark: view.state.facet(EditorView.darkTheme), foreground: hex(css.color),
            background: hex(css.backgroundColor)});
    }
} finally {
    view.destroy();
}
globalThis.nativeSupport = rows.map(row => JSON.stringify(row));
