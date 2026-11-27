// Use capture_browser_fixture.mjs with the local @codemirror/language archive and its pinned dependencies.
import {HighlightStyle, StreamLanguage, highlightingFor, syntaxHighlighting, syntaxTree} from '@codemirror/language';
import {EditorView} from '@codemirror/view';
import {tags} from '@lezer/highlight';

const source = 'same';
const base = {color: '#234567', fontWeight: 'bold', fontStyle: 'italic', textDecoration: 'underline'};
const normal = {fontWeight: 'normal', fontStyle: 'normal', textDecoration: 'none'};
const positive = {tag: tags.keyword, ...base};
const reset = {tag: tags.comment, ...normal};
const decorated = HighlightStyle.define([positive]);
const cleared = HighlightStyle.define([{tag: tags.keyword, ...normal}]);
const allClear = HighlightStyle.define([], {all: normal});
const missing = StreamLanguage.define({token(stream) { stream.skipToEnd(); return null; }});
const cases = [
    ['positive', 'keyword', [decorated], 1],
    ['layers_reset_first', 'keyword', [cleared, decorated], 2],
    ['layers_reset_last', 'keyword', [decorated, cleared], 2],
    ['layers_partial', 'keyword', [HighlightStyle.define([{tag: tags.keyword, fontWeight: 'normal'}]), decorated], 2],
    ['rules_reset_last', 'keyword comment', [HighlightStyle.define([positive, reset])], 2],
    ['rules_reverse_tags', 'comment keyword', [HighlightStyle.define([positive, reset])], 2],
    ['rules_reset_first', 'keyword comment', [HighlightStyle.define([reset, positive])], 2],
    ['rules_per_property', 'keyword comment atom', [HighlightStyle.define([positive, reset,
        {tag: tags.atom, fontWeight: 'bold'},
    ])], 3],
    ['duplicate_selector', 'keyword', [HighlightStyle.define([positive, {tag: tags.keyword, ...normal}])], 1],
    ['duplicate_replaces_reset', 'keyword comment', [HighlightStyle.define([positive, reset,
        {tag: tags.comment, color: base.color},
    ])], 2],
    ['rule_resets_all', 'comment', [HighlightStyle.define([reset], {all: base})], 2],
    ['rule_over_all_resets', 'keyword', [HighlightStyle.define([positive], {all: normal})], 2],
    ['all_resets_other_layer', 'keyword', [allClear, decorated], 2],
    ['all_resets_untagged', '', [allClear, HighlightStyle.define([], {all: base})], 2],
    ['scope_miss', 'keyword', [HighlightStyle.define([], {all: normal, scope: missing}), decorated], 1],
    ['scope_match', 'keyword', [language => HighlightStyle.define([], {all: normal, scope: language}), decorated], 2],
    ['scope_miss_suppresses_fallback', 'keyword', [HighlightStyle.define([], {all: normal, scope: missing})], 0],
    ['theme_miss_allows_fallback', 'keyword', [HighlightStyle.define([], {all: normal, themeType: 'dark'})], 1],
];

const rows = [];
for (const [name, rawTags, styles, expectedClasses] of cases) {
    const names = rawTags ? rawTags.split(' ') : [];
    const language = StreamLanguage.define({token(stream) { stream.skipToEnd(); return rawTags || null; }});
    const layers = styles.map(style => syntaxHighlighting(typeof style === 'function' ? style(language) : style));
    if (name.endsWith('fallback')) layers.push(syntaxHighlighting(decorated, {fallback: true}));
    const view = new EditorView({doc: source, parent: document.body, extensions: [language.extension,
        EditorView.theme({'.cm-line': {color: '#102030'}}), ...layers,
    ]});
    try {
        const tree = syntaxTree(view.state);
        const classes = (highlightingFor(view.state, names.map(name => tags[name]), tree.topNode.type) ?? '').split(' ').filter(Boolean);
        const line = view.contentDOM.querySelector('.cm-line');
        const span = line.firstElementChild;
        if (line.textContent !== source || classes.length !== expectedClasses || (expectedClasses === 0
            ? span !== null
            : span?.tagName !== 'SPAN' || span.textContent !== source || span.children.length !== 0 ||
                line.childNodes.length !== 1 || span.classList.length !== classes.length ||
                !classes.every(name => span.classList.contains(name)))) {
            throw new Error(`${name}: expected one unnested span covering the token (${expectedClasses} classes): ${line.outerHTML}`);
        }
        const parent = getComputedStyle(line);
        if (parent.fontWeight !== '400' || parent.fontStyle !== 'normal' || parent.textDecorationLine !== 'none') {
            throw new Error(`${name}: line already sets font weight, font style or decoration`);
        }
        const css = getComputedStyle(span ?? line);
        rows.push({name, tags: names, classes: classes.length, color: css.color,
            weight: css.fontWeight, style: css.fontStyle, decoration: css.textDecorationLine});
    } finally {
        view.destroy();
    }
}
globalThis.nativeSupport = rows.map(row => JSON.stringify(row));
