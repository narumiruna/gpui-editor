// Replay with node and the directory of SHA-512-pinned archives.
import assert from 'node:assert/strict';
import {mkdtempSync, readFileSync, realpathSync, rmSync} from 'node:fs';
import {resolve, join} from 'node:path';
import {tmpdir} from 'node:os';
import {pinnedModules} from './pinned_modules.mjs';

const cases = [
    'class A { void f(){ log(")"); } }',
    'class A { void f(){ log("[()]"); /* {>} */ } }',
    'class A { int[] a = {1, 2}; char c = \'}\'; }',
    'class A { Object o = new int[foo(1)]; }',
    'class A<T extends X<Z>> { void f(){} }',
    'class A { // 甲 (\r\n void f() { } // )\r\n}',
    'class A { String s = "(😀"; String t = ")"; }',
    'class A { String s = """\r\n  [😀]\r\n """; }',
    'class A { // (\r void f() { log("]"); }\r}',
    'class A { void f(){ log(1);',
    'class A { void f(){ log((1); } }',
    'class A { void f(){ log(1]; } }',
    'class A { void f(){ log([)]; } }',
    'class A { String s = "({[😀";',
    'class A { /* start ( 😀',
    'class A { @Ann(value=(1)) int a; }',
].map(source => ({source, top: 'Program'}));
cases.push({top: 'ClassContent', source: 'void f() { log("(😀)"); } // ]'});
// An optional Java parser audit JSONL supplies a larger corpus with original source and root.
const input = process.argv[3]
    ? readFileSync(process.argv[3], 'utf8').trim().split('\n').map(line => JSON.parse(line)).filter(row => row.source !== undefined)
    : cases;
const scratch = mkdtempSync(join(realpathSync(tmpdir()), 'gpui-java-brackets.'));
try {
    const {load} = pinnedModules(resolve(process.argv[2]), scratch);
    const {java, javaLanguage} = await load('@codemirror/lang-java');
    const classContent = javaLanguage.configure({top: 'ClassContent'});
    const {bracketMatching, forceParsing} = await load('@codemirror/language');
    const {EditorState} = await load('@codemirror/state');
    const {ViewPlugin} = await load('@codemirror/view');
    let rendered;
    const matching = bracketMatching({renderMatch(hit) {
        assert.equal(rendered, null);
        rendered = hit;
        return [];
    }});
    const plugin = matching.flat(Infinity).find(extension => extension instanceof ViewPlugin);
    assert(plugin);
    for (const {source, top} of input) {
        const offsets = new Map([[0, 0]]);
        let units = 0, bytes = 0;
        for (const [part] of source.matchAll(/\r\n|[^]/gu)) {
            units += part === '\r\n' ? 1 : part.length;
            bytes += Buffer.byteLength(part);
            offsets.set(units, bytes);
        }
        const language = top === 'ClassContent' ? classContent : java();
        const view = {state: EditorState.create({doc: source, extensions: [language, matching]}), dispatch(spec) {
            this.state = this.state.update(spec).state;
        }};
        assert(forceParsing(view, units, 5000));
        const span = ({from, to}) => [offsets.get(from), offsets.get(to)];
        const hits = [...offsets].map(([anchor, bytes]) => {
            rendered = null;
            plugin.create({state: view.state.update({selection: {anchor}}).state});
            return [bytes, rendered && [span(rendered.start), rendered.end ? span(rendered.end) : null, rendered.matched]];
        });
        console.log(JSON.stringify({source, top, hits}));
    }
} finally {
    rmSync(scratch, {recursive: true, force: true});
}
