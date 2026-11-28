// From the repository root, replay with the local SHA-512-pinned npm archives:
// node crates/codemirror/tests/fixtures/yaml_frontmatter_yaml.mjs /path/to/archives > crates/codemirror/tests/fixtures/yaml_frontmatter_yaml.jsonl
import assert from 'node:assert/strict';
import {mkdtempSync, realpathSync, rmSync} from 'node:fs';
import {tmpdir} from 'node:os';
import {join, resolve} from 'node:path';
import {projectedOffsets} from './editor_line_offsets.mjs';
import {pinnedModules} from './pinned_modules.mjs';

assert.equal(process.argv.length, 3, 'provide the directory of local npm archives');
const scratch = mkdtempSync(join(realpathSync(tmpdir()), 'yaml-frontmatter-yaml.'));
try {
    const {load, pins} = pinnedModules(resolve(process.argv[2]), scratch);
    assert.equal(pins.get('@codemirror/lang-yaml').version, '6.1.3');
    assert.equal(pins.get('@lezer/yaml').version, '1.0.4');
    const {yamlLanguage: stock, yamlFrontmatter} = await load('@codemirror/lang-yaml');
    const {jsonLanguage} = await load('@codemirror/lang-json');
    const {Compartment, EditorState, Prec} = await load('@codemirror/state');
    const {LanguageSupport, foldable, getIndentation, language, syntaxTree} = await load('@codemirror/language');
    const strict = stock.configure({strict: true}, 'strict body');
    assert.equal(stock.data, strict.data);
    const data = probe => stock.data.of({probe});
    const slot = new Compartment(), parent = new Compartment();
    const modes = {
        strict: yamlFrontmatter({content: new LanguageSupport(strict, data('shared'))}),
        recover: yamlFrontmatter({content: stock}),
        json: yamlFrontmatter({content: new LanguageSupport(jsonLanguage.configure({strict: true}), data('shared'))}),
    };
    const normalize = source => source.replace(/\r\n?/g, '\n');
    const emit = row => console.log(JSON.stringify(row));
    let source, state, active;

    function capture(step) {
        assert.equal(state.facet(language), modes[active].language);
        assert.equal(state.doc.toString(), normalize(source));
        const {byteAt} = projectedOffsets(source);
        const tree = syntaxTree(state), outer = tree.topNode;
        const body = outer.lastChild;
        const front = outer.getChild('Frontmatter')?.getChild('Stream');
        const range = node => [byteAt(node.from), byteAt(node.to)];
        const nodes = mount => {
            if (!mount || mount.name !== 'Stream') return null;
            const rows = [];
            let depth = 0;
            mount.cursor().iterate(node => { rows.push([depth++, node.name, ...range(node)]); }, () => depth--);
            return rows;
        };
        const groups = new Map();
        let pos = 0;
        const probe = at => {
            const entry = JSON.stringify([-1, 1].map(side => [
                stock.isActiveAt(state, at, side),
                state.languageDataAt('probe', at, side),
                state.languageDataAt('commentTokens', at, side).map(tokens => tokens.line),
                state.languageDataAt('indentOnInput', at, side).map(rule => ['  ]', '}', '] junk'].map(input => rule.test(input))),
            ]));
            if (!groups.has(entry)) groups.set(entry, []);
            groups.get(entry).push(byteAt(at));
        };
        probe(0);
        for (const char of state.doc.toString()) probe(pos += char.length);
        const lines = [];
        if (step.label === 'YAML body services') {
            for (let i = 1; i <= state.doc.lines; i++) {
                const {from, to} = state.doc.line(i);
                const fold = foldable(state, from, to);
                lines.push([...range({from, to}), getIndentation(state, from), fold && range(fold)]);
            }
        }
        const errors = [];
        tree.iterate({enter(node) { if (node.type.isError) errors.push(range(node)); }});
        emit({...step, source, body: range(body), frontNodes: nodes(front), bodyNodes: nodes(body), errors,
            regions: stock.findRegions(state).map(range),
            data: [...groups].map(([entry, positions]) => [JSON.parse(entry), positions]), lines});
    }

    function start(label, mode, text) {
        source = text;
        active = mode;
        state = EditorState.create({doc: source, extensions: [slot.of(modes[mode]), parent.of([])]});
        capture({label, mode, start: true});
    }

    function step(label, {replace, mode, override, selection, rejected} = {}) {
        let edit, changes, nextSource = source;
        if (replace) {
            const [before, insert] = replace;
            const from = source.indexOf(before), to = from + before.length;
            assert.ok(from >= 0, before);
            const prefix = source.slice(0, from), through = source.slice(0, to);
            edit = [Buffer.byteLength(prefix), Buffer.byteLength(through), insert];
            changes = {from: normalize(prefix).length, to: normalize(through).length, insert: normalize(insert)};
            nextSource = prefix + insert + source.slice(to);
        }
        const effects = mode ? slot.reconfigure(modes[mode]) : override ? parent.reconfigure(Prec.high(data('parent'))) : [];
        const update = () => state.update({changes, effects, selection: selection && {anchor: selection}}).state;
        if (rejected) {
            const previous = syntaxTree(state);
            let stop;
            assert.throws(update, error => {
                assert.ok(error instanceof SyntaxError);
                const match = /^No parse at (\d+)$/.exec(error.message);
                assert.ok(match, error.message);
                stop = projectedOffsets(nextSource).byteAt(Number(match[1]));
                return true;
            });
            assert.equal(state.doc.toString(), normalize(source));
            assert.equal(syntaxTree(state), previous);
            emit({label, edit, mode, selection, attempted: nextSource, stop});
            return;
        }
        state = update();
        active = mode ?? active;
        source = nextSource;
        capture({label, edit, mode, override, selection});
    }

    start('initial', 'strict', '---\r\nincomplete: [😀\r---\r\n名: [値]\r\n');
    step('selection only', {selection: 1});
    step('front edit shifts body', {replace: ['😀', '新😀']});
    step('reject bad body', {replace: [']', ''], selection: 2, rejected: true});
    step('allow bad body + relax', {replace: [']', ''], mode: 'recover'});
    step('reject strict reconfiguration alone', {mode: 'strict', rejected: true});
    step('reject invalid final body at shifted offset', {replace: ['値', '新しい値'], mode: 'strict', rejected: true});
    step('repair and install strict', {replace: ['値', '値]'], mode: 'strict'});
    step('delete closer to hide body', {replace: ['\r---\r\n', '\r']});
    step('high precedence parent data', {override: true});
    start('YAML body services', 'strict', '---\r\nx: 1\r---\r\nk: [\r\n  a,\r  b\n]\r\n');
    start('YAML front with JSON body', 'json', '---\r\nx: [😀\r---\r\n[1]\r\n');
    start('just a YAML body', 'strict', '名: [値]\r\n');
    start('two empty mounts', 'strict', '---\r\n---');
    start('only an empty body', 'strict', '');
} finally {
    rmSync(scratch, {recursive: true, force: true});
}
