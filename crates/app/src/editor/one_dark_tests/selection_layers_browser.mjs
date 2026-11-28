// Replay with capture_browser_fixture.mjs and @codemirror/theme-one-dark,@codemirror/lang-json.
import {json} from '@codemirror/lang-json';
import {bracketMatching} from '@codemirror/language';
import {EditorSelection, EditorState} from '@codemirror/state';
import {oneDarkTheme} from '@codemirror/theme-one-dark';
import {EditorView, drawSelection, highlightActiveLine, highlightActiveLineGutter, lineNumbers} from '@codemirror/view';

const source = '{ "value": [1, 2],\n  "next": 3 }';
const caret = source.indexOf('[') + 1;
const range = [source.indexOf('2'), source.indexOf(',\n') + 1];
const view = new EditorView({
    parent: document.body, doc: source,
    selection: EditorSelection.create([EditorSelection.cursor(caret), EditorSelection.range(...range)], 1),
    extensions: [EditorState.allowMultipleSelections.of(true), json(), oneDarkTheme, drawSelection(),
        highlightActiveLine(), lineNumbers(), highlightActiveLineGutter(), bracketMatching()],
});
view.focus();
view.measure();

const selectionLayer = view.scrollDOM.querySelector('.cm-selectionLayer');
const selections = [...selectionLayer.querySelectorAll('.cm-selectionBackground')];
const activeLines = [...view.contentDOM.querySelectorAll('.cm-activeLine')];
const activeGutters = [...view.scrollDOM.querySelectorAll('.cm-activeLineGutter')];
const brackets = [...view.contentDOM.querySelectorAll('.cm-matchingBracket')];
if (selections.length !== 1 || activeLines.length !== 1 || activeGutters.length !== 1
    || brackets.map(node => node.textContent).join('') !== '[]') throw new Error('Missing paint boxes');
const [selected] = selections;
const [line] = activeLines;
const [gutter] = activeGutters;
const [open, close] = brackets;
const selectionBox = selected.getBoundingClientRect();
const lineBox = line.getBoundingClientRect();
const openBox = open.getBoundingClientRect();
const closeBox = close.getBoundingClientRect();
const overlapsY = (a, b) => a.top < b.bottom && b.top < a.bottom;

function background(node) {
    const css = getComputedStyle(node).backgroundColor;
    const match = /^rgba?\((\d+), (\d+), (\d+)(?:, ([\d.]+))?\)$/.exec(css);
    if (!match) throw new Error(`Not an RGB color: ${css}`);
    return [...match.slice(1, 4).map(Number), Math.round(Number(match[4] ?? 1) * 255)]
        .map(part => part.toString(16).padStart(2, '0')).join('');
}

const record = {
    source, caret, range,
    focused: view.hasFocus && view.dom.classList.contains('cm-focused'),
    scrollerZ: getComputedStyle(view.scrollDOM).zIndex,
    selectionLayerZ: getComputedStyle(selectionLayer).zIndex,
    contentZ: getComputedStyle(view.contentDOM).zIndex,
    scrollerSiblings: selectionLayer.parentElement === view.scrollDOM
        && view.contentDOM.parentElement === view.scrollDOM,
    bracketsInActiveLine: line.contains(open) && line.contains(close),
    activeLineOverlapsSelection: lineBox.left <= selectionBox.left && lineBox.right >= selectionBox.right
        && overlapsY(lineBox, selectionBox),
    selectionCrossesCloseOnly: openBox.right < selectionBox.left && selectionBox.left < closeBox.left
        && closeBox.right < selectionBox.right && overlapsY(closeBox, selectionBox),
    selection: background(selected),
    activeLine: background(line),
    matchingBracket: background(close),
    activeGutter: background(gutter),
};
view.destroy();
globalThis.nativeSupport = [JSON.stringify(record)];
