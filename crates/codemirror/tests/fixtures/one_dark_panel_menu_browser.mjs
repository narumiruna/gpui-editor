// Capture with capture_browser_fixture.mjs and the theme-one-dark, search, lint, and autocomplete packages.
import {EditorSelection} from '@codemirror/state';
import {EditorView} from '@codemirror/view';
import {autocompletion, startCompletion} from '@codemirror/autocomplete';
import {linter, openLintPanel, setDiagnostics} from '@codemirror/lint';
import {openSearchPanel, search} from '@codemirror/search';
import {oneDarkTheme} from '@codemirror/theme-one-dark';

const output = [];
function color(value) {
    const match = /^rgba?\((\d+), (\d+), (\d+)(?:, ([\d.]+))?\)$/.exec(value);
    if (!match) throw new Error(`not an RGB color: ${value}`);
    if (Number(match[4] ?? 1) === 0) return null;
    if (Number(match[4] ?? 1) !== 1) throw new Error(`expected opaque role color: ${value}`);
    return match.slice(1, 4).map(part => Number(part).toString(16).padStart(2, '0')).join('');
}
function record(name, element) {
    if (!element) throw new Error(`missing ${name}`);
    const style = getComputedStyle(element);
    const top = Number.parseFloat(style.borderTopWidth);
    const bottom = Number.parseFloat(style.borderBottomWidth);
    if (Number.parseFloat(style.borderLeftWidth) || Number.parseFloat(style.borderRightWidth))
        throw new Error(`unexpected side border on ${name}`);
    const border = top || bottom ? color(top ? style.borderTopColor : style.borderBottomColor) : null;
    if ((top && style.borderTopStyle !== 'solid') || (bottom && style.borderBottomStyle !== 'solid'))
        throw new Error(`unexpected border style on ${name}`);
    output.push(JSON.stringify({name, fill: color(style.backgroundColor), text: color(style.color), border, top, bottom}));
}
const mount = (doc, extensions, selection) => new EditorView({doc, selection, parent: document.body, extensions: [oneDarkTheme, extensions]});

// These use the shared top and bottom panel wrappers. The native Diagnostics
// and References panels use the same roles, not the browser placement or controls.
const top = mount('abc', search({top: true}));
openSearchPanel(top);
record('stock-top-search', top.dom.querySelector('.cm-panels-top'));
top.destroy();

const bottom = mount('abc', linter(null));
bottom.dispatch(setDiagnostics(bottom.state, [{from: 0, to: 1, severity: 'error', message: 'fixture'}]));
openLintPanel(bottom);
record('stock-bottom-lint', bottom.dom.querySelector('.cm-panels-bottom'));
bottom.destroy();

const completion = mount('c', autocompletion({
    override: [() => ({from: 0, options: [{label: 'carry'}, {label: 'close'}]})],
    activateOnTyping: false,
}), EditorSelection.single(1));
completion.focus();
if (!startCompletion(completion)) throw new Error('startCompletion declined');
for (let step = 0; step < 40 && !completion.dom.querySelector('.cm-tooltip-autocomplete'); step++)
    await new Promise(resolve => setTimeout(resolve, 20));
const menu = completion.dom.querySelector('.cm-tooltip-autocomplete');
record('completion-menu', menu);
record('completion-selected', menu.querySelector('li[aria-selected="true"]'));
record('completion-unselected', menu.querySelector('li:not([aria-selected])'));
completion.destroy();
globalThis.nativeSupport = output;
