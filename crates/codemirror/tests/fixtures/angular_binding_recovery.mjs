// From the repository root, generate angular_binding_recovery.jsonl with Node:
// node crates/codemirror/tests/fixtures/angular_binding_recovery.mjs [archive-root]
// Missing archives are downloaded and checked against docs/codemirror-upstream.json.
import assert from 'node:assert/strict';
import {mkdtempSync, realpathSync, rmSync} from 'node:fs';
import {tmpdir} from 'node:os';
import {join, resolve} from 'node:path';
import {pinnedModules} from './pinned_modules.mjs';

const cases = [
    ['missing bracket', '<comp [title="name" #ref="value">Hi {{who}}</comp>'],
    ['missing paren', "<comp (click='if (ready) {\n    run();\n}' [title]=\"name\">{{who}}</comp>"],
    ['missing inner paren', '<comp [(model]="state" [(other)]="next"></comp>'],
    ['missing outer bracket', '<comp [(model)="state" [(other)]="next"></comp>'],
    ['spaces and Unicode', '<comp [😺.label = "\n    item.name\n  ">{{who}}</comp>'],
    ['balanced control', '<comp [title]="name" #ref="value">Hi {{who}}</comp>'],
];

const root = mkdtempSync(join(realpathSync(tmpdir()), 'gpui-angular-binding-recovery.'));
try {
    const {load} = pinnedModules(resolve(process.argv[2] ?? join(root, 'archives')), root);
    const angularExports = await load('@codemirror/lang-angular');
    assert.deepEqual(Object.keys(angularExports), ['angular', 'angularLanguage']);
    const {angular} = angularExports;
    const {EditorState} = await load('@codemirror/state');
    const {foldable, getIndentation, indentUnit, syntaxTree} = await load('@codemirror/language');
    const {highlightTree, tags} = await load('@lezer/highlight');
    const tagNames = new Map();
    for (const [base, tag] of Object.entries(tags)) {
        if (typeof tag === 'function') continue;
        tagNames.set(tag, base);
        for (const [modifier, apply] of Object.entries(tags)) {
            if (typeof apply === 'function') tagNames.set(apply(tag), `${base}.${modifier}`);
        }
    }
    const highlighter = {style: list => list.map(tag => tagNames.get(tag)).filter(Boolean).join('+') || null};
    for (const [name, input] of cases) {
        const at = unit => Buffer.byteLength(input.slice(0, unit));
        const state = EditorState.create({doc: input, extensions: [angular(), indentUnit.of('  ')]});
        const tree = syntaxTree(state);
        const nodes = [], highlights = [], folds = [], indents = [];
        tree.iterate({enter: n => { nodes.push(`${n.name}:${at(n.from)}-${at(n.to)}`); }});
        highlightTree(tree, highlighter, (from, to, style) => highlights.push([at(from), at(to), style]));
        for (let i = 1; i <= state.doc.lines; i++) {
            const line = state.doc.line(i);
            const fold = foldable(state, line.from, line.to);
            folds.push(fold ? [at(fold.from), at(fold.to)] : null);
            indents.push(getIndentation(state, line.from));
        }
        console.log(JSON.stringify({name, input, nodes, highlights, folds, indents}));
    }
} finally {
    rmSync(root, {recursive: true, force: true});
}
