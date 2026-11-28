// Run with Node and a directory of local SHA-512-pinned npm archives.
import assert from 'node:assert/strict';
import {existsSync, mkdtempSync, realpathSync, rmSync} from 'node:fs';
import {tmpdir} from 'node:os';
import {join, resolve} from 'node:path';
import {projectedOffsets} from './editor_line_offsets.mjs';
import {pinnedModules} from './pinned_modules.mjs';

assert.equal(process.argv.length, 3, 'provide the directory of local npm archives');
const archives = resolve(process.argv[2]);
const scratch = mkdtempSync(join(realpathSync(tmpdir()), 'rust-configured-folds.'));
try {
    const {load, pins, installed} = pinnedModules(archives, scratch);
    // Do not let the shared loader download a missing archive.
    for (const {name, version} of pins.values()) {
        const path = join(archives, name.replace('/', '__'), `${name.split('/').at(-1)}-${version}.tgz`);
        assert(existsSync(path), `missing offline archive: ${path}`);
    }
    const {rustLanguage: stock} = await load('@codemirror/lang-rust');
    const {EditorState, Compartment, Prec} = await load('@codemirror/state');
    const {foldable, foldNodeProp, foldService, foldInside, syntaxTree} = await load('@codemirror/language');
    const emit = (caseName, fields) => console.log(JSON.stringify({case: caseName, ...fields}));
    const stockRows = stock.parser.nodeSet.types.filter(row => row.prop(foldNodeProp));
    assert.deepEqual(stockRows.map(row => row.id),
        [7, 29, 43, 54, 56, 58, 59, 61, 78, 96, 108, 109, 114, 141, 156, 161, 181, 185, 189, 194, 209, 211, 216, 232, 233, 242, 245]);
    assert(stockRows.every(row => row.id === 7 || row.prop(foldNodeProp) === foldInside));
    emit('pins', {modules: [...installed.keys()].map(name => {
        const {version, integrity} = pins.get(name);
        return [name, version, integrity];
    }), rows: stockRows.map(row => [row.id, row.name])});

    const source = '// 🦀\r\nfn λ() { \r\n  let s = "字😀";\r  s\r\n}\r\n';
    const nested = 'fn f() { if yes { \r\n  1\r\n} }\r\n';
    const trailingError = 'fn f() {\r\n  let x = 1;\r\n  @\r\n';
    const comments = ['/* open\r\n  more\r\n*/', '/* open\r\n  more\r\n', '/* open\r  😀', '/* open\r\n  😀x'];
    emit('inputs', {source, nested, trailingError, comments});
    const create = (extensions, doc = source) => EditorState.create({doc, extensions});
    const withBlock = (callback, base = stock) => base.configure({props: [foldNodeProp.add(ty => ty.id === 43 ? callback : undefined)]});
    const block = state => syntaxTree(state).topNode.getChild('FunctionItem').getChild('Block');
    const calls = [];
    const callback = (node, state) => {
        assert.equal(node.type, configured.parser.nodeSet.types[43]);
        const children = [];
        for (let child = node.firstChild; child; child = child.nextSibling) children.push(child);
        calls.push({node, parent: node.parent, children, head: state.selection.main.head});
        const range = foldInside(node);
        return range && {from: range.from + Number(state.selection.main.head !== 0), to: range.to - 1};
    };
    const configured = withBlock(callback);
    const disabled = withBlock(null, configured);
    assert.equal(disabled.parser.nodeSet.types[43].prop(foldNodeProp), null);
    assert.equal(configured.parser.nodeSet.types[43].prop(foldNodeProp), callback);
    assert.equal(configured.parser.nodeSet.types[7], stock.parser.nodeSet.types[7]);

    function query(caseName, state, text, lineNo, expected, expectedCalls) {
        assert.equal(state.doc.toString(), text.replace(/\r\n?/g, '\n'));
        const {byteAt} = projectedOffsets(text);
        const b = n => {
            try { return byteAt(n); } catch (error) {
                assert(error instanceof assert.AssertionError);
                return null;
            }
        };
        const {from, to} = state.doc.line(lineNo);
        const result = foldable(state, from, to);
        const fold16 = result && [result.from, result.to];
        const fold = fold16?.map(b) ?? null;
        assert.deepEqual(fold, expected, caseName);
        const span = node => [node.name, node.type.id, byteAt(node.from), byteAt(node.to)];
        const logged = calls.splice(0).map(call => ({
            node: span(call.node), parent: span(call.parent), children: call.children.map(span), head: byteAt(call.head),
        }));
        assert.equal(logged.length, expectedCalls, caseName);
        emit(caseName, {line16: [from, to], line: [byteAt(from), byteAt(to)], fold16, fold, calls: logged});
    }

    const language = new Compartment(), services = new Compartment();
    const initial = create([language.of(configured), services.of([])]);
    const originalTree = syntaxTree(initial);
    assert.equal(block(initial).type, configured.parser.nodeSet.types[43]);
    const selected = initial.update({selection: {anchor: initial.doc.line(3).from}}).state;
    assert.equal(syntaxTree(selected), originalTree);
    query('configured', initial, source, 2, [18, 45], 1);
    query('stock', create(stock), source, 2, [18, 47], 0);
    query('selection only', selected, source, 2, [19, 45], 1);
    const editedSource = source.replace('字', '橋橋');
    const at = selected.doc.toString().indexOf('字');
    const edited = selected.update({changes: {from: at, to: at + 1, insert: '橋橋'}}).state;
    assert.notEqual(syntaxTree(edited), originalTree);
    assert.equal(block(edited).type, configured.parser.nodeSet.types[43]);
    query('edit shifts closing range', edited, editedSource, 2, [19, 48], 1);
    const noRule = selected.update({effects: language.reconfigure(disabled)}).state;
    assert.notEqual(syntaxTree(noRule), originalTree);
    query('explicit removal', noRule, source, 2, null, 0);
    query('restore stock', noRule.update({effects: language.reconfigure(stock)}).state, source, 2, [18, 47], 0);
    query('retained selected', selected, source, 2, [19, 45], 1);
    query('retained initial', initial, source, 2, [18, 45], 1);
    assert.equal(syntaxTree(selected), originalTree);
    assert.equal(syntaxTree(initial), originalTree);

    const serviceCalls = [];
    const providers = result => [
        Prec.low(foldService.of(() => { serviceCalls.push('low'); return {from: 13, to: 37}; })),
        foldService.of(() => { serviceCalls.push('normal'); return result; }),
        Prec.high(foldService.of(() => { serviceCalls.push('high'); return null; })),
    ];
    for (const [name, end, projection] of [['service wins', 31, 40], ['invalid first service result', 999, null]]) {
        const serviced = selected.update({effects: services.reconfigure(providers({from: 15, to: end}))}).state;
        assert.equal(syntaxTree(serviced), originalTree);
        query(name, serviced, source, 2, [19, projection], 0);
        const order = serviceCalls.splice(0);
        assert.deepEqual(order, ['high', 'normal']);
        emit(`${name}, order`, {order});
        query(`removed ${name}`, serviced.update({effects: services.reconfigure([])}).state, source, 2, [19, 45], 1);
        assert.deepEqual(serviceCalls, []);
    }

    const strict = configured.configure({strict: true});
    query('strict valid', create(strict), source, 2, [18, 45], 1);
    const semicolon = initial.doc.toString().indexOf(';');
    const cut = {from: semicolon, to: semicolon + 1};
    const missingSemicolon = source.replace(';', '');
    const recovering = initial.update({changes: cut}).state;
    assert.equal(block(recovering).type, configured.parser.nodeSet.types[43]);
    query('missing semicolon', recovering, missingSemicolon, 2, [18, 44], 1);
    query('missing semicolon, disabled', create(disabled, missingSemicolon), missingSemicolon, 2, null, 0);
    assert.throws(() => syntaxTree(recovering.update({effects: language.reconfigure(strict)}).state), SyntaxError);
    emit('strict rejects missing semicolon', {});
    const repaired = recovering.update({changes: {from: semicolon, insert: ';'}, effects: language.reconfigure(strict)}).state;
    assert.equal(block(repaired).type, strict.parser.nodeSet.types[43]);
    query('repaired strict input', repaired, source, 2, [18, 45], 1);
    const missingBrace = source.replace('}\r\n', '');
    const open = create(configured, missingBrace);
    assert.equal(block(open).type, configured.parser.nodeSet.types[43]);
    assert(block(open).lastChild.type.isError);
    assert.equal(block(open).lastChild.from, block(open).to);
    query('missing closing brace', open, missingBrace, 2, [18, 45], 1);
    query('missing closing brace, disabled', create(disabled, missingBrace), missingBrace, 2, null, 0);
    const error = block(create(stock, trailingError)).lastChild;
    assert(error.type.isError && error.from < error.to);
    query('trailing nonempty error folds to node end', create(stock, trailingError), trailingError, 1, [8, 29], 0);

    query('nested stock outer', create(stock, nested), nested, 1, [8, 27], 0);
    for (const [label, proposal, expected] of [
        ['callback returns null', null, [17, 25]],
        ['callback starts off the line', {from: 19, to: 25}, [17, 25]],
        ['callback ends at the line end', {from: 8, to: 18}, [17, 25]],
        ['invalid outer callback', {from: 8, to: 999}, [8, null]],
    ]) {
        const order = [];
        const parser = withBlock(node => {
            order.push(node.parent.name);
            return node.parent.name === 'FunctionItem' ? proposal : foldInside(node);
        });
        query(label, create(parser, nested), nested, 1, expected, 0);
        assert.deepEqual(order, ['IfExpression', 'FunctionItem']);
        emit(`${label}, order`, {order});
    }
    const fallbackPrefix = 'fn f() { \r\n  1\r\n}\r\n';
    const fallbackTail = 'r#"x";';
    const fallbackCount = 2048;
    const fallbackSource = fallbackPrefix + fallbackTail.repeat(fallbackCount);
    // This input forces the native local fallback, not the pinned parser's.
    const fallbackBlock = stock.parser.parse(fallbackSource).topNode.getChild('FunctionItem').getChild('Block');
    assert.equal(fallbackBlock.type, stock.parser.nodeSet.types[43]);
    assert.equal(disabled.parser.parse(fallbackSource).topNode.getChild('FunctionItem').getChild('Block').type,
        disabled.parser.nodeSet.types[43]);
    emit('local fallback input', {prefix: fallbackPrefix, tail: fallbackTail, count: fallbackCount, term: fallbackBlock.type.id});
    query('local fallback input, pinned stock', create(stock, fallbackSource), fallbackSource, 1, [8, 16], 0);
    query('local fallback input, pinned disabled', create(disabled, fallbackSource), fallbackSource, 1, null, 0);

    [[2, 17], [2, 14], [2, 10], [2, null]].forEach((expected, index) => {
        const comment = comments[index];
        query(`stock comment ${index}`, create(configured, comment), comment, 1, expected, 0);
    });
} finally {
    rmSync(scratch, {recursive: true, force: true});
}
