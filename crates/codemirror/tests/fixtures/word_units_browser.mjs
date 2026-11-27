// Run with capture_browser_fixture.mjs and the SHA-512-verified archives.
import {cursorSubwordForward, selectSubwordForward} from '@codemirror/commands';
import {highlightSelectionMatches} from '@codemirror/search';
import {EditorState} from '@codemirror/state';
import {EditorView} from '@codemirror/view';

const rows = [];
const encoder = new TextEncoder();
const byteAt = (editor, pos) => encoder.encode(editor.state.sliceDoc(0, pos)).length;
for (const word of ['', '😀', '😁', '\u{1fa00}']) {
    const extra = EditorState.languageData.of(() => [{wordChars: word}]);
    for (const [source, anchor, head] of [
        ['s 😀s s😀\r\ns', 0, 1],
        ['s 😀s s😀\r\ns', 0, 0],
        ['😀 😀', 0, 2],
    ]) {
        const cursor = anchor === head;
        const editor = new EditorView({state: EditorState.create({doc: source, selection: {anchor, head},
            extensions: [EditorState.lineSeparator.of('\r\n'), extra,
                highlightSelectionMatches({wholeWords: true, highlightWordAroundCursor: cursor})]}),
            parent: document.body});
        const marks = [];
        for (const provide of editor.state.facet(EditorView.decorations)) {
            const set = typeof provide === 'function' ? provide(editor) : provide;
            for (let iter = set.iter(); iter.value; iter.next()) {
                const name = iter.value.spec.class;
                if (name?.startsWith('cm-selectionMatch')) {
                    marks.push([byteAt(editor, iter.from), byteAt(editor, iter.to), name]);
                }
            }
        }
        rows.push({source, word, selection: [byteAt(editor, anchor), byteAt(editor, head)], cursor, marks});
        editor.destroy();
    }
    for (const extend of [false, true]) {
        const source = 'HTTP😀Parser';
        const editor = new EditorView({state: EditorState.create({doc: source, extensions: [extra]}),
            parent: document.body});
        const handled = (extend ? selectSubwordForward : cursorSubwordForward)(editor);
        const {anchor, head} = editor.state.selection.main;
        rows.push({source, word, extend, handled, selection: [byteAt(editor, anchor), byteAt(editor, head)]});
        editor.destroy();
    }
}
globalThis.nativeSupport = rows.map(row => JSON.stringify(row));
