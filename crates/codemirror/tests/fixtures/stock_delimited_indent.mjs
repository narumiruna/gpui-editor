// Run with Node and a directory of SHA-512-pinned npm archives.
import assert from 'node:assert/strict';
import {mkdtempSync, realpathSync, rmSync} from 'node:fs';
import {tmpdir} from 'node:os';
import {join, resolve} from 'node:path';
import {pinnedModules} from './pinned_modules.mjs';
import {projectedOffsets} from './editor_line_offsets.mjs';

assert.equal(process.argv.length, 3, 'usage: node stock_delimited_indent.mjs <archive directory>');
const scratch = mkdtempSync(join(realpathSync(tmpdir()), 'gpui-stock-delimited-indent.'));
try {
    const {load} = pinnedModules(resolve(process.argv[2]), scratch);
    const {EditorState} = await load('@codemirror/state');
    const {getIndentation, IndentContext} = await load('@codemirror/language');
    const cases = {
        rust: ['fn f(§) {}', 'fn f(§  ) {}', 'fn f(  §) {}', 'fn f() {§}', 'fn f() { let x = [§]; }',
            'fn f() { foo( /*x*/ §); }', 'fn f() { foo(  x§); }', 'fn f() { foo(\r\n§  ); }', 'fn f(§',
            'fn f() { foo( /*x*/§', '// 前 😀\r\nfn café(  §) {}'],
        cpp: ['void f(§) {}', 'void f(§  ) {}', 'void f(  §) {}', 'void f() {§}', 'int a[] = {§};',
            'void f() { foo( /*x*/ §); }', 'void f() { foo(  x§); }', 'void f() { foo(\r\n§  ); }', 'void f(§',
            'void f() { foo( /*x*/§', '// 前 😀\r\nvoid café(  §) {}'],
        css: ['a {§}', 'a {§  }', 'a {  §}', 'a { color: foo(§); }', 'a { grid-template-columns: [§]; }',
            'a {§ /*x*/ }', 'a {  color:red;§}', 'a {\r\n§  }', 'a {§', 'a { /*x*/§', '/* 前 😀 */\r\n.café {  §}',
            'a { ?§}', 'a { )§}', 'a { )\n§}', 'a { )§', 'a { ?§',
            'a[§] {}', '[§] {}', 'a[  §] {}', 'a[ /*x*/ §] {}', 'a[title§] {}',
            '/*猫😀*/\r\n.猫😀[\r\n /*前😀*/\r\n §] {}'],
    };
    for (const [grammar, marked] of Object.entries(cases)) {
        const language = await load(`@codemirror/lang-${grammar}`);
        const extensions = language[grammar]();
        for (const text of marked) {
            const [left, right] = text.split('§');
            const source = left + right;
            const pos = projectedOffsets(left).units;
            const state = EditorState.create({doc: source, extensions});
            const columns = [{}, {simulateBreak: pos}, {simulateBreak: pos, simulateDoubleBreak: true}]
                .map(options => getIndentation(new IndentContext(state, options), pos) ?? null);
            console.log(JSON.stringify({grammar, source, at: projectedOffsets(source).byteAt(pos), columns}));
        }
    }
} finally {
    rmSync(scratch, {recursive: true, force: true});
}
