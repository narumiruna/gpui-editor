// From the repository root:
// node crates/codemirror/tests/fixtures/wast_oracle.mjs /private/tmp/gpui-upstream-exports.UVI6VVWZ \
//   > crates/codemirror/tests/fixtures/wast_oracle.jsonl
import assert from 'node:assert/strict';
import {execFileSync} from 'node:child_process';
import {createHash} from 'node:crypto';
import {readFileSync, realpathSync} from 'node:fs';
import {createRequire, findPackageJSON} from 'node:module';
import {dirname, join, resolve} from 'node:path';
import {pathToFileURL} from 'node:url';

const root = resolve(process.argv[2]);
const manifest = JSON.parse(readFileSync(new URL('../../../../docs/codemirror-upstream.json', import.meta.url)));
const packages = [...manifest.packages, ...manifest.referenceDependencies];
const modules = new Map();
for (const name of ['@codemirror/lang-wast', '@codemirror/state', '@codemirror/language',
                    '@codemirror/view', '@codemirror/streamparser',
                    '@lezer/common', '@lezer/lr', '@lezer/highlight']) {
    const pin = packages.find(row => row.name === name);
    const archive = join(root, name.replace('/', '__'), pin.tarball.split('/').at(-1));
    const hash = createHash('sha512').update(readFileSync(archive)).digest('base64');
    assert.equal(`sha512-${hash}`, pin.integrity, name);
    const dir = realpathSync(join(root, 'node_modules', name));
    assert.equal(dir, join(dirname(archive), 'package'));
    for (const file of ['package.json', 'dist/index.js', 'dist/index.cjs']) {
        assert.deepEqual(readFileSync(join(dir, file)), execFileSync('tar', ['-xOf', archive, `package/${file}`]), `${name}/${file}`);
    }
    const pkg = JSON.parse(readFileSync(join(dir, 'package.json')));
    assert.equal(pkg.name, name);
    assert.equal(pkg.version, pin.version);
    assert.equal(pkg.exports.import ?? pkg.exports['.'].import, './dist/index.js');
    modules.set(name, {dir, deps: Object.keys(pkg.dependencies ?? {})});
    console.error(`${name} ${pin.version}: verified archive and resolved files`);
}
for (const {dir, deps} of modules.values()) {
    for (const dep of deps.filter(dep => modules.has(dep))) {
        const resolved = findPackageJSON(dep, pathToFileURL(join(dir, 'dist/index.js')));
        assert.equal(realpathSync(resolved), join(modules.get(dep).dir, 'package.json'));
    }
}
const load = name => import(pathToFileURL(join(modules.get(name).dir, 'dist/index.js')));
const wastModule = await load('@codemirror/lang-wast');
const {wast, wastLanguage} = wastModule;
const {EditorState} = await load('@codemirror/state');
const {LRLanguage, LanguageSupport, forceParsing, foldable, getIndentation, indentUnit,
       IndentContext, language, syntaxTree, matchBrackets} = await load('@codemirror/language');
const {NodeProp, Tree} = await load('@lezer/common');
const {LRParser} = await load('@lezer/lr');
const {highlightTree, tags, tagHighlighter} = await load('@lezer/highlight');
const exports = ['wast', 'wastLanguage'];
assert.deepEqual(Object.keys(wastModule), exports);
const require = createRequire(join(root, 'node_modules', 'verify-wast.cjs'));
assert.deepEqual(Object.keys(require('@codemirror/lang-wast')).sort(), exports);
assert.ok(wastLanguage instanceof LRLanguage);
assert.ok(wastLanguage.parser instanceof LRParser);
assert.equal(wastLanguage.name, 'wast');
assert.equal(wastLanguage.allowsNesting, false);
const support = wast();
assert.ok(support instanceof LanguageSupport);
assert.equal(support.language, wastLanguage);
assert.deepEqual(support.support, []);
assert.notEqual(wast(), support);

const tagNames = ['paren', 'variableName', 'typeName', 'keyword', 'number', 'string', 'lineComment', 'blockComment'];
const highlighter = tagHighlighter(tagNames.map(name => ({tag: tags[name], class: name})));
const normalize = text => text.replace(/\r\n?/g, '\n');
const bytes = text => Buffer.byteLength(text);
const byteAt = (text, at) => bytes(text.slice(0, at));
const offsets = text => {
    const map = [0];
    let at = 0;
    for (const ch of text.match(/\r\n|[^]/gu) ?? []) {
        if (ch.length === 2 && ch !== '\r\n') map.push(null);
        map.push(at += bytes(ch));
    }
    return map;
};

// FNV-1a over compact JSON arrays. Store record counts too. This keeps the
// checked-in fixture small without sampling the recorded ranges.
function signature(rows) {
    let hash = 2166136261;
    for (const byte of Buffer.from(JSON.stringify(rows))) hash = Math.imul(hash ^ byte, 16777619);
    return [rows.length, hash >>> 0];
}

function snapshot(state, tree, source) {
    const map = offsets(source);
    assert.ok(tree instanceof Tree);
    const nodes = [], highlights = [], indents = [], folds = [];
    let depth = 0, errors = 0;
    tree.iterate({enter: n => {
        nodes.push([depth++, n.name, map[n.from], map[n.to]]);
        if (n.type.isError) errors++;
    }, leave: () => { depth--; }});
    highlightTree(tree, highlighter, (from, to, tag) => highlights.push([map[from], map[to], tag]));
    for (let pos = 0; pos < map.length; pos++) {
        if (map[pos] === null) continue;
        const context = options => new IndentContext(state, options);
        indents.push([map[pos], getIndentation(state, pos),
            getIndentation(context({simulateBreak: pos}), pos),
            getIndentation(context({simulateBreak: pos, simulateDoubleBreak: true}), pos),
            getIndentation(context({overrideIndentation: from => from === 0 ? 7 : -1}), pos)]);
    }
    for (let n = 1; n <= state.doc.lines; n++) {
        const {from, to} = state.doc.line(n);
        const fold = foldable(state, from, to);
        folds.push([map[from], map[to], fold ? [map[fold.from], map[fold.to]] : null]);
    }
    return {errors, nodes: signature(nodes), highlights: signature(highlights),
            indents: signature(indents), folds: signature(folds)};
}

function complete(state) {
    // Use the headless part of the view's parse/dispatch protocol. Services
    // must see the completed state tree, not a detached parser result.
    const view = {state, dispatch(spec) { this.state = this.state.update(spec).state; }};
    assert.ok(forceParsing(view, state.doc.length, 5000));
    assert.equal(syntaxTree(view.state).length, state.doc.length);
    return view.state;
}

// Count actual shared tree objects, not just the presence of a fragment.
function objects(tree) {
    const result = new Set();
    const queue = [...tree.children];
    for (const child of queue) {
        result.add(child);
        if (child instanceof Tree) queue.push(...child.children);
    }
    return result;
}

// The three edit coordinates are UTF-16 indexes into the original spelling.
// Enclose a changed CRLF seam before converting a batch to editor coordinates.
function transaction(source, edits) {
    return edits.map(([start, end, insert]) => {
        const from = source[start - 1] === '\r' ? start - 1 : start;
        const to = source[end] === '\n' ? end + 1 : end;
        const old = normalize(source.slice(from, to));
        const replacement = normalize(source.slice(from, start) + insert + source.slice(end, to));
        const at = normalize(source.slice(0, from)).length;
        return {from: at, to: at + old.length, insert: replacement, old};
    }).filter(edit => edit.old !== edit.insert);
}

const replace = (source, old, insert) => {
    const start = source.indexOf(old);
    assert.ok(start >= 0 && source.indexOf(old, start + 1) < 0, JSON.stringify(old));
    return [start, start + old.length, insert];
};
const settings = [EditorState.tabSize.of(8), indentUnit.of('    ')];
function replay(name, source, steps) {
    let state = complete(EditorState.create({doc: source, extensions: [support, ...settings]}));
    let tree = syntaxTree(state);
    console.log(JSON.stringify({name, source, expected: snapshot(state, tree, source)}));
    for (const [name, batch] of steps) {
        const edits = batch(source).sort((a, b) => a[0] - b[0]);
        const changes = edits.map(([from, to, insert]) => [byteAt(source, from), byteAt(source, to), insert]);
        state = complete(state.update({changes: transaction(source, edits)}).state);
        for (const [from, to, insert] of edits.slice().reverse()) {
            source = source.slice(0, from) + insert + source.slice(to);
        }
        assert.equal(state.doc.toString(), normalize(source));
        const next = syntaxTree(state);
        const prior = objects(tree);
        const shared = [...objects(next)].filter(child => prior.has(child)).length;
        const expected = snapshot(state, next, source);
        const fresh = complete(EditorState.create({doc: source, extensions: [wastLanguage.extension, ...settings]}));
        assert.deepEqual(expected, snapshot(fresh, syntaxTree(fresh), source), name);
        console.log(JSON.stringify({name, changes, shared, expected}));
        tree = next;
    }
}

const funcs = (first, count) => Array.from({length: count}, (_, index) => {
    const n = first + index;
    const eol = ['\r\n', '\n', '\r'][n % 3];
    return `  (func $f${n} (param $x i32) (result i32)${eol}` +
           `    local.get $x i32.const ${n} i32.add) ;; résumé 😀 ${n}${eol}`;
}).join('');
replay('module and script assertions',
    '(module $calc\r\n  (memory (export "mémoire") 1)\n' + funcs(0, 32) +
    '\t(func $patch (result i32)\r    i32.const -0x1_f)\r\n' +
    '  (; aide 🐈\r  read-only ;)\n  (export "résultat 😀" (func $patch))\r\n' +
    funcs(32, 32) + ') ;; close module\r' +
    '(assert_return (invoke "résultat 😀") (i32.const -31))\n', [
    ['strict disjoint replacements', s => [replace(s, '-0x1_f', '+0x20'), replace(s, 'read-only', 'inspect 🦀')]],
    ['split and join CRLF seams', s => {
        const drop = s.indexOf('\r\n', s.indexOf('i32.const +0x20)'));
        const join = s.indexOf('\r', s.indexOf('(; aide')) + 1;
        return [[drop, drop + 1, ''], [join, join, '\n'], replace(s, 'export "résultat 😀"', 'export "東京 😀"')];
    }],
    ['break string, number and module close', s => [replace(s, '+0x20', '??? 0x_'),
        replace(s, '"東京 😀" (func', '"東京 😀 (func'), replace(s, ') ;; close module', ';; close module')]],
    ['repair string and introduce stray close', s => [replace(s, '"東京 😀 (func', '"東京 😀" (func'),
        replace(s, ';; résumé 😀 35', ';; résumé 😀 35\r  ))')]],
    ['repair remaining errors', s => [replace(s, '??? 0x_', '32'),
        replace(s, '35\r  ))', '35'), replace(s, ';; close module', ') ;; close module')]],
    ['strict batch after repair', s => [replace(s, 'i32.const 32)\n', 'i32.const 64)\n'),
        replace(s, 'inspect 🦀', 'inspect 🐈 again')]],
]);

const data = 'abc\\41😀'.repeat(134);
replay('long tokens and late recovery',
    '(module\r\n' + funcs(100, 29) +
    `  (data (i32.const 0) "payload:${data}\r\nsecond block\\nשלום")\n` +
    '  (; docs:\r\n' + '    abcdefg chunk\r\n'.repeat(39) + '    last chunk ;)\r' +
    funcs(129, 29) + ') ;; end payload module\n', [
    ['shift long token and replace comment delimiter', s => [replace(s, 'payload:', 'payload新版:'),
        replace(s, 'last chunk ;)', 'last chunk )')]],
    ['repair comment and remove trailing close', s => [replace(s, 'last chunk )', 'last chunk ;)'),
        replace(s, ') ;; end payload module', ';; end payload module')]],
    ['repair late EOF and change data', s => [replace(s, 'second block\\nשלום', 'second 🦀 block'),
        replace(s, ';; end payload module', ') ;; end payload module')]],
]);

// Install both public forms, then edit without reinstalling either one.
const source = ';; שלום 😀\r(module\r\n  (; doc\rsecond line ;)\n  (func $f (result i32) i32.const 0)\r\n  (export "ق )" (func $f))\r\n)';
function installed(extension) {
    let state = EditorState.create({doc: source, extensions: extension});
    function lookup(raw) {
        assert.equal(state.facet(language), wastLanguage);
        state = complete(state);
        const tree = syntaxTree(state);
        assert.equal(state.doc.toString(), normalize(raw));
        const map = offsets(raw);
        const keys = ['commentTokens', 'closeBrackets', 'autocomplete', 'indentOnInput', 'wordChars'];
        const at = (pos, side = -1) => keys.map(key => state.languageDataAt(key, pos, side));
        for (const pos of [0, 1, state.doc.toString().indexOf('export'), state.doc.length]) {
            for (const side of [-1, 1]) {
                assert.ok(wastLanguage.isActiveAt(state, pos, side));
                assert.deepEqual(at(pos, side), at(0));
            }
        }
        const props = [];
        tree.iterate({enter: n => {
            const isolation = n.type.prop(NodeProp.isolate) ?? null;
            const openedBy = n.type.prop(NodeProp.openedBy) ?? null;
            const closedBy = n.type.prop(NodeProp.closedBy) ?? null;
            const group = n.type.prop(NodeProp.group) ?? null;
            if (isolation !== null || openedBy || closedBy || group) {
                props.push([n.name, map[n.from], map[n.to], isolation, openedBy, closedBy, group]);
            }
        }});
        const bracket = matchBrackets(state, state.doc.toString().indexOf('(module'), 1);
        assert.ok(bracket.matched);
        return {data: at(0), props, regions: wastLanguage.findRegions(state).map(({from, to}) => [map[from], map[to]]),
                bracket: [[map[bracket.start.from], map[bracket.start.to]], [map[bracket.end.from], map[bracket.end.to]]]};
    }
    const initial = lookup(source);
    state = state.update({changes: {from: 0, insert: '\n'}}).state;
    return {initial, edited: lookup('\n' + source)};
}
const installation = installed(support);
assert.deepEqual(installation, installed(wastLanguage.extension));
console.log(JSON.stringify({name: 'support installation', source, ...installation}));
