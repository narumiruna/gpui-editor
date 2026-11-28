// Run with Node and a directory of local SHA-512-pinned npm archives.
import assert from 'node:assert/strict';
import {existsSync, mkdtempSync, realpathSync, rmSync} from 'node:fs';
import {tmpdir} from 'node:os';
import {join, resolve} from 'node:path';
import {projectedOffsets} from './editor_line_offsets.mjs';
import {pinnedModules} from './pinned_modules.mjs';

assert.equal(process.argv.length, 3, 'provide the directory of local npm archives');
const archives = resolve(process.argv[2]);
const scratch = mkdtempSync(join(realpathSync(tmpdir()), 'cpp-configured-services.'));
try {
    const {load, pins, installed} = pinnedModules(archives, scratch);
    for (const {name, version} of pins.values()) {
        assert(existsSync(join(archives, name.replace('/', '__'), `${name.split('/').at(-1)}-${version}.tgz`)), name);
    }
    const {cppLanguage: stock, cpp} = await load('@codemirror/lang-cpp');
    const {parser: raw} = await load('@lezer/cpp');
    const {NodeProp} = await load('@lezer/common');
    const {Compartment, EditorState, Prec} = await load('@codemirror/state');
    const {ViewPlugin} = await load('@codemirror/view');
    const {
        bracketMatching, bracketMatchingHandle, foldable, foldInside, foldNodeProp, foldService,
        matchBrackets, syntaxTree,
    } = await load('@codemirror/language');
    const emit = (name, fields) => console.log(JSON.stringify({case: name, ...fields}));
    const rows = stock.parser.nodeSet.types.flatMap(row => {
        const rule = row.prop(foldNodeProp);
        return rule ? [[row.id, row.name, rule === foldInside ? 'inside' : 'comment']] : [];
    });
    assert.deepEqual(rows, [[5, 'BlockComment', 'comment'], [78, 'FieldDeclarationList', 'inside'],
        [126, 'CompoundStatement', 'inside'], [136, 'InitializerList', 'inside'], [260, 'EnumeratorList', 'inside']]);
    assert(!stock.parser.nodeSet.types.some(row => row.name === 'DeclarationList'));
    const roles = parser => parser.nodeSet.types.flatMap(row => {
        assert.equal(row.prop(bracketMatchingHandle), undefined);
        const opened = row.prop(NodeProp.openedBy), closed = row.prop(NodeProp.closedBy);
        return opened || closed ? [[row.id, row.name, opened ?? null, closed ?? null]] : [];
    });
    const stockRoles = roles(stock.parser);
    assert.deepEqual(stockRoles, roles(raw));
    assert.deepEqual(stockRoles, [[12, ')', ['('], null], [13, '(', null, [')']], [51, '{', null, ['}']],
        [52, '}', ['{'], null], [53, '[', null, [']']], [54, ']', ['['], null]]);
    assert.equal(cpp().language, stock);
    emit('pins', {modules: [...installed.keys()].sort().map(name => [name, pins.get(name).version, pins.get(name).integrity]), rows, stockRoles});
    const make = (doc, extensions = stock) => EditorState.create({doc, extensions});
    const span = (n, b) => n && [n.name, n.type.id, b(n.from), b(n.to)];
    const source = '// 🦀\r\nint f() { \r\n  const char*s = "字😀";\r\n  return 1;\r\n}\r\n';
    const ascii = 'int f() { \r\n  return 1;\r\n}\r\n';
    const nested = 'int f() { if (true) { \r\n  return 1;\r\n} }\r\n';
    const stockLists = 'struct S {\r\n  int x;\r\n};\r\nenum E {\r\n  A\r\n};\r\nint a[] = {\r\n  1\r\n};\r\n';
    const comments = ['/* open\r\n  more\r\n*/', '/* open\r\n  more\r\n', '/* open\r  😀', '/* open\r\n  😀x'];
    emit('fold inputs', {source, ascii, nested, stockLists, comments});
    const calls = [];
    const withBlock = (rule, base = stock) => base.configure({props: [foldNodeProp.add(t => t.id === 126 ? rule : undefined)]});
    const changed = withBlock((node, state) => {
        assert.equal(node.type, changed.parser.nodeSet.types[126]);
        const b = projectedOffsets(state.doc.toString() === source.replace(/\r\n/g, '\n') ? source : editedSource).byteAt;
        const children = [];
        for (let child = node.firstChild; child; child = child.nextSibling) children.push(span(child, b));
        const head = state.selection.main.head;
        calls.push({node: span(node, b), parent: span(node.parent, b), children, head: b(head)});
        const inside = foldInside(node);
        return inside && {from: inside.from + Number(head !== 0), to: inside.to - 1};
    });
    const removed = withBlock(null, changed);
    assert.equal(removed.parser.nodeSet.types[126].prop(foldNodeProp), null);
    assert.equal(removed.parser.nodeSet.types[5], stock.parser.nodeSet.types[5]);
    function query(name, state, text, line, expected, numCalls = 0) {
        const {byteAt} = projectedOffsets(text);
        const {from, to} = state.doc.line(line);
        assert.equal(state.doc.toString(), text.replace(/\r\n?/g, '\n'));
        const found = foldable(state, from, to);
        const fold16 = found && [found.from, found.to];
        const fold = fold16?.map(n => { try { return byteAt(n); } catch { return null; } }) ?? null;
        assert.deepEqual(fold, expected, name);
        const logged = calls.splice(0);
        assert.equal(logged.length, numCalls, name);
        emit(name, {line: [byteAt(from), byteAt(to)], fold16, fold, calls: logged});
    }
    const language = new Compartment(), services = new Compartment();
    const initial = make(source, [language.of(changed), services.of([])]);
    const initialTree = syntaxTree(initial);
    const selected = initial.update({selection: {anchor: initial.doc.line(3).from}}).state;
    assert.equal(syntaxTree(selected), initialTree);
    const at = selected.doc.toString().indexOf('字'), editedSource = source.replace('字', '橋橋');
    const edited = selected.update({changes: {from: at, to: at + 1, insert: '橋橋'}}).state;
    assert.notEqual(syntaxTree(edited), initialTree);
    const disabled = selected.update({effects: language.reconfigure(removed)}).state;
    const restored = disabled.update({effects: language.reconfigure(stock)}).state;
    for (const [name, state, text, fold, n] of [
        ['configured', initial, source, [18, 61], 1], ['stock', make(source), source, [18, 63], 0],
        ['selection only', selected, source, [19, 61], 1], ['edit', edited, editedSource, [19, 64], 1],
        ['explicit removal', disabled, source, null, 0], ['restore', restored, source, [18, 63], 0],
        ['retained selected', selected, source, [19, 61], 1], ['retained initial', initial, source, [18, 61], 1],
    ]) query(name, state, text, 2, fold, n);
    query('stock ASCII CRLF', make(ascii), ascii, 1, [9, 25]);
    query('removed ASCII CRLF', make(ascii, removed), ascii, 1, null);
    for (const [line, expected] of [[1, [10, 22]], [4, [34, 41]], [7, [56, 63]]]) {
        query(`stock lists ${line}`, make(stockLists, changed), stockLists, line, expected);
    }
    for (const [index, expected] of [[0, [2, 17]], [1, [2, 14]], [2, [2, 10]], [3, [2, null]]]) {
        query(`comment ${index}`, make(comments[index], changed), comments[index], 1, expected);
    }
    const order = [];
    for (const [name, to, expected] of [['service wins', initial.doc.line(4).from, [18, 50]], ['invalid first service', 999, [18, null]]]) {
        const service = (id, result) => foldService.of(() => { order.push(id); return result; });
        const entries = [Prec.low(service('low', {from: 15, to: 40})),
            service('normal', {from: 15, to}), Prec.high(service('high', null))];
        const serviced = selected.update({effects: services.reconfigure(entries)}).state;
        assert.equal(syntaxTree(serviced), initialTree);
        query(name, serviced, source, 2, expected);
        assert.deepEqual(order, ['high', 'normal']);
        emit(`${name} order`, {order: order.splice(0)});
    }
    query('nested outer wins', make(nested), nested, 1, [9, 39]);
    for (const [name, proposal, expected] of [
        ['outer returns null', null, [21, 37]],
        ['outer starts after line', {from: 23, to: 35}, [21, 37]],
        ['outer ends at line end', {from: 9, to: 22}, [21, 37]],
        ['outer past EOF', {from: 9, to: 999}, [9, null]],
    ]) {
        const outer = withBlock(node => {
            order.push(node.parent.name);
            return node.parent.name === 'FunctionDefinition' ? proposal : foldInside(node);
        });
        query(name, make(nested, outer), nested, 1, expected);
        assert.deepEqual(order, ['IfStatement', 'FunctionDefinition']);
        emit(`${name} order`, {order: order.splice(0)});
    }

    const selectors = {second: node => node.firstChild?.nextSibling, skip: () => null};
    const configure = (props, handles = selectors) => stock.configure({props: [
        NodeProp.closedBy.add(type => props.closedBy?.[type.id]),
        NodeProp.openedBy.add(type => props.openedBy?.[type.id]),
        bracketMatchingHandle.add(type => handles[props.handles?.[type.id]]),
    ]});
    const pluginFor = options => bracketMatching(options).flat(Infinity).find(item => item instanceof ViewPlugin);
    const options = {brackets: '<>', maxScanDistance: 1};
    const marked = {closedBy: {102: ['LambdaExpression']}, openedBy: {104: ['ParenthesizedExpression']},
        handles: {102: 'second', 104: 'second'}};
    const brackets = 'void f() { g((¦1¦2¦), [x]{ ¦return R"(😀\r\n字)"; }); }\r\n';
    const output = (state, input, options, plugin) => {
        const {byteAt} = projectedOffsets(input);
        const head = state.selection.main.head;
        const bspan = range => range ? [byteAt(range.from), byteAt(range.to)] : null;
        const hit = value => value ? [bspan(value.start), bspan(value.end), value.matched] : null;
        const marks = [];
        for (const it = plugin.create({state}).decorations.iter(); it.value; it.next()) marks.push([...bspan(it), it.value.spec.class]);
        return {head: byteAt(head), directional: [-1, 1].map(dir => hit(matchBrackets(state, head, dir, options))), marks};
    };
    for (const [label, annotated, props, config, forward] of [
        ['selected handles', brackets, marked, options, [[14, 16], [22, 48], true]],
        ['mismatched name', brackets, {...marked, closedBy: {102: ['Identifier']}}, options, [[14, 16], [22, 48], false]],
        ['skipped mate', brackets, {...marked, handles: {...marked.handles, 104: 'skip'}}, options, [[14, 16], null, true]],
        ['skipped start falls back to text', 'void f() { /* ¦[😀] */ }\r\n', {closedBy: {5: ['BlockComment']}, handles: {5: 'skip'}}, {}, [[14, 15], [19, 20], true]],
        ['empty stock roles', 'void f¦() {}\r\n', {closedBy: {13: []}, openedBy: {12: []}}, options, [[6, 7], [7, 8], false]],
        ['stock roles independent of options', 'void f¦() {}\r\n', {}, options, [[6, 7], [7, 8], true]],
    ]) {
        const parts = annotated.split('¦'), text = parts.join('');
        const plugin = pluginFor(config);
        const state = make(text, [configure(props), bracketMatching(config)]);
        let anchor = 0;
        const queries = parts.slice(0, -1).map(part => {
            anchor += projectedOffsets(part).units;
            return output(state.update({selection: {anchor}}).state, text, config, plugin);
        });
        assert.deepEqual(queries[0].directional[1], forward, label);
        emit('brackets', {label, source: text, props, options: config, queries});
    }
    const template = 'const char *s = "字";\r\nstd::vector<int> xs;\r\n';
    const props = {closedBy: {33: []}, openedBy: {66: []}};
    const plugin = pluginFor(options);
    const bracketLanguage = new Compartment();
    const before = make(template, [bracketLanguage.of(configure(props)), bracketMatching(options)]);
    const anchor = before.doc.toString().indexOf('<');
    const cursor = before.update({selection: {anchor}}).state;
    assert.equal(syntaxTree(cursor), syntaxTree(before));
    const from = before.doc.toString().indexOf('字'), insert = '🦀🦀';
    const edit = cursor.update({changes: {from, to: from + 1, insert}}).state;
    const editText = template.replace('字', insert);
    const reset = edit.update({effects: bracketLanguage.reconfigure(stock)}).state;
    const {byteAt} = projectedOffsets(template);
    const summary = [cursor, edit, reset, cursor, edit].map((state, i) => output(state, i % 3 === 0 ? template : editText, options, plugin));
    assert.deepEqual(summary[0], summary[3]);
    assert.deepEqual(summary[1], summary[4]);
    assert.equal(summary[0].directional[1][2], false);
    assert.equal(summary[2].directional[1][2], true);
    emit('brackets retained', {source: template, props, options, edit: {from: byteAt(from), to: byteAt(from + 1), insert}, queries: summary});

    const foreignSource = 'int f() { return (12); }';
    const foreignTree = stock.parser.parse(foreignSource);
    const foreign = foreignTree.resolveInner(foreignSource.indexOf('12'), 1);
    const foreignProps = {closedBy: {102: []}, handles: {102: 'foreign'}};
    const foreignState = make(foreignSource, configure(foreignProps, {foreign: () => foreign}));
    assert.notEqual(foreignTree, syntaxTree(foreignState));
    const foreignHead = foreignSource.indexOf('12');
    const hit = matchBrackets(foreignState, foreignHead, 1, options);
    assert.deepEqual(hit, {start: {from: 18, to: 20}, matched: false});
    emit('foreign tree pin accepts offsets', {source: foreignSource, head: foreignHead, hit});
} finally {
    rmSync(scratch, {recursive: true, force: true});
}
