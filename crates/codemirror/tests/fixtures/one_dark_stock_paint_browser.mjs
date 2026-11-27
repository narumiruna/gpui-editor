// Capture with capture_browser_fixture.mjs and @codemirror/theme-one-dark,@codemirror/lang-markdown.
import {Compartment, EditorSelection} from '@codemirror/state';
import {EditorView, drawSelection, lineNumbers} from '@codemirror/view';
import {foldEffect, codeFolding, syntaxHighlighting} from '@codemirror/language';
import {markdown} from '@codemirror/lang-markdown';
import {oneDark, oneDarkTheme, oneDarkHighlightStyle} from '@codemirror/theme-one-dark';

const output = [];
const add = row => output.push(JSON.stringify(row));
const mount = config => new EditorView({parent: document.body, ...config});
function rgb(css) {
    const match = /^rgba?\((\d+), (\d+), (\d+)(?:, ([\d.]+))?\)$/.exec(css);
    if (!match) throw new Error(`Not an RGB color: ${css}`);
    return [Number(match[1]), Number(match[2]), Number(match[3]), Math.round(Number(match[4] ?? 1) * 255)]
        .map(part => part.toString(16).padStart(2, '0')).join('');
}

const draw = new Compartment();
const view = mount({doc: 'abc xyz', selection: EditorSelection.single(0, 3),
    extensions: [oneDarkTheme, draw.of(drawSelection())]});
for (const [name, change] of [
    ['selection-focused', () => view.focus()],
    ['selection-blurred', () => view.contentDOM.blur()],
]) {
    change();
    view.measure();
    const mark = view.dom.querySelector('.cm-selectionBackground');
    add({name, focused: view.dom.classList.contains('cm-focused'), selection: rgb(getComputedStyle(mark).backgroundColor)});
}
view.dispatch({effects: draw.reconfigure([])});
add({name: 'ordinary-caret', color: rgb(getComputedStyle(view.contentDOM).caretColor),
    selection: rgb(getComputedStyle(view.contentDOM.querySelector('.cm-line'), '::selection').backgroundColor)});
view.destroy();

const folded = mount({doc: 'abc\nxyz\nabc', extensions: [oneDarkTheme, lineNumbers(), codeFolding()]});
folded.dispatch({effects: foldEffect.of({from: 4, to: 7})});
folded.measure();
const fold = getComputedStyle(folded.dom.querySelector('.cm-foldPlaceholder'));
const gutter = getComputedStyle(folded.dom.querySelector('.cm-gutters'));
add({name: 'fold-and-gutter', fold: {color: rgb(fold.color), background: rgb(fold.backgroundColor), borderWidth: Number.parseFloat(fold.borderTopWidth)},
    gutter: {color: rgb(gutter.color), background: rgb(gutter.backgroundColor), borderWidth: Number.parseFloat(gutter.borderRightWidth)}});
folded.destroy();

const doc = '# Heading\n**Bold** *Italic* [Link](https://example.invalid)';
for (const [name, ext] of [['view-only', oneDarkTheme], ['highlight-only', syntaxHighlighting(oneDarkHighlightStyle)], ['combined', oneDark]]) {
    const view = mount({doc, extensions: [ext, markdown()]});
    view.measure();
    const tokens = {};
    for (const token of ['Heading', 'Bold', 'Italic', 'Link']) {
        const element = [...view.contentDOM.querySelectorAll('.cm-line, .cm-line span')].reverse().find(el => el.textContent.includes(token));
        if (!element) throw new Error(`Missing syntax token ${token}`);
        const {color, fontWeight, fontStyle, textDecorationLine} = getComputedStyle(element);
        tokens[token] = {color: rgb(color), weight: Number(fontWeight), italic: fontStyle === 'italic', underline: textDecorationLine === 'underline'};
    }
    add({name, tokens});
    view.destroy();
}
globalThis.nativeSupport = output;
