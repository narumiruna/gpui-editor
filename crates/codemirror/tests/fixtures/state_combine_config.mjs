// node crates/codemirror/tests/fixtures/state_combine_config.mjs <archive-root>
import {mkdtempSync, realpathSync, rmSync} from 'node:fs';
import {tmpdir} from 'node:os';
import {join, resolve} from 'node:path';
import {pinnedModules} from './pinned_modules.mjs';

const root = realpathSync(mkdtempSync(join(tmpdir(), 'state-config-combine-')));
try {
    const {load} = pinnedModules(resolve(process.argv[2]), root);
    const {combineConfig} = await load('@codemirror/state');
    for (const [name, inputs, defaults, rules] of [
        ['default only', [], [['size', 2], ['flag', true]], {}],
        ['undefined never overrides a defined value', [[['size', 2]], [['size'], ['flag']]], [['size', 3], ['flag', false]], {}],
        ['missing, undefined, and null differ', [[['flag'], ['mode', null]]], [['mode', 'fallback']], {}],
        ['equal inputs do not cause conflicts', [[['style', 'a']], [['style', 'a']]], [], {}],
        ['combine in input order', [[['x', 1], ['y', 1]], [['y', 2], ['x', 2]], [['x', 3], ['y', 3]]], [['x', 8]], {x: 'numbers', y: 'numbers'}],
        ['a rule can return undefined and allow defaults', [[['x', 1], ['y', 1]], [['x', 2], ['y', 2]], [['y', 3]]], [['x', 8]], {x: 'reset', y: 'reset'}],
        ['first conflict stops evaluation', [[['x', 1], ['b', 1], ['a', 1]], [['x', 3], ['b', 3], ['a', 3]]], [], {x: 'numbers'}],
    ]) {
        const calls = [];
        const merge = Object.fromEntries(Object.entries(rules).map(([key, rule]) => [key, (left, right) => {
            calls.push([key, left, right]);
            return rule === 'numbers' ? left * 10 + right : undefined;
        }]));
        let values = null, conflict = null;
        try {
            values = Object.entries(combineConfig(inputs.map(Object.fromEntries), Object.fromEntries(defaults), merge))
                .sort(([a], [b]) => a.localeCompare(b)).map(([key, value]) => value === undefined ? [key] : [key, value]);
        } catch (error) {
            conflict = error.message.replace('Config merge conflict for field ', '');
        }
        console.log(JSON.stringify({name, inputs, defaults, rules, calls, values, conflict}));
    }
} finally {
    rmSync(root, {recursive: true, force: true});
}
