// Capture with capture_browser_fixture.mjs and @codemirror/theme-one-dark.
import {Tag, tags} from '@lezer/highlight';
import {HighlightStyle, highlightingFor, syntaxHighlighting} from '@codemirror/language';
import {EditorView} from '@codemirror/view';
import {oneDarkHighlightStyle} from '@codemirror/theme-one-dark';

const isolated = Tag.define('number');
const numeric = Tag.define('same', tags.number);
const keyword = Tag.define('same', tags.keyword);
const child = Tag.define('child', numeric);
const call = Tag.define('call', tags.variableName);
const nestedCall = Tag.define('call', call);
const direct = HighlightStyle.define([
    {tag: numeric, color: '#123456'},
    {tag: keyword, color: '#654321'},
]);
const modifier = HighlightStyle.define([
    {tag: tags.function(numeric), color: '#112233'},
    {tag: numeric, color: '#445566'},
]);
const specificity = HighlightStyle.define([
    {tag: child, color: '#334455'},
    {tag: tags.function(numeric), color: '#667788'},
]);
const cases = [
    ['isolated name', isolated, oneDarkHighlightStyle],
    ['standard parent', numeric, oneDarkHighlightStyle],
    ['standard ancestor', child, oneDarkHighlightStyle],
    ['same name other parent', keyword, oneDarkHighlightStyle],
    ['modified standard ancestor', tags.function(nestedCall), oneDarkHighlightStyle],
    ['multiple modifiers', tags.function(tags.definition(nestedCall)), oneDarkHighlightStyle],
    ['concrete identity', numeric, direct],
    ['distinct same name identity', keyword, direct],
    ['custom parent identity', child, direct],
    ['independent same name', Tag.define('same'), direct],
    ['modified custom parent', tags.function(child), modifier],
    ['unmodified custom parent', child, modifier],
    ['child unmodified first', tags.function(child), specificity],
];
const rows = [];
for (const [name, tag, style] of cases) {
    const parent = document.body.appendChild(document.createElement('div'));
    const view = new EditorView({doc: 'x', parent, extensions: [syntaxHighlighting(style)]});
    try {
        const cls = highlightingFor(view.state, [tag]);
        let rgb = null;
        if (cls) {
            const element = view.contentDOM.appendChild(document.createElement('span'));
            element.className = cls;
            element.textContent = 'x';
            const match = /^rgb\((\d+), (\d+), (\d+)\)$/.exec(getComputedStyle(element).color);
            if (!match) throw new Error(`missing RGB for ${name}`);
            rgb = match.slice(1).map(x => Number(x).toString(16).padStart(2, '0')).join('');
        }
        rows.push({name, rgb});
    } finally {
        view.destroy();
        parent.remove();
    }
}
let rejected = false;
try {
    Tag.define('not allowed', tags.function(numeric));
} catch (error) {
    rejected = error instanceof Error && /modified tag/.test(error.message);
}
if (!rejected) throw new Error('expected modified parent rejection');
rows.push({name: 'modified parent rejected', rejected});
globalThis.nativeSupport = rows.map(row => JSON.stringify(row));
