// Run with Node and a directory of local SHA-512-pinned npm archives.
import assert from 'node:assert/strict';
import {mkdtempSync, realpathSync, rmSync} from 'node:fs';
import {tmpdir} from 'node:os';
import {join, resolve} from 'node:path';
import {projectedOffsets, projectInput} from './editor_line_offsets.mjs';
import {pinnedModules} from './pinned_modules.mjs';

assert.equal(process.argv.length, 3, 'provide the directory of local npm archives');
const scratch = mkdtempSync(join(realpathSync(tmpdir()), 'lr-node-props.'));
try {
    const {load} = pinnedModules(resolve(process.argv[2]), scratch);
    const {Compartment, EditorState} = await load('@codemirror/state');
    const {defineLanguageFacet, languageDataProp, syntaxTree} = await load('@codemirror/language');
    const {markdown} = await load('@codemirror/lang-markdown');
    const {NodeProp} = await load('@lezer/common');
    const {jsonLanguage: json} = await load('@codemirror/lang-json');
    const {lezerLanguage: lezer} = await load('@codemirror/lang-lezer');
    const emit = row => console.log(JSON.stringify(row));
    const value = prop => prop == null ? null : prop;

    for (const [stock, other, source, target] of [
        [json, lezer, '{\r\n  "名😀": [0, true]\r\n}', 'Array'],
        [lezer, json, '// 😀\r\n@top Main { "名" }\r\n', 'Body'],
    ]) {
        const name = stock.name;
        const typeRow = type => [type.id, type.name, type.isTop, type.isSkipped, type.isError,
            type.isAnonymous, value(type.prop(NodeProp.group)), value(type.prop(NodeProp.openedBy)),
            value(type.prop(NodeProp.closedBy)), value(type.prop(NodeProp.isolate))];
        emit({kind: 'rows', name, types: stock.parser.nodeSet.types.map(typeRow)});

        const key = new NodeProp({combine: (old, next) => `${old}+${next}`});
        const seen = new NodeProp();
        const prior = stock.configure({props: [key.add(type => type.name === target ? 'seed' : undefined)]});
        const next = prior.configure({props: [
            key.add(type => type.name === target ? 'first' : undefined),
            key.add(type => type.name === target ? 'second' : undefined),
            seen.add(type => type.prop(key)),
        ]});
        const again = next.configure({strict: true}, 'renamed');
        assert.equal(next.data, again.data);
        assert.equal(next.parser.topNode, stock.parser.topNode);
        const types = next.parser.nodeSet.types;
        assert.deepEqual(types.map((type, index) => type === stock.parser.nodeSet.types[index]),
            types.map(type => type.name !== target));
        const visit = (tree, byteAt) => {
            const rows = [];
            tree.iterate({enter(node) {
                assert.equal(node.type, types[node.type.id]);
                rows.push([node.type.id, byteAt(node.from), byteAt(node.to), value(node.type.prop(key)),
                    value(node.type.prop(seen))]);
            }});
            return rows;
        };
        const raw = visit(next.parser.parse(source), pos => Buffer.byteLength(source.slice(0, pos)));
        const state = EditorState.create({doc: source, extensions: again});
        const editor = visit(syntaxTree(state), projectedOffsets(source).byteAt);
        assert(editor.some(row => row[3] === 'seed+first+second' && row[4] === 'seed'));
        const change = state.doc.toString().indexOf('名');
        const edit = state.update({changes: {from: change, to: change + 1, insert: '鏡'}});
        const edited = projectInput(source, edit);
        const after = visit(syntaxTree(edit.state), projectedOffsets(edited).byteAt);
        emit({kind: 'values', name, source, target, raw, editor, edited, after});

        const alternate = defineLanguageFacet({probe: 'default'});
        const cases = [['replacement', alternate], ['absent', null], ['cross', other.data]];
        for (const [caseName, facet] of cases) {
            const selected = stock.configure({props: [languageDataProp.add(type => type.isTop ? facet : undefined)]});
            assert.equal(selected.data, stock.data);
            assert.equal(selected.parser.topNode.prop(languageDataProp), facet);
            const slot = new Compartment();
            const entries = slot.of(alternate.of({probe: 'override'}));
            const data = [entries, stock.data.of({probe: 'stock'}), other.data.of({probe: 'other'})];
            const whole = EditorState.create({doc: source, extensions: [selected, ...data]});
            const mountedSource = `前 😀\r\n\r\n\`\`\`key\r\n${source}\r\n\`\`\``;
            const mount = markdown({codeLanguages: label => label === 'key' ? selected : null});
            const mounted = EditorState.create({doc: mountedSource, extensions: [mount, ...data]});
            const at = mounted.doc.toString().indexOf(source.replace(/\r\n?/g, '\n'));
            const describe = (state, raw, at) => {
                const {byteAt} = projectedOffsets(raw);
                const next = state.update({effects: slot.reconfigure([])}).state;
                assert.equal(syntaxTree(next), syntaxTree(state));
                return {at: byteAt(at), active: [stock, other].map(handle => [-1, 1].map(side => handle.isActiveAt(state, at, side))),
                    regions: [stock, other].map(handle => handle.findRegions(state).map(({from, to}) => [byteAt(from), byteAt(to)])),
                    probe: state.languageDataAt('probe', at), removed: next.languageDataAt('probe', at)};
            };
            emit({kind: 'facet', name, caseName, source, mountedSource,
                whole: describe(whole, source, 0), mount: describe(mounted, mountedSource, at)});
        }
    }
} finally {
    rmSync(scratch, {recursive: true, force: true});
}
