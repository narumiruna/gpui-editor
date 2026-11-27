// node crates/codemirror/tests/fixtures/projected_comments.mjs ARCHIVE_ROOT
import {mkdtempSync, realpathSync, rmSync} from 'node:fs';
import {tmpdir} from 'node:os';
import {join, resolve} from 'node:path';
import {projectedOffsets, projectInput} from './editor_line_offsets.mjs';
import {pinnedModules} from './pinned_modules.mjs';

const root = realpathSync(mkdtempSync(join(tmpdir(), 'projected-comments-')));
try {
    const {load} = pinnedModules(resolve(process.argv[2]), root);
    const {EditorState, EditorSelection} = await load('@codemirror/state');
    const commands = await load('@codemirror/commands');
    const cases = [
        ['line caret', 'before\r  😀§é\r\nafter', 'toggleComment', '//'],
        ['line range ends at next line', 'before\r  §字\r\n    β\r§after', 'toggleComment', '//'],
        ['line removal', 'before\r  // 😀§é\r\nafter', 'lineUncomment', '//'],
        ['block fallback', 'before\r  😀§é\r\nafter', 'toggleComment', null],
        ['block line removal', 'before\r  /* 😀§é */\r\nafter', 'toggleBlockCommentByLine', null],
        ['block range', 'before\r  😀§é\r    🦀§β\r\nafter', 'toggleBlockCommentByLine', null],
        ['two carets', 'first\r  §α§\r\nsecond\r    😀§é§\nend', 'toggleComment', '//'],
        ['read only', 'before\r  // 😀§é\r\nafter', 'lineUncomment', '//', true],
    ];
    for (const [name, marked, command, line, readOnly = false] of cases) {
        const source = marked.replaceAll('§', '');
        const {byteAt} = projectedOffsets(source);
        const offsets = [...marked.matchAll(/§/g)].map((match, i) =>
            source.slice(0, match.index - i).replace(/\r\n?/g, '\n').length);
        const ranges = offsets.length === 1
            ? [EditorSelection.cursor(offsets[0])]
            : offsets.filter((_, i) => i % 2 === 0).map((at, i) => EditorSelection.range(at, offsets[i * 2 + 1]));
        const calls = [];
        const state = EditorState.create({doc: source,
            selection: EditorSelection.create(ranges), extensions: [
                EditorState.allowMultipleSelections.of(true), EditorState.readOnly.of(readOnly),
                EditorState.languageData.of((_state, pos, side) => {
                    calls.push([byteAt(pos), side]);
                    return [{commentTokens: {line, block: {open: '/*', close: '*/'}}}];
                }),
            ]});
        let transaction;
        const handled = commands[command]({state, dispatch(tr) { transaction = tr; }});
        const input = ranges.map(range => [byteAt(range.anchor), byteAt(range.head)]);
        const output = handled ? projectInput(source, transaction) : null;
        const select = output !== null ? transaction.newSelection.ranges.map(range =>
            [range.anchor, range.head].map(projectedOffsets(output).byteAt)) : null;
        console.log(JSON.stringify({name, source, command, line, readOnly, input, calls, output, select}));
    }
} finally {
    rmSync(root, {recursive: true, force: true});
}
