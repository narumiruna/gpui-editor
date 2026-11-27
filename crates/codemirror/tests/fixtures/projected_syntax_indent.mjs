// node crates/codemirror/tests/fixtures/projected_syntax_indent.mjs <archive-root> > crates/codemirror/tests/fixtures/projected_syntax_indent.jsonl
import assert from 'node:assert/strict';
import {mkdtempSync, realpathSync, rmSync} from 'node:fs';
import {tmpdir} from 'node:os';
import {join, resolve} from 'node:path';
import {projectedOffsets} from './editor_line_offsets.mjs';
import {pinnedModules} from './pinned_modules.mjs';

const root = realpathSync(mkdtempSync(join(tmpdir(), 'projected-indent-')));
try {
    const {load} = pinnedModules(resolve(process.argv[2]), root);
    const {EditorSelection, EditorState, Transaction} = await load('@codemirror/state');
    const commands = await load('@codemirror/commands');
    const {go} = await load('@codemirror/lang-go');
    const {json} = await load('@codemirror/lang-json');
    const {cpp} = await load('@codemirror/lang-cpp');
    const {java} = await load('@codemirror/lang-java');
    const {rust} = await load('@codemirror/lang-rust');
    const {indentOnInput} = await load('@codemirror/language');
    const languages = {go, json, cpp, java, rust};
    const normalize = text => text.replace(/\r\n?/g, '\n');
    const cases = [
        ['indent a new Go line', 'go', 'insertNewlineAndIndent', 'package p\r\nfunc f() {§\r  println("猫😀")\r\n}\r\n'],
        ['insert below the last selected logical line', 'go', 'insertBlankLine', 'package p\r\nfunc f() {\r  pr§intln("猫😀")\r  oth§er()\r\n}\r\n', true],
        ['split Go arguments with intervening spaces', 'go', 'insertNewlineAndIndent', 'func f() {\r  use(  §  )\r}\r\n'],
        ['do not split a bracket-like gap in a Go comment', 'go', 'insertNewlineAndIndent', 'func f() {\r  // ( § ) text\r}\r\n'],
        ['split a JSON bracket pair', 'json', 'insertNewlineAndIndent', '{\r  "猫": [§]\r  }\r\n'],
        ['reindent a backwards selection', 'json', 'indentSelection', '{\r§"猫": [\r\n   "😀"\r  ]\r§}\r\n', true],
        ['ordinary newline without markup', 'go', 'insertNewlineAndIndent', 'package p\r\nvar n = "猫😀"§\r\nvar other = 2\r'],
        ['reject inserting LF after a separate CR', 'go', 'insertNewlineAndIndent', 'package p\r§var x = 1\n'],
        ['reject removing indentation between separate breaks', 'json', 'indentSelection', '[\r  §  \n1]\r\n'],
        ['type a C++ closer on a projected line', 'cpp', 'typeCloser', 'void f() {\r\n  call("猫😀");\r    §\r\n'],
        ['type a Java closer on a projected line', 'java', 'typeCloser', 'class A {\r  void f() {\r\n    call("猫😀");\r      §\r\n}\r\n'],
        ['type a Rust closer on a projected line', 'rust', 'typeCloser', 'fn f() {\r\n  call("猫😀");\r    §\r\n'],
    ];
    for (const [name, language, command, marked, reverse] of cases) {
        let source = '';
        const positions = [];
        for (const ch of marked) {
            if (ch === '§') positions.push(source.length);
            else source += ch;
        }
        const unit = at => normalize(source.slice(0, at)).length;
        const unicodeSelection = [unit(positions[0]), unit(positions.at(-1))];
        if (reverse) unicodeSelection.reverse();
        const extensions = [languages[language]()];
        if (command === 'typeCloser') extensions.push(indentOnInput());
        const state = EditorState.create({doc: source, selection: EditorSelection.single(...unicodeSelection), extensions});
        let transaction;
        if (command === 'typeCloser') {
            transaction = state.update({...state.replaceSelection('}'), annotations: Transaction.userEvent.of('input.type')});
        } else {
            assert.ok(commands[command]({state, dispatch(update) { transaction = update; }}));
        }
        assert.ok(transaction, name);
        const {byteAt} = projectedOffsets(source);
        const raw = Buffer.from(source);
        const edits = [];
        const separator = after => {
            const start = byteAt(after), end = byteAt(after + 1);
            return raw.subarray(start, end).toString();
        };
        transaction.changes.iterChanges((from, to, _fromB, _toB, text) => {
            const line = state.doc.lineAt(from);
            const ending = line.to < state.doc.length ? separator(line.to)
                : line.from ? separator(line.from - 1) : '\r\n';
            edits.push([byteAt(from), byteAt(to), text.toString().replaceAll('\n', ending)]);
        });
        let updated = raw;
        for (const [from, to, insert] of edits.reverse()) {
            updated = Buffer.concat([updated.subarray(0, from), Buffer.from(insert), updated.subarray(to)]);
        }
        const output = updated.toString();
        const refused = normalize(output) !== transaction.newDoc.toString();
        const select = refused ? null : [transaction.newSelection.main.anchor, transaction.newSelection.main.head]
            .map(projectedOffsets(output).byteAt);
        console.log(JSON.stringify({name, language, command, source, selection: unicodeSelection.map(byteAt),
            event: transaction.annotation(Transaction.userEvent), scroll: transaction.scrollIntoView,
            output: refused ? null : output, select}));
    }
} finally {
    rmSync(root, {recursive: true, force: true});
}
