// Run with capture_browser_fixture.mjs, package codemirror and the verified archives.
import {basicSetup} from 'codemirror';
import {bracketMatching} from '@codemirror/language';
import {Compartment, EditorSelection, EditorState} from '@codemirror/state';
import {Decoration, EditorView} from '@codemirror/view';

const cases = [
    {source: '🐈 (x]', at: [3, 6], options: []},
    {source: '(x)', at: [0], options: [{afterCursor: false}]},
    {source: '(x)', at: [1], options: [{afterCursor: false}]},
    {source: 'é «(x)»', at: [2], options: [{brackets: '«»'}]},
    {source: '(x)', at: [0], options: [{afterCursor: false}, {afterCursor: true}]},
    {source: '🐈 (x\r\n  🐈\ry)', at: [4], options: [{maxScanDistance: 3}]},
    {source: '🐈 (x\r\n  🐈\ry)', at: [4], options: [{maxScanDistance: 0}]},
    {source: '(é\r\n)', at: [1, 4], options: [{maxScanDistance: 3}]},
    {source: '(x) (y] (z', at: [1, 7, 9], options: [{render: true}]},
];
const encoder = new TextEncoder();
const rows = [];
for (const {source, at, options} of cases) {
    const positions = [0];
    let offset = 0;
    for (const part of source.match(/\r\n|[^]/gu)) {
        if (part.length === 2 && part !== '\r\n') positions.push(null);
        positions.push(offset += encoder.encode(part).length);
    }
    const selection = EditorSelection.create(at.map(pos => EditorSelection.cursor(pos)));
    const updated = [];
    const extensions = options.map(({render, ...config}) => bracketMatching(render ? {
        ...config,
        renderMatch(match, state) {
            const range = pair => pair ? [positions[pair.from], positions[pair.to]] : null;
            updated.push([range(match.start), range(match.end), match.matched, state.selection.ranges.length]);
            return [Decoration.mark({class: match.matched ? 'custom-match' : 'custom-miss'}).range(match.start.from, match.start.to)];
        },
    } : config));
    const slot = new Compartment();
    const parent = document.createElement('div');
    document.body.append(parent);
    let editor;
    try {
        editor = new EditorView({parent, state: EditorState.create({doc: source, selection,
            extensions: [basicSetup, slot.of(extensions)]})});
        const marks = () => {
            const result = [];
            for (const field of editor.state.facet(EditorView.decorations)) {
                const set = typeof field === 'function' ? field(editor) : field;
                for (let iter = set.iter(); iter.value; iter.next()) {
                    const label = iter.value.spec.class;
                    if (label?.endsWith('Bracket') || label?.startsWith('custom-')) {
                        result.push([positions[iter.from], positions[iter.to], label]);
                    }
                }
            }
            return result;
        };
        const before = marks();
        const callbacks = updated.slice();
        editor.dispatch({selection, effects: slot.reconfigure([])});
        rows.push({source, options, selection: at.map(pos => positions[pos]), before, callbacks, after: marks()});
    } catch (error) {
        if (!String(error).includes('afterCursor')) throw error;
        rows.push({source, options, selection: at.map(pos => positions[pos]), conflict: true});
    } finally {
        editor?.destroy();
        parent.remove();
    }
}
globalThis.nativeSupport = rows.map(row => JSON.stringify(row));
