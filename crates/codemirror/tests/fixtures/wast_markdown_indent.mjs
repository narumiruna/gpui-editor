// Run with Node and a directory of local SHA-512-pinned npm archives.
import assert from 'node:assert/strict';
import {mkdtempSync, realpathSync, rmSync} from 'node:fs';
import {tmpdir} from 'node:os';
import {join, resolve} from 'node:path';
import {projectedOffsets} from './editor_line_offsets.mjs';
import {pinnedModules} from './pinned_modules.mjs';

assert.equal(process.argv.length, 3, 'provide the directory of local npm archives');
const scratch = mkdtempSync(join(realpathSync(tmpdir()), 'wast-markdown-indent.'));
try {
    const {load} = pinnedModules(resolve(process.argv[2]), scratch);
    const {EditorState} = await load('@codemirror/state');
    const {IndentContext, getIndentation, indentNodeProp, indentService, indentUnit} =
        await load('@codemirror/language');
    const {markdown} = await load('@codemirror/lang-markdown');
    const {wastLanguage} = await load('@codemirror/lang-wast');
    for (const [name, prefix, pad] of [['ordinary', '', '  '], ['quoted', '> ', '']]) {
        const source = '前 😀\r\n\r\n' + [pad + '```wat', pad + '(module "😀"', pad + '  (func 12)',
            pad + '    ()', pad + '  (; first', pad + '   second ;)', pad + ')', pad + '```']
            .map(line => prefix + line).join('\r\n').replace('"😀"\r\n', '"😀"\r') + '\n\nTail';
        const {byteAt} = projectedOffsets(source);
        const calls = [];
        const span = node => node ? [node.name, byteAt(node.from), byteAt(node.to)] : null;
        const called = (cx, next) => {
            const line = cx.lineAt(cx.pos);
            calls.push([span(cx.node), span(cx.node.parent), byteAt(cx.pos),
                [byteAt(line.from), byteAt(line.from + line.text.length), line.text], cx.baseIndent,
                next, byteAt(cx.state.selection.main.head)]);
        };
        const configured = wastLanguage.configure({props: [indentNodeProp.add({
            App(cx) {
                const next = cx.continue();
                called(cx, next);
                return (next ?? 0) + cx.baseIndent + cx.unit;
            },
            BlockComment(cx) {
                called(cx, null);
                return null;
            },
        })]});
        const create = (language, extra = []) => {
            const initial = EditorState.create({doc: source,
                extensions: [markdown({codeLanguages: info => info === 'wat' ? language : null}),
                    indentUnit.of('   '), ...extra]});
            return initial.update({selection: {anchor: initial.doc.toString().indexOf('Tail')}}).state;
        };
        const state = create(configured);
        const stock = create(wastLanguage);
        const text = state.doc.toString();
        const outer = text.indexOf('(module');
        const inner = text.indexOf('(func');
        const empty = text.indexOf('()') + 1;
        const comment = text.indexOf('second');
        const close = text.lastIndexOf(')');
        const pick = text.indexOf('12');
        const overrideLine = state.doc.lineAt(outer).from;
        const positions = [
            ['outer opener', outer, false, false],
            ['inside outer', outer + 1, false, false],
            ['before inner opener', inner, false, false],
            ['inside inner', pick, false, false],
            ['simulated double break', empty, true, false],
            ['host line override', pick, false, true],
            ['inside comment', comment, false, false],
            ['outer closer', close, false, false],
            ['body end', close + 1, false, false],
        ];
        const queries = positions.map(([label, at, doubleBreak, override]) => {
            const options = {};
            if (doubleBreak) Object.assign(options, {simulateBreak: at, simulateDoubleBreak: true});
            if (override) options.overrideIndentation = from => from === overrideLine ? 9 : -1;
            const result = getIndentation(new IndentContext(state, options), at);
            const stockResult = getIndentation(new IndentContext(stock, options), at);
            return {label, at: byteAt(at), doubleBreak, overrideAt: override ? byteAt(overrideLine) : null,
                result, stock: stockResult, calls: calls.splice(0)};
        });
        const withService = create(configured, [indentService.of(() => 19)]);
        assert.equal(getIndentation(withService, pick), 19);
        assert.deepEqual(calls, []);
        console.log(JSON.stringify({name, source, selection: byteAt(state.selection.main.head), queries}));
    }
} finally {
    rmSync(scratch, {recursive: true, force: true});
}
