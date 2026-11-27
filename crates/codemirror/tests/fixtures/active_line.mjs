// node crates/codemirror/tests/fixtures/capture_browser_fixture.mjs <archive-root> <chrome> @codemirror/theme-one-dark crates/codemirror/tests/fixtures/active_line.mjs
import {Compartment, EditorSelection, EditorState} from '@codemirror/state';
import {Decoration, EditorView, highlightActiveLine, highlightActiveLineGutter, lineNumbers} from '@codemirror/view';
import {oneDarkTheme} from '@codemirror/theme-one-dark';

const body = '😀 first row has very long words that wrap with a narrow viewport {\n  hidden text\n} suffix after the replacement which also wraps substantially\nunrelated last line';
const {doc} = EditorState.create({doc: body});
const fold = [doc.line(1).to, doc.line(3).from];
const replacement = EditorView.decorations.of(Decoration.set([Decoration.replace({}).range(...fold)]));
const encoder = new TextEncoder();
const byte = pos => encoder.encode(doc.sliceString(0, pos)).length;
const rows = [{body, fold: fold.map(byte)}];
const theme = new Compartment(), content = new Compartment(), gutter = new Compartment();
const folds = new Compartment(), editable = new Compartment();
const parent = document.body.appendChild(document.createElement('div'));
parent.style.width = '200px';
const view = new EditorView({doc: body, parent, extensions: [
    EditorView.lineWrapping, EditorState.allowMultipleSelections.of(true),
    EditorState.readOnly.of(true), lineNumbers(),
    theme.of(oneDarkTheme), content.of([]), gutter.of([]), folds.of([]), editable.of([]),
]});

function color(el) {
    const [r, g, b, a = 1] = getComputedStyle(el).backgroundColor.match(/[\d.]+/g).map(Number);
    return ((r << 24) | (g << 16) | (b << 8) | Math.round(a * 255)) >>> 0;
}
function capture(name) {
    view.measure();
    const {ranges, mainIndex} = view.state.selection;
    rows.push({
        name, mainIndex, focused: view.hasFocus,
        selections: ranges.map(({anchor, head}) => [byte(anchor), byte(head)]),
        blocks: ranges.map(({head}) => {
            const block = view.lineBlockAt(head);
            return [byte(block.from), byte(block.to)];
        }),
        content: [...view.dom.querySelectorAll('.cm-line.cm-activeLine')].map(el => [byte(view.posAtDOM(el)), color(el)]),
        gutter: [...view.dom.querySelectorAll('.cm-lineNumbers .cm-activeLineGutter')].map(el => [byte(doc.line(Number(el.textContent)).from), color(el)]),
    });
}
function configure(...effects) { view.dispatch({effects}); }
function select(head) { view.dispatch({selection: {anchor: doc.length, head}}); }

view.dispatch({selection: EditorSelection.create([
    EditorSelection.cursor(5), EditorSelection.cursor(8), EditorSelection.range(doc.length, doc.line(3).from),
], 2)});
capture('theme-only');
view.focus();
capture('theme-only-focused');
view.contentDOM.blur();
configure(theme.reconfigure([]), content.reconfigure(highlightActiveLine()));
capture('line-only-light');
configure(content.reconfigure([]), gutter.reconfigure(highlightActiveLineGutter()));
capture('gutter-only-light');
configure(theme.reconfigure(oneDarkTheme), content.reconfigure(highlightActiveLine()), editable.reconfigure(EditorView.editable.of(false)));
capture('both-one-dark-readonly-uneditable');
configure(folds.reconfigure(replacement));
for (const [name, head] of [
    ['fold-start', doc.line(1).from + 3],
    ['fold-inside-hidden', doc.line(2).from + 2],
    ['fold-visible-suffix', doc.line(3).from + 2],
    ['unrelated', doc.line(4).from + 2],
]) {
    select(head);
    capture(name);
}
configure(editable.reconfigure([]), gutter.reconfigure([]));
view.focus();
capture('remove-gutter-opt-in');
configure(gutter.reconfigure(highlightActiveLineGutter()), content.reconfigure([]));
capture('remove-content-opt-in');
configure(gutter.reconfigure([]));
capture('remove-both-opt-ins');
configure(theme.reconfigure([]));
capture('no-theme-no-opt-ins');
view.destroy();
globalThis.nativeSupport = rows.map(row => JSON.stringify(row));
