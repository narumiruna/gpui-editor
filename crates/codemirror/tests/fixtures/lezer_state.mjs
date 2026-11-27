// Run with Node and the directory of SHA-512-pinned npm archives.
import assert from 'node:assert/strict';
import {mkdtempSync, realpathSync, rmSync} from 'node:fs';
import {tmpdir} from 'node:os';
import {join, resolve} from 'node:path';
import {projectedOffsets, projectInput} from './editor_line_offsets.mjs';
import {pinnedModules} from './pinned_modules.mjs';

const cases = [
    ['body with space after the caret', '@top Main {| }'],
    ['body with space before the caret', '@top Main { |}'],
    ['empty body', '@top Main {|}'],
    ['space and tab', '@top Main { \t|  }'],
    ['tokens body', '@tokens {| }'],
    ['skip body', '@skip { | }'],
    ['precedence body', '@precedence { | }'],
    ['parentheses in a nested expression', '@top Main { (| ) }'],
    ['angles are not a matching pair', '@top Main { Word<| > }'],
    ['token between the braces', '@top Main { |Word }'],
    ['surrounded by a literal', '@top Main { "(| )" }'],
    ['comment between the braces', '@top Main {| /* comment */ }'],
    ['braces on different lines', '@top Main {|\r\n}'],
    ['unicode before the line', '// 😀\r\n@top Main { | }'],
];
assert.equal(process.argv.length, 3, 'usage: node lezer_state.mjs <pinned archive directory>');
const scratch = mkdtempSync(join(realpathSync(tmpdir()), 'gpui-lezer-state.'));
try {
    const {load} = pinnedModules(resolve(process.argv[2]), scratch);
    const {lezer, lezerLanguage} = await load('@codemirror/lang-lezer');
    const {Compartment, EditorState} = await load('@codemirror/state');
    const {foldable, forceParsing, getIndentation, syntaxTree} = await load('@codemirror/language');
    const {insertNewlineAndIndent} = await load('@codemirror/commands');
    const {closeBrackets, insertBracket} = await load('@codemirror/autocomplete');
    const emit = value => console.log(JSON.stringify(value));
    const viewFor = spec => {
        const view = {state: EditorState.create(spec),
            dispatch(tr) { view.state = tr.state ?? view.state.update(tr).state; }};
        return view;
    };
    const complete = view => assert(forceParsing(view, view.state.doc.length, 5000));
    for (const [name, marked] of cases) {
        const [left, right] = marked.split('|');
        const source = left + right;
        const head = left.replaceAll('\r\n', '\n').length;
        const view = viewFor({doc: source, selection: {anchor: head},
            extensions: [lezer(), EditorState.lineSeparator.of('\r\n')]});
        complete(view);
        assert(insertNewlineAndIndent(view));
        emit({kind: 'enter', name, source, head: Buffer.byteLength(left), after: view.state.sliceDoc(),
            position: Buffer.byteLength(view.state.sliceDoc(0, view.state.selection.main.head))});
    }
    const source = '// 😀\r\n@top Main {\r  Word\n}\n';
    const slot = new Compartment();
    const view = viewFor({doc: source, extensions: [slot.of(lezer())]});
    const snapshot = (name, raw, saved = view.state) => {
        const {byteAt} = projectedOffsets(raw);
        const block = saved.doc.line(2);
        const from = saved.doc.line(3).from;
        const fold = foldable(saved, block.from, block.to);
        const rule = saved.languageDataAt('indentOnInput', from);
        emit({kind: 'snapshot', name, source: raw, root: syntaxTree(saved).topNode.name,
            indentAt: byteAt(from), indent: getIndentation(saved, from) ?? null,
            block: [byteAt(block.from), byteAt(block.to)], fold: fold ? [byteAt(fold.from), byteAt(fold.to)] : null,
            comments: saved.languageDataAt('commentTokens', from),
            reindent: rule.map(rule => [' }', ' a}', ' )'].map(text => rule.test(text))),
            brackets: saved.languageDataAt('closeBrackets', from)});
    };
    complete(view);
    const old = view.state;
    snapshot('before edit', source);
    view.dispatch(view.state.update({selection: {anchor: 1}}));
    assert.equal(syntaxTree(view.state), syntaxTree(old));
    snapshot('selection changed', source);
    const from = view.state.doc.line(2).from;
    view.dispatch(view.state.update({changes: {from, insert: ' '}}));
    const edited = source.replace('@top', ' @top');
    complete(view);
    snapshot('edited', edited);
    snapshot('retained', source, old);
    view.dispatch(view.state.update({effects: slot.reconfigure([])}));
    snapshot('detached', edited);
    view.dispatch(view.state.update({effects: slot.reconfigure(lezerLanguage)}));
    complete(view);
    snapshot('reattached', edited);
    for (const [name, marked, input] of [
        ['before literal opener', '@top Main { |"😀" }', '"'],
        ['inside a literal', '@top Main { "😀 | " }', '"'],
        ['before single-quoted literal opener', "@top Main { |'a' }", "'"],
    ]) {
        const [left, right] = marked.split('|');
        const source = left + right;
        const view = viewFor({doc: source, selection: {anchor: left.length}, extensions: [lezer(), closeBrackets()]});
        complete(view);
        const {state} = view;
        const tr = insertBracket(state, input) ?? state.update(state.replaceSelection(input));
        const after = projectInput(source, tr);
        emit({kind: 'quote', name, source, input, head: Buffer.byteLength(left), after,
            position: projectedOffsets(after).byteAt(tr.newSelection.main.head)});
    }
} finally {
    rmSync(scratch, {recursive: true, force: true});
}
