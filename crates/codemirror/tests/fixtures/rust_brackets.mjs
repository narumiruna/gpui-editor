// Replay with node and the directory of SHA-512-pinned archives.
import assert from 'node:assert/strict';
import {mkdtempSync, realpathSync, rmSync} from 'node:fs';
import {join, resolve} from 'node:path';
import {tmpdir} from 'node:os';
import {projectedOffsets} from './editor_line_offsets.mjs';
import {pinnedModules} from './pinned_modules.mjs';

const cases = [
    'fn main() { log(")"); }',
    'fn main() { log(r#")"#); }',
    'fn main() { log(/* ) */ 1); }',
    'fn f() { let c = \'}\'; let p = (vec![1, 2], ")"); }',
    'fn m() { dbg!(r#"[😀)"#, br#"]"#); }',
    'fn f() { let t = (b"[", b"]"); }',
    'macro_rules! make { ($e:expr) => { println!("}"); }; }',
    '#[cfg(any(unix, target_os = "(none)"))] fn f() {}',
    'fn f() { // 😀 (\r\n log("]"); // )\r\n}',
    'fn f() { // 😀 (\r log("]"); // )\r}',
    'fn f() { let s = "\\\r\n[😀]"; }',
    'fn f() { log(")"',
    'fn f() { log([1, 2); }',
    'fn f() { /* open 😀]\r\n',
];
const scratch = mkdtempSync(join(realpathSync(tmpdir()), 'gpui-rust-brackets.'));
try {
    const {load} = pinnedModules(resolve(process.argv[2]), scratch);
    const {rust} = await load('@codemirror/lang-rust');
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
    for (const source of cases) {
        const {units, byteAt} = projectedOffsets(source);
        const view = {state: EditorState.create({doc: source, extensions: [rust(), matching]}), dispatch(spec) {
            this.state = this.state.update(spec).state;
        }};
        assert(forceParsing(view, units, 5000));
        const range = ({from, to}) => [byteAt(from), byteAt(to)];
        const checks = [];
        let anchor = 0;
        for (const part of ['', ...source.match(/\r\n|[^]/gu)]) {
            anchor += part === '\r\n' ? 1 : part.length;
            rendered = null;
            plugin.create({state: view.state.update({selection: {anchor}}).state});
            checks.push([byteAt(anchor), rendered && [range(rendered.start), rendered.end ? range(rendered.end) : null, rendered.matched]]);
        }
        console.log(JSON.stringify({source, checks}));
    }
} finally {
    rmSync(scratch, {recursive: true, force: true});
}
