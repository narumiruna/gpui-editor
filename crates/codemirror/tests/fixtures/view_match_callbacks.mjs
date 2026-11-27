// Run against the integrity-checked browser import map for the pinned packages.
import {EditorState} from '@codemirror/state';
import {Decoration, EditorView, MatchDecorator, ViewPlugin, WidgetType} from '@codemirror/view';

class Label extends WidgetType {
    constructor(text) { super(); this.text = text; }
    eq(other) { return this.text === other.text; }
    toDOM() {
        const span = document.createElement('span');
        span.textContent = this.text;
        return span;
    }
}
const doc = '猫 @cat\n@dog tail\nplain';
const pattern = /(@)(?<name>[a-z]+)/g;
const cases = [
    ['decorate', {
        regexp: pattern,
        boundary: /\s/,
        decorate(add, from, to, match) {
            add(from, from + match[1].length, Decoration.replace({widget: new Label('at')}));
            add(from + match[1].length, to, Decoration.mark({class: `name-${match.groups.name}`}));
            add(to, to, Decoration.widget({widget: new Label(`#${match.groups.name}`), side: 1}));
        },
    }],
    ['optional', {
        regexp: pattern,
        decoration(match, view) {
            return match.groups.name === 'cat' ? null : Decoration.replace({
                widget: new Label(view.state.phrase('Match') + ':' + match.groups.name),
            });
        },
    }],
];

function ranges(view, decorations) {
    const doc = view.state.doc;
    const byte = offset => new TextEncoder().encode(doc.sliceString(0, offset)).length;
    const out = [];
    for (let iter = decorations.iter(); iter.value; iter.next()) {
        const spec = iter.value.spec;
        const kind = spec.class ? 'mark' : (iter.from === iter.to ? 'point' : 'replace');
        out.push([byte(iter.from), byte(iter.to), kind, spec.class ?? spec.widget.text]);
    }
    return out;
}
const rows = [];
for (const [caseName, config] of cases) {
    const matcher = new MatchDecorator(config);
    const plugin = ViewPlugin.fromClass(class {
        constructor(view) { this.decorations = matcher.createDeco(view); }
        update(update) { this.decorations = matcher.updateDeco(update, this.decorations); }
    }, {decorations: plugin => plugin.decorations});
    const view = new EditorView({doc, parent: document.body,
        extensions: [plugin, EditorState.phrases.of({Match: 'Hit'})]});
    view.measure();
    const seed = ranges(view, view.plugin(plugin).decorations);
    const steps = [];
    for (const edit of [{from: 6, insert: 's'}, {from: 9, to: 12, insert: 'man'}, {from: 8, to: 12, insert: ''}]) {
        const byte = offset => new TextEncoder().encode(view.state.doc.sliceString(0, offset)).length;
        const editSpec = [byte(edit.from), byte(edit.to ?? edit.from), edit.insert];
        view.dispatch({changes: edit});
        steps.push({edit: editSpec, doc: view.state.doc.toString(), updated: ranges(view, view.plugin(plugin).decorations), rebuilt: ranges(view, matcher.createDeco(view))});
    }
    view.destroy();
    rows.push({case: caseName, doc, seed, steps});
}
globalThis.nativeSupport = rows.map(row => JSON.stringify(row));
