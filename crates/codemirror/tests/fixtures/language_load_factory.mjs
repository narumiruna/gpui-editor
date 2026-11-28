// node crates/codemirror/tests/fixtures/language_load_factory.mjs <archive-root>
import assert from 'node:assert/strict';
import {mkdtempSync, realpathSync, rmSync} from 'node:fs';
import {tmpdir} from 'node:os';
import {join, resolve} from 'node:path';
import {pinnedModules} from './pinned_modules.mjs';

const root = realpathSync(mkdtempSync(join(tmpdir(), 'language-load-factory-')));
try {
    const {load, pins} = pinnedModules(resolve(process.argv[2]), root);
    assert.equal(pins.get('@codemirror/language').version, '6.13.1');
    const {LanguageDescription} = await load('@codemirror/language');
    let starts = 0, finish;
    const initial = {name: 'initial'}, loaded = {name: 'ready'};
    const handle = LanguageDescription.of({name: 'Custom', support: initial, load() {
        if (++starts === 1) throw Error('failed synchronously');
        return new Promise(resolve => { finish = resolve; });
    }});
    const events = [['before', starts, handle.support.name]];
    assert.throws(() => handle.load(), /failed synchronously/);
    events.push(['failed', starts, handle.support.name]);
    const retry = handle.load();
    events.push(['requested', starts, handle.support.name]);
    assert.equal(handle.load(), retry);
    events.push(['joined', starts, handle.support.name]);
    finish(loaded);
    assert.equal(await retry, loaded);
    assert.equal(handle.load(), retry);
    events.push(['loaded', starts, handle.support.name]);
    console.log(JSON.stringify(events));
} finally {
    rmSync(root, {recursive: true, force: true});
}
