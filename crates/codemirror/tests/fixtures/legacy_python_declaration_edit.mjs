// Run with Node and a directory of SHA-512-pinned npm archives.
import assert from 'node:assert/strict';
import {mkdtempSync, realpathSync, rmSync} from 'node:fs';
import {tmpdir} from 'node:os';
import {dirname, join, resolve} from 'node:path';
import {pathToFileURL} from 'node:url';
import {pinnedModules} from './pinned_modules.mjs';

assert.equal(process.argv.length, 3, 'usage: node legacy_python_declaration_edit.mjs <archive directory>');
const scratch = mkdtempSync(join(realpathSync(tmpdir()), 'gpui-python-declarations.'));
try {
    const {install, load} = pinnedModules(resolve(process.argv[2]), scratch);
    const python = join(dirname(install('@codemirror/legacy-modes').path), 'mode', 'python.js');
    const {mkPython} = await import(pathToFileURL(python));
    const {StringStream} = await load('@codemirror/streamparser');
    for (const [source, name] of [['# example\ndef f(x):\n  print x', 'x'], ['# example\nclass C(Base):\n  pass', 'Base']]) {
        const at = source.indexOf('(');
        const range = [at + 1, at + 1 + name.length];
        const insert = ' ';
        const changed = source.slice(0, at) + insert + source.slice(at + 1);
        const token = text => {
            assert.equal(Buffer.byteLength(text), text.length);
            const mode = mkPython({version: 2});
            const state = mode.startState();
            let offset = 0;
            for (const line of text.split('\n')) {
                const stream = new StringStream(line, 4, 2);
                while (!stream.eol()) {
                    stream.start = stream.pos;
                    const style = mode.token(stream, state);
                    assert(stream.pos > stream.start);
                    if (offset + stream.start === range[0]) {
                        assert.equal(offset + stream.pos, range[1]);
                        assert.equal(stream.current(), name);
                        return style;
                    }
                }
                offset += line.length + 1;
            }
            assert.fail(`no token for ${name}`);
        };
        console.log(JSON.stringify({source, at, name, range, insert, changed, before: token(source), after: token(changed)}));
    }
} finally {
    rmSync(scratch, {recursive: true, force: true});
}
