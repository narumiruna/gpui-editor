// Generate rust_edit_oracle.jsonl with Node 22.14+ and the pinned npm archives:
// node crates/codemirror/tests/fixtures/rust_edit_oracle.mjs <archive-root>
import assert from 'node:assert/strict';
import {execFileSync} from 'node:child_process';
import {mkdtempSync, readFileSync, realpathSync, rmSync} from 'node:fs';
import {tmpdir} from 'node:os';
import {join, resolve} from 'node:path';
import {fileURLToPath} from 'node:url';
import {projectedOffsets} from './editor_line_offsets.mjs';
import {pinnedModules} from './pinned_modules.mjs';

const archives = resolve(process.argv[2]);
const root = realpathSync(mkdtempSync(join(tmpdir(), 'rust-edit-oracle-')));
try {
    const {installed, load, pins} = pinnedModules(archives, root);
    const pkg = await load('@codemirror/lang-rust');
    const inventory = JSON.parse(readFileSync(new URL('../../../../docs/codemirror-upstream-exports.json', import.meta.url)));
    for (const {name, version, tarball, verifiedIntegrity, artifact, modules} of [...inventory.packages, ...inventory.referenceDependencies]) {
        if (!installed.has(name)) continue;
        assert.equal(version, pins.get(name).version);
        assert.equal(verifiedIntegrity, pins.get(name).integrity);
        assert.equal(tarball, pins.get(name).tarball);
        const dir = join(archives, name.replace('/', '__'));
        const archive = join(dir, artifact);
        const {file} = modules.find(mod => mod.specifiers.includes(name));
        assert.equal(fileURLToPath(installed.get(name).url), join(root, 'node_modules', name, file.replace(/^package\//, '')));
        assert.deepEqual(readFileSync(installed.get(name).url), execFileSync('tar', ['-xOf', archive, file]));
        console.error(`verified ${name} ${version} ${file} ${verifiedIntegrity}`);
    }
    const {rust, rustLanguage} = pkg;
    assert.deepEqual(Object.keys(pkg), ['rust', 'rustLanguage']);
    assert.deepEqual(Object.keys(await load('@lezer/rust')), ['parser']);
    const support = rust();
    assert.equal(support.language, rustLanguage);
    assert.deepEqual(support.support, []);
    assert.deepEqual(support.extension, [rustLanguage, []]);
    assert.equal(rustLanguage.name, 'rust');
    const {foldable, forceParsing, getIndentation, indentUnit, IndentContext, syntaxTree} = await load('@codemirror/language');
    const {EditorState} = await load('@codemirror/state');
    const {highlightTree, tags} = await load('@lezer/highlight');
    const extensions = [support, EditorState.tabSize.of(4), indentUnit.of('    ')];
    const config = EditorState.create({extensions});
    assert.deepEqual(config.languageDataAt('commentTokens', 0), [{line: '//', block: {open: '/*', close: '*/'}}]);
    assert.deepEqual(config.languageDataAt('closeBrackets', 0), [{stringPrefixes: ['b', 'r', 'br']}]);
    const trigger = config.languageDataAt('indentOnInput', 0)[0];
    for (const text of ['{', '}', ' \t}', '\uFEFF{', '\n{']) assert(trigger.test(text));
    for (const text of ['', 'x {', '{ ', '}\n', '\u0085}']) assert(!trigger.test(text));

    const names = new Map(Object.entries(tags).filter(([, tag]) => typeof tag !== 'function')
        .map(([name, tag]) => [tag, name]));
    for (const [name, modifiers] of [
        ['variableName', ['definition']], ['variableName', ['function']],
        ['variableName', ['definition', 'function']], ['propertyName', ['function']],
        ['variableName', ['special']], ['string', ['special']],
    ]) {
        names.set(modifiers.reduce((tag, modifier) => tags[modifier](tag), tags[name]),
            [name, ...modifiers].join('.'));
    }
    const highlighter = {style(styles) {
        return styles.map(tag => {
            assert.ok(names.has(tag));
            return names.get(tag);
        }).join(' ');
    }};

    const prefix = '// 🦀\r\nconst FIRST: i32 = 0;\r\nconst SECOND: i32 = 1;\r\nconst THIRD: i32 = 2;\r\n';
    const edits = [
        ['missing-semi', prefix + 'fn λ() {\r\n    let ch = "λ";\r\n    log(ch);\r\n}\r\n', '"λ";', '"λ"'],
        ['missing-operand', 'fn f() {\n    let x = 1 + 2;\n    work(x);\n}\n', '+ 2;', '+ ;'],
        ['eof-block', 'fn f() {\n    let x = 1;\n}\n', '}\n', ''],
        ['eof-call', 'fn f() {\n    work(1,\n         2);\n}\n', ');\n}\n', ''],
        ['eof-param', 'fn f(a: u32,\n    b: u32) {}\n', 'b: u32) {}\n', ''],
        ['unclosed-comment', 'fn f() {\n    let ink = br#"😀"#;\n    /* unclosed\n    */\n}\n', '    */\n}\n', ''],
        ['unclosed-file-comment-crlf', '/* open\r\n  more\r\n*/', '*/', ''],
        ['unclosed-file-comment-astral', '/* open\r  😀*/', '*/', ''],
        ['unclosed-raw', 'fn f() {\n    let x = br##"😀"##;\n    work();\n}\n', '"##;', '"#;'],
        ['unclosed-string', 'fn f() {\n    let x = "é";\n    work();\n}\n', '";', ';'],
        ['stray-scalar', 'fn λ() {\r\n    work();\r\n}\r\n', 'work();', '😀 work();'],
        ['attribute', '#[cfg(\r\n    feature = "🎨"\r\n)]\r\nfn f() {}\r\n', ')]', ']'],
        ['macro', 'fn f() {\n    draw! {\n        [a b]\n    }\n}\n', 'a b]', 'a b)'],
        ['ambiguous-if', prefix + 'fn f() {\r\n    if left && right {\r\n        yes();\r\n    } else {\r\n        no();\r\n    }\r\n}\r\n', '&& right', '&&'],
        ['nested-angle', 'fn f(v: Vec<Vec<u32>>) {\n    call(v);\n}\n', '>>)', '>)'],
        ['ambiguous-closure', 'fn f() {\n    let x = || call();\n    apply(x);\n}\n', '|| call();', '|| ;'],
        ['split-crlf', prefix + 'fn f() {\r\n    call();\r\n}\r\n', '{\r\n    call', '{\r;\n    call'],
        ['lone-cr-after-line-comment', '// 猫😀\rfn f() {\r\n    call(1);\r}\r\n', 'call(1);', 'call( ;'],
    ];
    // Store long runs of spaces as a byte offset and a count in the fixture.
    const largeHead = 'const FIRST: i32 = 0;\r\n// 🦀';
    for (const [bytes, tail, search, replacement] of [
        ...[16383, 16384, 16385, 65536].map(bytes => [bytes, '\r\nconst LAST: i32 = 1;\r\n', '1;', '1']),
        [131072, '\r\nfn last() {\r\n    let x = 1;\r\n}\r\n', '}\r\n', ''],
    ]) {
        const before = largeHead + tail;
        const spaces = bytes - Buffer.byteLength(before.replace(search, replacement));
        edits.push([`large-eof-${bytes}`, before, search, replacement, [largeHead.length, spaces]]);
    }

    const functionBody = '\r\nfn main() {\r    let x = 1 + 2;\r\n    work(x);\n}\r\n';
    for (const bytes of [16384, 16385, 131072]) {
        const before = largeHead + functionBody;
        const replacement = '+ ;';
        const spaces = bytes - Buffer.byteLength(before.replace('+ 2;', replacement));
        edits.push([`large-middle-${bytes}`, before, '+ 2;', replacement, [largeHead.length, spaces]]);
    }
    const earlyHead = 'fn main() {\r\n    work();\r\n}\r//λ';
    const earlyBefore = earlyHead + '\r\nconst LAST: i32 = 7;\r\n';
    const earlySpaces = 65536 - Buffer.byteLength(earlyBefore.replace('work();', '😀 work();'));
    edits.push(['large-early-65536', earlyBefore, 'work();', '😀 work();', [earlyHead.length, earlySpaces]]);

    function complete(state) {
        const view = {state, dispatch(spec) { this.state = this.state.update(spec).state; }};
        assert(forceParsing(view, state.doc.length, 5000));
        return view.state;
    }
    function change(before, after) {
        const a = [...before], b = [...after];
        let from = 0, suffix = 0;
        while (from < a.length && a[from] === b[from]) from++;
        while (suffix < a.length - from && suffix < b.length - from && a.at(-1 - suffix) === b.at(-1 - suffix)) suffix++;
        return [a.slice(0, from).join(''), a.slice(0, a.length - suffix).join(''), b.slice(from, b.length - suffix).join('')];
    }
    function snapshot(state, source) {
        const {byteAt, units} = projectedOffsets(source);
        assert.equal(units, state.doc.length);
        const tree = syntaxTree(state);
        const nodes = [], spans = [], folds = [], indents = [];
        let level = 0;
        tree.iterate({enter: node => nodes.push([level++, node.name, byteAt(node.from), byteAt(node.to)]), leave: () => { level--; }});
        highlightTree(tree, highlighter, (from, to, cls) => spans.push([byteAt(from), byteAt(to), cls]));
        for (let n = 1; n <= state.doc.lines; n++) {
            const {from, to} = state.doc.line(n);
            const fold = foldable(state, from, to);
            folds.push([byteAt(from), byteAt(to), fold ? [byteAt(fold.from), byteAt(fold.to)] : null]);
            for (const pos of new Set([from, to])) {
                const split = new IndentContext(state, {simulateBreak: pos, simulateDoubleBreak: true});
                indents.push([byteAt(pos), getIndentation(state, pos) ?? null, getIndentation(split, pos) ?? null]);
            }
        }
        return {nodes, spans, folds, indents};
    }
    for (const [id, before, search, replacement, padding] of edits) {
        const at = before.indexOf(search);
        assert(at >= 0);
        const broken = before.slice(0, at) + replacement + before.slice(at + search.length);
        let previous, previousState;
        for (const compact of [before, broken, before]) {
            const [padAt, spaces] = padding ?? [0, 0];
            const head = compact.slice(0, padAt);
            const source = head + ' '.repeat(spaces) + compact.slice(padAt);
            const cold = complete(EditorState.create({doc: source, extensions}));
            const expected = snapshot(cold, source);
            let edit = null;
            if (previous !== undefined) {
                const [a, b, insert] = change(previousState.doc.toString(), cold.doc.toString());
                const updated = complete(previousState.update({changes: {from: a.length, to: b.length, insert}}).state);
                assert.deepEqual(snapshot(updated, source), expected, id);
                const [c, d, raw] = change(previous, source);
                edit = [Buffer.byteLength(c), Buffer.byteLength(d), raw];
            }
            console.log(JSON.stringify({id, source: compact, edit, ...expected,
                padding: padding && [Buffer.byteLength(head), spaces]}));
            previous = source;
            previousState = cold;
        }
    }
} finally {
    rmSync(root, {recursive: true, force: true});
}
