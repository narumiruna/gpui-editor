// Run with the integrity-checked upstream browser import map.
import {Compartment, EditorState, Prec, RangeSet} from '@codemirror/state';
import {
    Decoration, EditorView, GutterMarker, WidgetType, gutter, gutterLineClass, gutterWidgetClass,
    gutters, lineNumberWidgetMarker, lineNumbers,
} from '@codemirror/view';

const doc = '猫 one\nbeta\nlast';
const config = new Compartment();
const calls = [];
const bytes = offset => new TextEncoder().encode(doc.slice(0, offset)).length;

class Widget extends WidgetType {
    constructor(label) { super(); this.label = label; }
    eq(other) { return this.label === other.label; }
    toDOM() {
        const node = document.createElement('div');
        node.style.height = '15px';
        node.textContent = this.label;
        return node;
    }
}

class Marker extends GutterMarker {
    constructor(name, content) {
        super();
        this.elementClass = `widget-${name}`;
        if (content !== undefined) this.toDOM = () => document.createTextNode(content);
    }
}

const marker = (name, accepts, text = () => undefined) => (_view, widget, block) => {
    calls.push([name, widget.label, bytes(block.from), bytes(block.to), block.type]);
    return accepts(widget.label) ? new Marker(name, text(widget.label)) : null;
};

const decorations = Decoration.set([
    Decoration.widget({widget: new Widget('before'), block: true}).range(0),
    Decoration.widget({widget: new Widget('inline')}).range(2),
    Decoration.widget({widget: new Widget('after'), block: true, side: 1}).range(5),
    Decoration.replace({widget: new Widget('replace'), block: true}).range(6, 10),
    Decoration.widget({widget: new Widget('silent'), block: true, side: 1}).range(doc.length),
], true);
const view = new EditorView({
    state: EditorState.create({doc, extensions: [EditorView.decorations.of(decorations), config.of([])]}),
    parent: document.body,
});
view.measure();
view.dispatch({effects: config.reconfigure([
    gutter({class: 'after', side: 'after', widgetMarker: marker('local', name => name === 'before', name => name)}),
    lineNumbers(),
    gutter({class: 'empty', renderEmptyElements: true}),
    lineNumberWidgetMarker.of(marker('number-1', name => name === 'after', () => '↑')),
    lineNumberWidgetMarker.of(marker('number-2', name => name === 'replace', () => 'R')),
    gutterWidgetClass.of(marker('common', name => name === 'before' || name === 'replace')),
    Prec.high(gutterWidgetClass.of(marker('high', name => name === 'replace', () => '*'))),
])});
const slots = Array.from(view.dom.querySelectorAll('.cm-gutter'), column => ({
    class: column.className,
    rows: Array.from(column.querySelectorAll('.cm-gutterElement'), el => [el.className, el.textContent])
        .filter(([classes]) => classes.includes('widget-')),
}));
view.destroy();

const local = ['a', 'b', 'c', 'd'].map(name => RangeSet.of(new Marker(`local-${name}`).range(0)));
const shared = ['a', 'b', 'c', 'd', 'e'].map(name => RangeSet.of(new Marker(`shared-${name}`).range(0)));
const tied = new EditorView({
    doc,
    parent: document.body,
    extensions: [gutter({class: 'ties', markers: () => local}), shared.map(set => gutterLineClass.of(set))],
});
const ties = Array.from(tied.dom.querySelector('.cm-gutterElement').classList).slice(1);
tied.destroy();
const cover = [];
for (const direction of ['ltr', 'rtl']) {
    for (const columns of ['none', 'before', 'both']) {
        const parent = document.body.appendChild(document.createElement('div'));
        parent.dir = direction;
        const policy = new Compartment();
        const view = new EditorView({doc, parent, extensions: [policy.of(gutters()),
            columns === 'none' ? [] : lineNumbers(),
            columns === 'both' ? gutter({side: 'after'}) : [],
        ]});
        view.measure();
        const before = view.dom.querySelector('.cm-gutters-before');
        const after = view.dom.querySelector('.cm-gutters-after');
        const bounds = (left, right) => ({left, right, top: 0, bottom: 80});
        before.getBoundingClientRect = () => direction === 'ltr' ? bounds(10, 25) : bounds(280, 300);
        if (after) after.getBoundingClientRect = () => direction === 'ltr' ? bounds(280, 300) : bounds(10, 25);
        for (const options of [[], [false], [true, false], [true]]) {
            view.dispatch({effects: policy.reconfigure([gutters(), ...options.map(fixed => gutters({fixed}))])});
            const edges = view.state.facet(EditorView.coveredMargins).map(read => read(view)).find(Boolean);
            cover.push({direction, columns, options, fixed: before.style.position === 'sticky',
                edges: edges ? [edges.left ?? null, edges.right ?? null] : null});
        }
        view.destroy();
        parent.remove();
    }
}
globalThis.nativeSupport = [JSON.stringify({calls, slots, ties, cover})];
