// Run with Node and a directory of local SHA-512-pinned npm archives.
import assert from 'node:assert/strict';
import {mkdtempSync, realpathSync, rmSync} from 'node:fs';
import {tmpdir} from 'node:os';
import {join, resolve} from 'node:path';
import {projectedOffsets, projectInput} from './editor_line_offsets.mjs';
import {pinnedModules} from './pinned_modules.mjs';

assert.equal(process.argv.length, 3, 'provide the directory of local npm archives');
const scratch = mkdtempSync(join(realpathSync(tmpdir()), 'markdown-configured-rust.'));
try {
    const {load} = pinnedModules(resolve(process.argv[2]), scratch);
    const {Compartment, EditorState} = await load('@codemirror/state');
    const {foldable, foldInside, foldNodeProp, syntaxTree} = await load('@codemirror/language');
    const {markdown} = await load('@codemirror/lang-markdown');
    const {rustLanguage: stock} = await load('@codemirror/lang-rust');
    const source = '前 😀\r\n\r\n```rr opts\r\nfn λ() { \r\n  let t = "😀";\r\n  t\r\n}\r\n```\r\n\r\n> ```other\r\n> fn f() {\r\n>   1\r\n> }\r\n> ```';
    const calls = [];
    const configured = stock.configure({props: [foldNodeProp.add(row => row.id === 43 ? (node, state) => {
        const fold = foldInside(node);
        calls.push([node.name, node.type.id, node.from, node.to, state.selection.main.head]);
        return fold && {from: fold.from + Number(state.selection.main.head !== 0), to: fold.to};
    } : undefined)]}, 'rust-configured');
    const strict = configured.configure({strict: true}, 'rust-strict');
    const disabled = configured.configure({props: [foldNodeProp.add(row => row.id === 43 ? null : undefined)]}, 'rust-no-fold');
    const slot = new Compartment(), data = stock.data.of({probe: 'rust'});
    const install = (handle, fallback = stock) => markdown({codeLanguages(info) {
        assert.notEqual(info, 'rr opts');
        return info === 'rr' ? handle : null;
    }, defaultCodeLanguage: fallback});
    const initial = EditorState.create({doc: source, extensions: [slot.of(install(configured)), data]});
    const emit = fields => console.log(JSON.stringify(fields));
    const read = (step, raw, state) => {
        const {byteAt} = projectedOffsets(raw);
        const regions = stock.findRegions(state).map(({from,to}) => [byteAt(from),byteAt(to)]);
        const spots = [0, ...stock.findRegions(state).flatMap(({from, to}) => [from, from + 1, to])];
        const probes = [...new Set(spots)].map(at => [byteAt(at), ...[-1, 1].map(side => [
            configured.isActiveAt(state, at, side), state.languageDataAt('probe', at, side),
        ])]);
        const folds = [];
        for (let i = 1; i <= state.doc.lines; ++i) {
            const {from, to, text} = state.doc.line(i);
            if (!text.includes('fn ')) continue;
            const fold = foldable(state, from, to);
            folds.push([byteAt(from), byteAt(to), fold ? [byteAt(fold.from), byteAt(fold.to)] : null]);
        }
        return {step, raw, regions, probes, folds, calls: calls.splice(0).map(([name, term, from, to, head]) => [name, term, byteAt(from), byteAt(to), byteAt(head)])};
    };
    const select = initial.doc.toString().indexOf('let');
    const selected = initial.update({selection: {anchor: select}}).state;
    assert.equal(syntaxTree(initial), syntaxTree(selected));
    emit(read('initial', source, initial));
    emit({...read('selection', source, selected), select: projectedOffsets(source).byteAt(select)});
    const wide = initial.doc.toString().indexOf('😀', select);
    const changes = {from: wide, to: wide + 2, insert: '猫😀'};
    const edit = selected.update({changes});
    emit({...read('edit', projectInput(source, edit), edit.state), edit: [projectedOffsets(source).byteAt(wide), projectedOffsets(source).byteAt(wide + 2), changes.insert]});
    emit(read('disable', source, selected.update({effects: slot.reconfigure(install(disabled))}).state));
    emit(read('strict', source, selected.update({effects: slot.reconfigure(install(strict))}).state));
    emit(read('quoted', source, selected.update({effects: slot.reconfigure(install(configured, configured))}).state));
    const semicolon = initial.doc.toString().indexOf(';');
    const cut = selected.update({changes: {from: semicolon, to: semicolon + 1}});
    const broken = projectInput(source, cut);
    const replace = install(strict);
    const effect = slot.reconfigure(replace);
    let reject;
    try {
        cut.state.update({effects: effect}).state;
    } catch (error) {
        const match = /^No parse at (\d+)$/.exec(error.message);
        if (!match) throw error;
        reject = projectedOffsets(broken).byteAt(Number(match[1]));
    }
    assert(Number.isSafeInteger(reject));
    emit({step: 'reject', raw: broken, cut: projectedOffsets(source).byteAt(semicolon), reject});
    const repair = cut.state.update({effects: effect, changes: {from: semicolon, insert: ';'}});
    assert.equal(projectInput(broken, repair), source);
    emit(read('repair', source, repair.state));
    emit(read('retained', source, selected));
} finally {
    rmSync(scratch, {recursive: true, force: true});
}
