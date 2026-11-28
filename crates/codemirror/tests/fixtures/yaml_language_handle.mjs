// Run with Node and a directory of local SHA-512-pinned npm archives.
import assert from 'node:assert/strict';
import {mkdtempSync, realpathSync, rmSync} from 'node:fs';
import {tmpdir} from 'node:os';
import {join, resolve} from 'node:path';
import {projectedOffsets} from './editor_line_offsets.mjs';
import {pinnedModules} from './pinned_modules.mjs';

assert.equal(process.argv.length, 3, 'provide the directory of local npm archives');
const scratch = mkdtempSync(join(realpathSync(tmpdir()), 'yaml-language-handle.'));
try {
    const {load} = pinnedModules(resolve(process.argv[2]), scratch);
    const {yaml, yamlLanguage: stock} = await load('@codemirror/lang-yaml');
    const {TreeBuffer} = await load('@lezer/common');
    const {EditorState, Compartment, Prec} = await load('@codemirror/state');
    const {language, languageDataProp, syntaxTree} = await load('@codemirror/language');
    const {languages} = await load('@codemirror/language-data');
    const {json} = await load('@codemirror/lang-json');
    const entry = languages.find(entry => entry.name === 'YAML');
    const catalog = await entry.load();
    assert.equal(await entry.load(), catalog);
    const named = stock.configure({}, 'named YAML');
    const strict = named.configure({strict: true}, 'strict YAML');
    const inherited = strict.configure({}, '');
    const recovery = inherited.configure({strict: false});
    assert.notEqual(yaml(), yaml());
    assert.equal(yaml().language, stock);
    assert.equal(catalog.language, stock);
    assert.deepEqual(yaml().support, []);
    for (const handle of [named, strict, inherited, recovery]) {
        assert.notEqual(handle.parser, stock.parser);
        assert.equal(handle.data, stock.data);
        assert.equal(handle.parser.topNode, stock.parser.topNode);
    }
    assert.equal(recovery.name, strict.name);
    assert.equal(stock.parser.topNode.name, 'Stream');
    assert.equal(stock.parser.topNode.prop(languageDataProp), stock.data);

    const emit = (kind, fields) => console.log(JSON.stringify({kind, ...fields}));
    const rows = (tree, byteAt) => {
        const treeRows = [];
        let depth = 0;
        tree.iterate({
            enter(node) { treeRows.push([depth++, node.name, byteAt(node.from), byteAt(node.to)]); },
            leave() { depth--; },
        });
        return treeRows;
    };
    const result = (run, byteAt) => {
        try {
            return {tree: rows(run(), byteAt)};
        } catch (error) {
            const at = /^No parse at (\d+)$/.exec(error.message);
            if (!(error instanceof SyntaxError) || !at) throw error;
            return {offset: byteAt(Number(at[1]))};
        }
    };
    const rawByte = source => at => {
        assert(!/^[\uDC00-\uDFFF]/.test(source.slice(at, at + 1)), 'offset bisects surrogate pair');
        return Buffer.byteLength(source.slice(0, at));
    };
    const make = (doc, extensions) => EditorState.create({doc, extensions});
    const valid = '名: [😀, 値]\r\n';
    for (const [name, source] of [
        ['empty', ''], ['valid', valid],
        ['block scalar CRLF', 'key: |\r\n  😀\r\nnext: ok\r\n'],
        ['CRLF comment', '# 😀\r\n名: [値]\r\n'],
        ['lone CR comment', '# 😀\r名: [値]\r'],
        ['lone CR before an incomplete value', '# 😀\r名: [値'],
        ['multiple leading comments', '# a\n# 😀\n# z\n\n  k: v'],
        ['nested maps in a sequence', '- # a\n  # 😀\n  k: v\n- # b\n  q: w\n'],
        ['explicit mapping', '# a\n# 😀\n# z\n? k\n: v'],
        ['between two pairs', '# pre\nk: v\n# mid\np: q\n# end\n'],
        ['incomplete flow after comments', '# a\r\n# 😀\r\nk: [v'],
        ['implicit flow pair', 'root: [\r\n  😀,\r  名: 値\r\n]'],
        ['missing closer', valid.replace(']', '')],
        ['bad token after CRLF', '\r\n名: [😀, @]\r\n'],
        ['no schema validation', 'a: yes\na: no\n'],
    ]) {
        // Raw Rust trees match the unbuffered pin, not a wrapped packed start.
        const raw = handle => result(() => handle.parser.configure({bufferLength: 0}).parse(source), rawByte(source));
        const editor = extension => result(() => syntaxTree(make(source, extension)), projectedOffsets(source).byteAt);
        assert.deepEqual(raw(recovery), raw(stock));
        assert.deepEqual(raw(inherited), raw(strict));
        emit('parse', {name, source, raw: raw(stock), strictRaw: raw(strict),
            editor: editor(yaml()), strictEditor: editor(strict)});
    }

    // The first comment sets a nonzero buffer base. The Document and
    // BlockMapping start at zero, so their relative starts wrap in Uint16.
    // With a short input, iteration hides them. With a long trailing comment,
    // the wrapped start is inside the root, and iteration shows reversed ranges.
    for (const eol of ['\n', '\r\n']) {
        for (const tailLength of [4, 1 << 16]) {
            const source = ['', '# a', '# b', 'k: v', '#' + 'x'.repeat(tailLength)].join(eol);
            const raw = rows(stock.parser.configure({bufferLength: 0}).parse(source), rawByte(source));
            const packed = stock.parser.parse(source);
            const [buffer] = packed.children;
            assert(buffer instanceof TreeBuffer);
            assert.equal(packed.positions[0], eol.length);
            const relativeStart = (1 << 16) - packed.positions[0];
            const wrapped = [];
            for (let i = 0; i < buffer.buffer.length; i += 4) {
                if (buffer.buffer[i + 1] === relativeStart) wrapped.push(buffer.set.types[buffer.buffer[i]].name);
            }
            assert.deepEqual(wrapped, ['Document', 'BlockMapping']);
            const packedUtf16 = rows(packed, at => at);
            assert.deepEqual(rows(strict.parser.parse(source), at => at), packedUtf16);
            const hiddenNodes = raw.length - packedUtf16.length;
            const reversedUtf16 = packedUtf16.filter(([, , from, to]) => from > to);
            assert.deepEqual([hiddenNodes, reversedUtf16.length], tailLength === 4 ? [8, 0] : [0, 2]);
            emit('rawPacking', {eol, tailLength, raw, packed: {relativeStart, hiddenNodes, reversedUtf16}});
        }
    }

    for (const [name, source] of [
        ['deep editor tree', '['.repeat(160) + 'x' + ']'.repeat(160)],
        ['large comment', '#' + 'x'.repeat(1 << 20)],
    ]) {
        const rawRoot = strict.parser.parse(source).topNode.name;
        const editorRoot = syntaxTree(make(source, strict)).topNode.name;
        assert.equal(rawRoot, 'Stream');
        assert.equal(editorRoot, 'Stream');
        emit('limit', {name, bytes: Buffer.byteLength(source), rawRoot, editorRoot});
    }

    const slot = new Compartment();
    const before = make(valid, slot.of(strict));
    const original = syntaxTree(before);
    assert.equal(syntaxTree(before.update({selection: {anchor: 1}}).state), original);
    const at = before.doc.toString().indexOf(']');
    const cut = {from: at, to: at + 1};
    const invalid = valid.replace(']', '');
    const proposed = valid.replace(']', '新');
    const invalidByte = projectedOffsets(invalid).byteAt;
    const validByte = projectedOffsets(valid).byteAt;
    const rejected = result(() => syntaxTree(before.update({changes: cut}).state), invalidByte);
    assert.equal(typeof rejected.offset, 'number');
    assert.equal(syntaxTree(before), original);
    const relaxed = before.update({changes: cut, effects: slot.reconfigure(recovery)}).state;
    assert.deepEqual(result(() => syntaxTree(relaxed.update({effects: slot.reconfigure(strict)}).state), invalidByte), rejected);
    const strictSwap = insert => relaxed.update({changes: {from: at, insert}, effects: slot.reconfigure(strict)}).state;
    const rejectedProposed = result(() => syntaxTree(strictSwap('新')), projectedOffsets(proposed).byteAt);
    assert.equal(typeof rejectedProposed.offset, 'number');
    const repaired = strictSwap(']');
    assert.equal(repaired.facet(language), strict);
    assert.equal(syntaxTree(before), original);
    emit('edit', {source: valid, removeAt: validByte(at), invalid, insert: '新', proposed, rejected,
        rejectedProposed, recovered: rows(syntaxTree(relaxed), invalidByte),
        repaired: rows(syntaxTree(repaired), validByte)});

    const distinct = named.configure({}, '');
    assert.notEqual(distinct.parser, named.parser);
    const installed = make(valid, slot.of(yaml()));
    const stockTree = syntaxTree(installed);
    assert.equal(syntaxTree(installed.update({effects: slot.reconfigure(catalog)}).state), stockTree);
    const renamed = installed.update({effects: slot.reconfigure(named)}).state;
    assert.notEqual(syntaxTree(renamed), stockTree);
    assert.notEqual(syntaxTree(renamed.update({effects: slot.reconfigure(distinct)}).state), syntaxTree(renamed));

    const entries = [named.data.of({probe: 'normal'}), Prec.high(stock.data.of({probe: 'high'}))];
    const sample = '{}';
    const primary = make(sample, [...entries, distinct, Prec.high(named)]);
    assert.equal(primary.facet(language), named);
    const data = (state, key) => state.languageDataAt(key, 0, 1);
    const single = make(sample, [...entries, yaml()]);
    assert.deepEqual(data(single, 'probe'), ['high', 'normal']);
    const repeated = make(sample, [...entries, yaml(), catalog, yaml()]);
    assert.deepEqual(data(repeated, 'probe'), data(single, 'probe'));
    assert.deepEqual(data(repeated, 'commentTokens'), data(single, 'commentTokens'));
    // Distinct roots register separate providers, all reading the primary root.
    assert.deepEqual(data(primary, 'probe'), ['high', 'normal', 'high', 'normal']);
    const other = make(sample, [strict, Prec.high(json())]);
    assert.equal(other.facet(language).name, 'json');
    assert(!stock.isActiveAt(other, 0));
    assert.deepEqual(stock.findRegions(other), []);
    const position = side => ({side, active: stock.isActiveAt(single, 0, side),
        data: single.languageDataAt('probe', 0, side)});
    assert.deepEqual([-1, 1].map(position), [-1, 1].map(side => ({side, active: true, data: ['high', 'normal']})));
    const [indent] = data(single, 'indentOnInput');
    const indentCases = ['', ']', '}', '  \t]', '\u00a0}', 'x]', '] ', '\u0085]'];
    emit('data', {stock: stock.name, named: named.name, primary: primary.facet(language).name,
        other: other.facet(language).name, probe: data(single, 'probe'),
        twoProviders: data(primary, 'probe'),
        comments: data(single, 'commentTokens').map(tokens => tokens.line),
        indent: indentCases.map(prefix => [prefix, indent.test(prefix)])});
} finally {
    rmSync(scratch, {recursive: true, force: true});
}
