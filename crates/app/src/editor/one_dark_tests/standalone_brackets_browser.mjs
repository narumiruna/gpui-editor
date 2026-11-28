// Chrome 158.0.8127.37. Replay with capture_browser_fixture.mjs and
// @codemirror/language,@codemirror/theme-one-dark,@codemirror/lang-javascript.
import {javascript} from '@codemirror/lang-javascript';
import {bracketMatching} from '@codemirror/language';
import {Compartment, EditorSelection, EditorState} from '@codemirror/state';
import {oneDarkTheme} from '@codemirror/theme-one-dark';
import {Decoration, EditorView, drawSelection} from '@codemirror/view';

const source = 'const result = run("🐱 )", 42);\n// «café\n// still»\nconst extra = (\n';
const selected = [source.indexOf('"'), source.indexOf(';') + 1];
const carets = [source.indexOf('('), source.indexOf('«') + 1, source.lastIndexOf('(') + 1];
const byte = pos => new TextEncoder().encode(source.slice(0, pos)).length;
const selection = EditorSelection.create([
    EditorSelection.cursor(carets[0]), EditorSelection.range(...selected),
    ...carets.slice(1).map(at => EditorSelection.cursor(at)),
], 1);
const customPairs = {brackets: '«»', afterCursor: false};
const closeOnly = Decoration.mark({class: 'cm-nonmatchingBracket'});
const renderer = {renderMatch: match => match.end ? [closeOnly.range(match.end.from, match.end.to)] : []};

function background(node) {
    const css = getComputedStyle(node).backgroundColor;
    const match = /^rgba?\((\d+), (\d+), (\d+)(?:, ([\d.]+))?\)$/.exec(css);
    if (!match) throw new Error(`Not an RGB color: ${css}`);
    return [...match.slice(1, 4).map(Number), Math.round(Number(match[4] ?? 1) * 255)]
        .map(part => part.toString(16).padStart(2, '0')).join('');
}

const blurTarget = document.body.appendChild(document.createElement('button'));
const records = [];
for (const path of ['sample.js', 'sample.txt']) {
    const brackets = new Compartment();
    const view = new EditorView({parent: document.body, doc: source, selection,
        extensions: [EditorState.allowMultipleSelections.of(true), oneDarkTheme, drawSelection(),
            brackets.of([]), path.endsWith('.js') ? javascript() : []]});
    view.dom.style.width = '640px';
    view.focus();
    // The pinned plugin recomputes on an explicit selection update, not on an option-only change.
    const reconfigure = extension => view.dispatch({selection, effects: brackets.reconfigure(extension)});
    for (const mode of ['theme-only', 'default', 'blurred', 'custom-pairs', 'limited', 'rendered', 'removed']) {
        if (mode === 'default') reconfigure(bracketMatching());
        if (mode === 'blurred') blurTarget.focus();
        if (mode === 'custom-pairs') {
            view.focus();
            reconfigure(bracketMatching(customPairs));
        }
        if (mode === 'limited') reconfigure(bracketMatching({...customPairs, maxScanDistance: 1}));
        if (mode === 'rendered') reconfigure(bracketMatching(renderer));
        if (mode === 'removed') reconfigure([]);
        view.measure();
        if (view.state.doc.toString() !== source || !view.state.selection.eq(selection)) {
            throw new Error('The source or selection changed');
        }
        const layer = view.scrollDOM.querySelector('.cm-selectionLayer');
        const selections = [...layer.querySelectorAll('.cm-selectionBackground')];
        if (selections.length !== 1) throw new Error('Expected one selection rectangle');
        const [selectedNode] = selections;
        const selectionBox = selectedNode.getBoundingClientRect();
        const marks = [...view.contentDOM.querySelectorAll('.cm-matchingBracket, .cm-nonmatchingBracket')]
            .flatMap(node => {
                const color = background(node);
                if (color === '00000000') return [];
                const start = view.posAtDOM(node, 0), end = start + node.textContent.length;
                const bounds = node.getBoundingClientRect();
                const glyphs = document.createRange();
                glyphs.selectNodeContents(node);
                const glyphBounds = glyphs.getBoundingClientRect();
                return [{range: [byte(start), byte(end)], text: node.textContent,
                    matched: node.classList.contains('cm-matchingBracket'), color,
                    glyphWidth: bounds.width > 0 && bounds.left === glyphBounds.left && bounds.right === glyphBounds.right,
                    overlapsSelection: bounds.left < selectionBox.right && selectionBox.left < bounds.right
                        && bounds.top < selectionBox.bottom && selectionBox.top < bounds.bottom}];
            });
        records.push(JSON.stringify({path, mode, source, ranges: selection.ranges.map(r => [byte(r.anchor), byte(r.head)]),
            focused: view.hasFocus && view.dom.classList.contains('cm-focused'), marks, selectionColor: background(selectedNode),
            selectionBelowContent: layer.parentElement === view.scrollDOM && view.contentDOM.parentElement === view.scrollDOM
                && getComputedStyle(view.scrollDOM).zIndex === '0' && Number(getComputedStyle(layer).zIndex) < 0
                && getComputedStyle(view.contentDOM).zIndex === 'auto'}));
    }
    view.destroy();
}
globalThis.nativeSupport = records;
