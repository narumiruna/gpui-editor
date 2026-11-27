// Run against the integrity-checked browser import map for the pinned packages.
import {Compartment, EditorSelection, EditorState} from '@codemirror/state';
import {
    Decoration, EditorView, WidgetType, highlightActiveLine, highlightTrailingWhitespace, highlightWhitespace,
} from '@codemirror/view';

class Label extends WidgetType {
    toDOM() {
        const node = document.createElement('div');
        node.textContent = 'replacement';
        return node;
    }
}
const source = '猫 alpha\nbeta γ\nend  \t';
const selection = EditorSelection.create([
    EditorSelection.cursor(1), EditorSelection.cursor(6), EditorSelection.range(7, 10),
    EditorSelection.cursor(13), EditorSelection.cursor(source.length),
], 2);
function state(view) {
    const doc = view.state.doc;
    const byte = pos => new TextEncoder().encode(doc.sliceString(0, pos)).length;
    return {doc: doc.toString(), byte};
}
function marks(view, classes) {
    const {byte} = state(view);
    const marks = [];
    for (const provider of view.state.facet(EditorView.decorations)) {
        const set = typeof provider === 'function' ? provider(view) : provider;
        for (let iter = set.iter(); iter.value; iter.next()) {
            const name = iter.value.spec.class;
            if (classes.includes(name)) marks.push([byte(iter.from), byte(iter.to), name]);
        }
    }
    return marks.sort((a, b) => a[0] - b[0] || a[1] - b[1] || a[2].localeCompare(b[2]));
}
const rows = [];
for (const [name, replacements] of [
    ['plain', Decoration.none],
    ['join_lines', Decoration.set(Decoration.replace({}).range(4, 12))],
    ['block_range', Decoration.set(Decoration.replace({block: true, widget: new Label()}).range(4, 12))],
]) {
    const view = new EditorView({doc: source, parent: document.body, extensions: [
        EditorState.allowMultipleSelections.of(true), EditorView.decorations.of(replacements), highlightActiveLine(),
    ]});
    view.measure();
    view.dispatch({selection});
    const {doc, byte} = state(view);
    const ranges = view.state.selection.ranges;
    const endpoints = ranges.map(range => [byte(range.anchor), byte(range.head)]);
    const blocks = ranges.map(range => {
        const block = view.lineBlockAt(range.head);
        return [byte(range.head), byte(block.from), byte(block.to)];
    });
    rows.push({kind: 'blocks', name, doc, endpoints, blocks, active: marks(view, ['cm-activeLine'])});
    view.destroy();
}

const slot = new Compartment();
const view = new EditorView({doc: source, parent: document.body, extensions: [slot.of([])]});
const plugins = [highlightActiveLine, highlightWhitespace, highlightTrailingWhitespace];
const classes = [['cm-activeLine'], ['cm-highlightSpace', 'cm-highlightTab'], ['cm-trailingSpace']];
for (const enabled of [[], [0, 1, 2], [1], [0, 2], []]) {
    view.dispatch({effects: slot.reconfigure(enabled.flatMap(index => {
        const extension = plugins[index]();
        return [extension, extension];
    }))});
    rows.push({kind: 'config', doc: view.state.doc.toString(), enabled,
        sets: classes.map(names => marks(view, names))});
}
view.destroy();
globalThis.nativeSupport = rows.map(row => JSON.stringify(row));
