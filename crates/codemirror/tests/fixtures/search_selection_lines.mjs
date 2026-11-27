// Run with capture_browser_fixture.mjs, package codemirror and the verified archives.
import {basicSetup} from 'codemirror';
import {highlightSelectionMatches, selectNextOccurrence, selectSelectionMatches} from '@codemirror/search';
import {EditorSelection, EditorState} from '@codemirror/state';
import {EditorView} from '@codemirror/view';

const encoder = new TextEncoder();
const rows = [];
for (const [source, separator] of [
    ['é\r\n🐈 | é\r🐈 | é\n🐈', null],
    ['é\r\n🐈 | é\r🐈 | é\n🐈', '\n'],
    ['é\r\n🐈 | é\r\n🐈', null],
]) {
    const offsets = [0];
    let end = 0;
    const parts = separator === null ? source.match(/\r\n|[^]/gu) : [...source];
    for (const part of parts) {
        if (part.length === 2 && part !== '\r\n') offsets.push(null);
        offsets.push(end += encoder.encode(part).length);
    }
    let start = 0;
    const matches = source.split(' | ').map(part => {
        const range = [start, start + encoder.encode(part).length];
        start = range[1] + 3;
        return range;
    });
    for (const [operation, chosen, min] of [
        ['highlight', [0], 4], ['highlight', [0], 5], ['highlight', [1], 4],
        ['all', [0], 0], ['all', [1], 0],
        ['next', [0], 0], ['next', [0, 1], 0], ['next', [1], 0],
    ]) {
        const selection = chosen.map(index => matches[index]);
        const initial = EditorSelection.create(selection.map(([from, to]) =>
            EditorSelection.range(offsets.indexOf(from), offsets.indexOf(to))), selection.length - 1);
        const editor = new EditorView({parent: document.body, state: EditorState.create({doc: source,
            selection: initial, extensions: [basicSetup,
                separator === null ? [] : EditorState.lineSeparator.of(separator),
                highlightSelectionMatches({minSelectionLength: min}),
            ]})});
        let result;
        if (operation === 'highlight') {
            result = [];
            for (const provide of editor.state.facet(EditorView.decorations)) {
                const set = typeof provide === 'function' ? provide(editor) : provide;
                for (let iter = set.iter(); iter.value; iter.next()) {
                    if (iter.value.spec.class === 'cm-selectionMatch') {
                        result.push([offsets[iter.from], offsets[iter.to]]);
                    }
                }
            }
        } else {
            const handled = (operation === 'all' ? selectSelectionMatches : selectNextOccurrence)(editor);
            const current = editor.state.selection;
            result = {handled, ranges: current.ranges.map(range => [offsets[range.anchor], offsets[range.head]]),
                main: current.mainIndex};
        }
        rows.push({source, separator, operation, selection, min, result});
        editor.destroy();
    }
}
globalThis.nativeSupport = rows.map(row => JSON.stringify(row));
