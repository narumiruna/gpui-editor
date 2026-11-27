// Run with Node 22.14+ and the pinned npm archives:
// node crates/codemirror/tests/fixtures/javascript_recovery_edits.mjs <archive-root>
// Add --emit to print the fixture.
import assert from 'node:assert/strict';
import {mkdtempSync, readFileSync, realpathSync, rmSync} from 'node:fs';
import {tmpdir} from 'node:os';
import {join, resolve} from 'node:path';
import {projectedOffsets, projectInput} from './editor_line_offsets.mjs';
import {pinnedModules} from './pinned_modules.mjs';

const dir = realpathSync(mkdtempSync(join(tmpdir(), 'javascript-recovery-')));
try {
    const {load} = pinnedModules(resolve(process.argv[2]), dir);
    const js = await load('@codemirror/lang-javascript');
    const lezer = await load('@lezer/javascript');
    const {EditorState} = await load('@codemirror/state');
    const {CompletionContext} = await load('@codemirror/autocomplete');
    const {ensureSyntaxTree, foldable, getIndentation, indentUnit} = await load('@codemirror/language');
    assert.deepEqual(Object.keys(js), [
        'autoCloseTags', 'completionPath', 'esLint', 'javascript', 'javascriptLanguage',
        'jsxLanguage', 'localCompletionSource', 'scopeCompletionSource', 'snippets',
        'tsxLanguage', 'typescriptLanguage', 'typescriptSnippets',
    ]);
    assert.deepEqual(Object.keys(lezer), ['parser']);

    const completeScope = js.scopeCompletionSource({
        __proto__: null,
        '𐐀': {__proto__: null, theme: {__proto__: null, field: 1, color() {}}},
    });
    function records(tree, byteAt) {
        const nodes = [];
        let depth = 0;
        tree.iterate({
            enter: node => nodes.push([depth++, node.name, byteAt(node.from), byteAt(node.to)]),
            leave: () => { depth--; },
        });
        return nodes;
    }
    function index(text, needle) {
        const at = text.indexOf(needle);
        assert(at >= 0 && at === text.lastIndexOf(needle), needle);
        return at;
    }
    const prefix = 'const outside = "😀";\nconst pre = outside;\nconst tail = 0;\n';
    const rows = [];
    for (const mode of ['js', 'ts', 'jsx', 'tsx']) {
        const typescript = mode.startsWith('ts'), jsx = mode.endsWith('x');
        const rawParser = lezer.parser.configure({dialect: [jsx && 'jsx', typescript && 'ts'].filter(Boolean).join(' ')});
        const language = js.javascript({typescript, jsx});
        const extensions = [language, indentUnit.of('  '), EditorState.tabSize.of(4)];
        const param = typescript ? '<T>(𐐀: Bag<T>)' : '(𐐀)';
        const decl = typescript ? 'step: T' : 'step';
        const expr = '𐐀.theme.ready';
        const member = '    𐐀.theme.';
        const source = (prefix + (typescript ? 'type Bag<T> = {value: T};\n' : '') +
            `function render${param} {\n  const ${decl} = ${expr};\n` +
            `  return ${jsx ? '<Panel>😀{' : ''}nested${typescript ? '<T>' : ''}(\n` +
            `    step,\n${member}field\n  )${jsx ? '}</Panel>' : ''};\n}\n`).replaceAll('\n', '\r\n');
        let raw = source;
        let state = EditorState.create({doc: source, extensions});
        const states = [];
        function snapshot(current) {
            const {byteAt} = projectedOffsets(raw);
            const tree = ensureSyntaxTree(current, current.doc.length, 1000);
            assert(tree);
            const nodes = records(tree, byteAt);
            assert.deepEqual(records(rawParser.parse(raw), pos => Buffer.byteLength(raw.slice(0, pos))), nodes);
            const text = current.doc.toString();
            const localPos = index(text, '    step,') + '    step'.length;
            const memberStart = index(text, member) + member.length;
            const memberPos = current.doc.lineAt(memberStart).to;
            const localContext = new CompletionContext(current, localPos, false);
            const memberContext = new CompletionContext(current, memberPos, false);
            const local = js.localCompletionSource(localContext);
            const scope = completeScope(memberContext);
            const lines = Array.from({length: current.doc.lines}, (_, i) => {
                const {from, to} = current.doc.line(i + 1);
                const fold = foldable(current, from, to);
                return [getIndentation(current, from) ?? null, fold ? [byteAt(fold.from), byteAt(fold.to)] : null];
            });
            return {nodes, lines, localPos: byteAt(localPos), memberPos: byteAt(memberPos),
                local: local ? [byteAt(local.from), local.options.map(option => [option.label, option.type])] : null,
                path: js.completionPath(memberContext),
                scope: scope ? [byteAt(scope.from), scope.options.map(option => [option.label, option.type, option.boost])] : null};
        }
        function save(id, edit) {
            const record = snapshot(state);
            assert.deepEqual(snapshot(EditorState.create({doc: raw, extensions})), record, `${mode}: ${id}`);
            states.push({id, edit, ...record});
        }
        function edit(id, before, remove, after, insert) {
            const {byteAt} = projectedOffsets(raw);
            const from = index(state.doc.toString(), before + remove + after) + before.length;
            const to = from + remove.length;
            const transaction = state.update({changes: {from, to, insert}});
            raw = projectInput(raw, transaction);
            state = transaction.state;
            save(id, [byteAt(from), byteAt(to), insert]);
        }
        save('seed', null);
        edit('delete initializer', `const ${decl} = `, expr, ';', '');
        edit('delete member name', member, 'field', '\n', '');
        edit('restore initializer', `const ${decl} = `, '', ';', expr);
        edit('restore member name', member, '', '\n', 'field');
        const [before, closer, after] = jsx ? ['  )}', '</Panel>', ';']
            : typescript ? ['𐐀: Bag<T', '>', ')'] : ['\n  ', ')', ';'];
        edit('delete closer', before, closer, after, '');
        edit('restore closer', before, '', after, closer);
        edit('valid member edit', member, 'field', '\n', 'éclair');
        rows.push({mode, source, states});
    }
    const output = rows.map(row => JSON.stringify(row)).join('\n') + '\n';
    if (process.argv.includes('--emit')) process.stdout.write(output);
    else {
        assert.equal(output, readFileSync(new URL('javascript_recovery_edits.jsonl', import.meta.url), 'utf8'));
        console.log(`${rows.length} pinned JavaScript dialect edit sequences matched`);
    }
} finally {
    rmSync(dir, {recursive: true, force: true});
}
