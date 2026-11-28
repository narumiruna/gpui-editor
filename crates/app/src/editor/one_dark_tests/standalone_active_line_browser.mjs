// Chrome 158.0.8127.37. Replay with capture_browser_fixture.mjs and
// @codemirror/theme-one-dark,@codemirror/lang-javascript.
import {javascript} from '@codemirror/lang-javascript';
import {Compartment, EditorSelection, EditorState} from '@codemirror/state';
import {oneDark} from '@codemirror/theme-one-dark';
import {EditorView, drawSelection, highlightActiveLine} from '@codemirror/view';

const source = "const label = 'café'; // this logical line wraps across several measured display rows\n\nconst end = 42;\n";
const range = [source.indexOf('label'), source.indexOf('label') + 'label'.length];
const caret = source.indexOf('\n\n') + 1;
const byte = pos => new TextEncoder().encode(source.slice(0, pos)).length;

function background(node) {
    const css = getComputedStyle(node).backgroundColor;
    const match = /^rgba?\((\d+), (\d+), (\d+)(?:, ([\d.]+))?\)$/.exec(css);
    if (!match) throw new Error(`Not an RGB color: ${css}`);
    return [...match.slice(1, 4).map(Number), Math.round(Number(match[4] ?? 1) * 255)]
        .map(part => part.toString(16).padStart(2, '0')).join('');
}

const blurTarget = document.body.appendChild(document.createElement('button'));
const records = [];
for (const path of ['sample.txt', 'sample.js']) {
    const active = new Compartment();
    const selection = EditorSelection.create([EditorSelection.range(...range), EditorSelection.cursor(caret)]);
    const view = new EditorView({
        parent: document.body, doc: source, selection,
        extensions: [EditorState.allowMultipleSelections.of(true), oneDark, drawSelection(),
            EditorView.lineWrapping, active.of([]), path.endsWith('.js') ? javascript() : []],
    });
    view.dom.style.width = '228px';
    view.focus();
    for (const mode of ['theme-only', 'active-focused', 'active-blurred', 'removed']) {
        if (mode === 'active-focused') view.dispatch({effects: active.reconfigure(highlightActiveLine())});
        if (mode === 'active-blurred') blurTarget.focus();
        if (mode === 'removed') view.dispatch({effects: active.reconfigure([])});
        view.measure();
        if (view.state.doc.toString() !== source || !view.state.selection.eq(selection)) {
            throw new Error('The source or selection changed');
        }
        const content = view.contentDOM.getBoundingClientRect();
        const lines = [...view.contentDOM.querySelectorAll('.cm-line')];
        if (lines.length !== view.state.doc.lines) throw new Error('Missing logical lines');
        const marked = lines.flatMap((node, index) => {
            if (!node.classList.contains('cm-activeLine')) return [];
            const rect = node.getBoundingClientRect();
            const block = view.lineBlockAt(view.state.doc.line(index + 1).from);
            return [{range: [byte(block.from), byte(block.to)], color: background(node),
                fullWidth: rect.left === content.left && rect.right === content.right,
                measuredHeight: rect.height === block.height,
                wrapped: rect.height > lines[1].getBoundingClientRect().height,
                blank: !node.textContent}];
        });
        const selectionLayer = view.scrollDOM.querySelector('.cm-selectionLayer');
        const selections = [...selectionLayer.querySelectorAll('.cm-selectionBackground')];
        if (selections.length !== 1) throw new Error('Expected one selection rectangle');
        const [selected] = selections;
        const selectedBox = selected.getBoundingClientRect();
        const firstLine = lines[0].getBoundingClientRect();
        records.push(JSON.stringify({path, mode, source, range: range.map(byte), caret: byte(caret),
            focused: view.hasFocus && view.dom.classList.contains('cm-focused'), marked,
            selectionColor: background(selected), selectionBelowContent:
                selectionLayer.parentElement === view.scrollDOM && view.contentDOM.parentElement === view.scrollDOM
                && getComputedStyle(view.scrollDOM).zIndex === '0'
                && Number(getComputedStyle(selectionLayer).zIndex) < 0
                && getComputedStyle(view.contentDOM).zIndex === 'auto',
            selectionOverlapsFirstLine: firstLine.left < selectedBox.left && selectedBox.right < firstLine.right
                && firstLine.top < selectedBox.bottom && selectedBox.top < firstLine.bottom}));
    }
    view.destroy();
}
globalThis.nativeSupport = records;
