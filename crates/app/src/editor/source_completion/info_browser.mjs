// Replay with crates/codemirror/tests/fixtures/capture_browser_fixture.mjs
// <archive-root> <chrome-executable> @codemirror/autocomplete,@codemirror/theme-one-dark <this file>
import {autocompletion, startCompletion} from '@codemirror/autocomplete';
import {Compartment} from '@codemirror/state';
import {oneDarkTheme} from '@codemirror/theme-one-dark';
import {EditorView} from '@codemirror/view';

const options = [
    {label: 'choiceA', info: '<a href="https://example.invalid">字</a>\n**literal**'},
    {label: 'choiceB'},
    {label: 'choiceC', info: 'Other choice'},
];
const theme = new Compartment();
const view = new EditorView({
    doc: 'choice', selection: {anchor: 6}, parent: document.body,
    extensions: [theme.of([]), autocompletion({
        activateOnTyping: false,
        interactionDelay: 0,
        override: [() => ({from: 0, options})],
    })],
});
const output = [];
const query = selector => view.dom.querySelector(selector);
const key = key => view.contentDOM.dispatchEvent(new KeyboardEvent('keydown', {key, bubbles: true}));
const hex = value => {
    const parts = /^rgb\((\d+), (\d+), (\d+)\)$/.exec(value);
    if (!parts) throw new Error(`Expected opaque color: ${value}`);
    return parts.slice(1).map(part => Number(part).toString(16).padStart(2, '0')).join('');
};
function capture(name, selected, text) {
    view.measure();
    const menu = query('.cm-tooltip-autocomplete');
    const row = menu?.querySelector('li[aria-selected="true"]');
    const info = query('.cm-completionInfo');
    if ((row?.querySelector('.cm-completionLabel').textContent ?? null) !== selected
        || (info?.textContent ?? null) !== text || Boolean(menu) !== Boolean(selected))
        throw new Error(`Unexpected completion state: ${name}`);
    let pane = null;
    if (info) {
        const style = getComputedStyle(info);
        pane = {
            text: info.textContent,
            literal: info.childNodes.length === 1 && info.firstChild.nodeType === Node.TEXT_NODE,
            arrow: Boolean(info.querySelector('.cm-tooltip-arrow')),
            describedBy: row.getAttribute('aria-describedby') === info.id,
            right: info.classList.contains('cm-completionInfo-right'),
            background: hex(style.backgroundColor),
            foreground: hex(style.color),
            border: ['Top', 'Right', 'Bottom', 'Left'].map(edge => parseFloat(style[`border${edge}Width`])),
        };
    }
    output.push(JSON.stringify({name, selected, pane}));
}
view.focus();
if (!startCompletion(view)) throw new Error('startCompletion declined');
for (let step = 0; step < 40 && !query('.cm-completionInfo'); step++)
    await new Promise(resolve => setTimeout(resolve, 20));
capture('initial', 'choiceA', options[0].info);
view.dispatch({effects: theme.reconfigure(oneDarkTheme)});
capture('themed', 'choiceA', options[0].info);
key('ArrowDown');
capture('no-info', 'choiceB', null);
key('ArrowDown');
capture('next-info', 'choiceC', options[2].info);
view.dispatch({effects: theme.reconfigure([])});
capture('theme-removed', 'choiceC', options[2].info);
key('Escape');
capture('closed', null, null);
view.destroy();
globalThis.nativeSupport = output;
