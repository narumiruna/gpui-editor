// Run against the integrity-checked browser import map for the pinned packages.
import {EditorState} from '@codemirror/state';
import {EditorView, highlightSpecialChars} from '@codemirror/view';

const doc = 'N\0字XYx\t\u200bZ';
const bytes = pos => new TextEncoder().encode(doc.slice(0, pos)).length;
const render = (code, desc, stock) => {
    if (code !== '字'.codePointAt(0)) return null;
    const node = document.createElement('span');
    node.className = 'cm-specialChar';
    node.textContent = `「${stock}」`;
    node.title = desc;
    return node;
};
const cases = [
    ['inactive', []],
    ['add_to_stock', [highlightSpecialChars({addSpecialChars: /Y/g})]],
    ['replace_and_add', [highlightSpecialChars({specialChars: /字X|x/gu, render}),
        highlightSpecialChars({addSpecialChars: /Y/g})]],
    ['replace_with_flags', [highlightSpecialChars({specialChars: /[y]/gi})]],
    ['combined_rebuilds_flags', [highlightSpecialChars({specialChars: /[y]/gi, addSpecialChars: /x/g})]],
];
const rows = [];
for (const [name, extensions] of cases) {
    const view = new EditorView({doc, parent: document.body, extensions: [
        ...extensions,
        EditorState.phrases.of({'Control character': 'Visible control'}),
    ]});
    view.measure();
    const marks = [];
    for (const provider of view.state.facet(EditorView.decorations)) {
        const set = typeof provider === 'function' ? provider(view) : provider;
        for (let cursor = set.iter(); cursor.value; cursor.next()) {
            const code = cursor.value.spec.widget?.code;
            if (typeof code === 'number') marks.push([bytes(cursor.from), bytes(cursor.to), code]);
        }
    }
    const appearance = [...view.dom.querySelectorAll('.cm-specialChar')].map(node =>
        [node.textContent, node.title]);
    rows.push({name, doc, marks, appearance});
    view.destroy();
}
globalThis.nativeSupport = rows.map(row => JSON.stringify(row));
