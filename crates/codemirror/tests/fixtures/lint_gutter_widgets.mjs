// Run with capture_browser_fixture.mjs, package @codemirror/lint and the verified archives.
import {lintGutter, setDiagnostics} from '@codemirror/lint';
import {EditorState} from '@codemirror/state';
import {Decoration, EditorView, WidgetType} from '@codemirror/view';

class Replacement extends WidgetType {
    toDOM() {
        const node = document.createElement('div');
        node.textContent = 'replaced lines';
        return node;
    }
}

const source = 'é\r\nfirst\r\nmiddle\nlast\nend';
const rows = [];
for (const replace of [false, true]) {
    const groups = [];
    const base = EditorState.create({doc: source});
    const line = number => base.doc.line(number).from;
    const rawOffsets = [0];
    const encoder = new TextEncoder();
    for (const part of source.match(/\r\n|[^]/gu)) {
        const previous = rawOffsets.at(-1);
        if (part.length === 2 && part !== '\r\n') rawOffsets.push(null);
        rawOffsets.push(previous + encoder.encode(part).length);
    }
    const options = lintGutter({tooltipFilter(diagnostics) {
        const summary = diagnostics.map(d => d.message);
        groups.push(summary);
        return diagnostics.filter(d => d.message !== 'inside-error');
    }});
    const widget = Decoration.replace({block: true, widget: new Replacement()}).range(line(2), line(5));
    const view = new EditorView({parent: document.body, state: EditorState.create({doc: source, extensions: [
        options, replace ? EditorView.decorations.of(Decoration.set(widget)) : [],
    ]})});
    view.dispatch(setDiagnostics(view.state, ['outside', 'start', 'inside-warning', 'inside-error', 'end'].map((message, index) => ({
        from: line(index + 1), to: line(index + 1), message,
        severity: ['info', 'error', 'warning', 'error', 'hint'][index],
    }))));
    view.measure();
    const markers = [...view.dom.querySelectorAll('.cm-lint-marker')].map(marker => marker.className);
    rows.push({source, replace, lineStarts: [1, 2, 3, 4, 5].map(i => rawOffsets[line(i)]),
        widget: [rawOffsets[line(2)], rawOffsets[line(5)]], markers, groups});
    view.destroy();
}
globalThis.nativeSupport = rows.map(row => JSON.stringify(row));
