// Capture with capture_browser_fixture.mjs and @codemirror/language.
import {Tag, tags} from '@lezer/highlight';
import {HighlightStyle, highlightingFor, syntaxHighlighting} from '@codemirror/language';
import {EditorView} from '@codemirror/view';

const root = Tag.define('root', tags.number);
const bases = {root, leaf: Tag.define('leaf', root), number: tags.number};
const modifiers = {
    a: Tag.defineModifier('same'),
    b: Tag.defineModifier('same'),
    c: Tag.defineModifier('c'),
    d: Tag.defineModifier('d'),
    function: tags.function,
    definition: tags.definition,
};
function tag([base, mods]) {
    return mods.reduce((value, name) => modifiers[name](value), bases[base]);
}
const cases = [
    ['exact custom', ['leaf', ['a']], [['leaf', ['a']], ['leaf', []]]],
    ['different same name', ['leaf', ['b']], [['leaf', ['a']], ['leaf', []]]],
    ['custom ancestor', ['leaf', ['b']], [['root', ['b']], ['number', ['b']]]],
    ['standard ancestor', ['leaf', ['b']], [['number', ['b']], ['number', []]]],
    ['unmodified child first', ['leaf', ['b']], [['leaf', []], ['root', ['b']]]],
    ['larger subset first', ['leaf', ['d', 'b', 'a']], [['leaf', ['b', 'd']], ['leaf', ['a']]]],
    ['first defined custom first', ['leaf', ['b', 'a']], [['leaf', ['a']], ['leaf', ['b']]]],
    ['first defined custom first reversed', ['leaf', ['a', 'b']], [['leaf', ['b']], ['leaf', ['a']]]],
    ['mixed subset first', ['leaf', ['a', 'function', 'definition']],
        [['leaf', ['function', 'definition']], ['leaf', ['a', 'definition']]]],
    ['highest differing id first', ['leaf', ['d', 'b', 'a', 'c']],
        [['leaf', ['c', 'b']], ['leaf', ['d', 'a']]]],
    ['mixed highest differing id first', ['leaf', ['b', 'a', 'function']],
        [['leaf', ['a', 'b']], ['leaf', ['a', 'function']]]],
    ['exact mixed', ['leaf', ['a', 'function', 'a']],
        [['leaf', ['function', 'a']], ['leaf', ['function']]]],
    ['duplicate selector last', ['leaf', ['function', 'a']],
        [['leaf', ['function', 'a']], ['leaf', ['a', 'function', 'a']]]],
    ['unrelated modifier ignored', ['root', ['function']], [['root', ['a']]]],
];
const colors = ['123456', '654321'];
const rows = [];
for (const [name, query, rules] of cases) {
    const style = HighlightStyle.define(rules.map((rule, i) => ({tag: tag(rule), color: `#${colors[i]}`})));
    const parent = document.body.appendChild(document.createElement('div'));
    const view = new EditorView({doc: 'x', parent, extensions: [syntaxHighlighting(style)]});
    try {
        const cls = highlightingFor(view.state, [tag(query)]);
        let rgb = null;
        if (cls) {
            const element = view.contentDOM.appendChild(document.createElement('span'));
            element.className = cls;
            element.textContent = 'x';
            const match = /^rgb\((\d+), (\d+), (\d+)\)$/.exec(getComputedStyle(element).color);
            if (!match) throw new Error(`missing RGB for ${name}`);
            rgb = match.slice(1).map(x => Number(x).toString(16).padStart(2, '0')).join('');
        }
        rows.push({name, query, rules, rgb});
    } finally {
        view.destroy();
        parent.remove();
    }
}
const a = modifiers.a;
const b = modifiers.b;
let modifiedParentRejected = false;
try {
    Tag.define('invalid', a(root));
} catch (error) {
    modifiedParentRejected = error instanceof Error && /modified tag/.test(error.message);
}
rows.push({idempotent: a(a(root)) === a(root), commutes: a(b(root)) === b(a(root)),
    distinct: a(root) !== b(root), modifiedParentRejected});
globalThis.nativeSupport = rows.map(row => JSON.stringify(row));
