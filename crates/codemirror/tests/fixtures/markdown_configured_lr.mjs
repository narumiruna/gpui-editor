// Run with Node and a directory of local SHA-512-pinned npm archives.
import assert from 'node:assert/strict';
import {mkdtempSync, realpathSync, rmSync} from 'node:fs';
import {tmpdir} from 'node:os';
import {join, resolve} from 'node:path';
import {projectedOffsets, projectInput} from './editor_line_offsets.mjs';
import {pinnedModules} from './pinned_modules.mjs';

assert.equal(process.argv.length, 3, 'provide the directory of local npm archives');
const scratch = mkdtempSync(join(realpathSync(tmpdir()), 'markdown-configured-lr.'));
try {
    const {load} = pinnedModules(resolve(process.argv[2]), scratch);
    const {EditorState, Compartment} = await load('@codemirror/state');
    const {syntaxTree} = await load('@codemirror/language');
    const {IterMode, NodeProp} = await load('@lezer/common');
    const {highlightTree, tagHighlighter, tags} = await load('@lezer/highlight');
    const {markdown} = await load('@codemirror/lang-markdown');
    const {jsonLanguage: json} = await load('@codemirror/lang-json');
    const {lezerLanguage: lezer} = await load('@codemirror/lang-lezer');
    const emit = row => console.log(JSON.stringify(row));
    const normalize = raw => raw.replace(/\r\n?/g, '\n');
    const mount = (info, first, second, suffix = '\r\n> ```') =>
        `前 😀\r\n\r\n> \`\`\`${info}\r\n> ${first}\r\n> ${second}${suffix}`;
    const install = (named, fallback, seen = []) => markdown({codeLanguages(name) {
        seen.push(name);
        return name === 'key' ? named : null;
    }, defaultCodeLanguage: fallback});
    const after = (source, action) => {
        try {
            return action();
        } catch (error) {
            const at = /^No parse at (\d+)$/.exec(error.message);
            if (!at) throw error;
            return projectedOffsets(source).byteAt(Number(at[1]));
        }
    };
    for (const [stock, other, first, second, rest, bad, partial] of [
        [json, lezer, '["😀",', '1]', '@top Main { "x" }', ']', '1'],
        [lezer, json, '// 😀', '@top Main { "x" }', '{"x":1}', ')', '@top Main { "x"'],
    ]) {
        const name = stock.name;
        const strict = stock.configure({strict: true}, name + '-strict');
        const loose = strict.configure({strict: false}, name + '-recovery');
        const fallback = other.configure({strict: true}, other.name + '-fallback');
        const source = mount('key extra', first, second)
            + `\r\n\r\n\`\`\`${loose.name}\r\n${rest}\r\n\`\`\`\r\n\r\n\`\`\`\r\n${rest}\r\n\`\`\`\r\n\r\n    ${rest}`;
        const seen = [];
        const state = EditorState.create({doc: source, extensions: [install(strict, fallback, seen),
            stock.data.of({probe: name}), other.data.of({probe: other.name})]});
        const {byteAt} = projectedOffsets(source);
        const choices = [];
        syntaxTree(state).iterate({mode: IterMode.IgnoreMounts, enter(node) {
            const child = node.tree?.prop(NodeProp.mounted);
            if (child) choices.push([strict, fallback].find(choice => choice.parser === child.parser)?.name);
        }});
        assert.deepEqual(choices, [strict.name, fallback.name, fallback.name, fallback.name]);
        const regions = parser => parser.findRegions(state).map(({from, to}) => [byteAt(from), byteAt(to)]);
        const at = normalize(source).indexOf('> ' + second);
        const probes = [at, at + 1, at + 2].map(pos => [byteAt(pos), ...[-1, 1].map(side => [
            strict.isActiveAt(state, pos, side), state.languageDataAt('probe', pos, side),
        ])]);
        emit({kind: 'resolve', name, source, seen, regions: [regions(stock), regions(other)], probes});
        for (const [caseName, input, select] of [
            ['named join', mount('key extra', first, bad), install(strict, loose)],
            ['unknown fallback', mount('unknown', first, bad), install(loose, strict)],
            ['bare fallback', mount('', first, bad), install(loose, strict)],
            ['closed EOF', mount('key', first, partial), install(strict, loose)],
            ['open EOF', mount('key', first, partial, ''), install(strict, loose)],
        ]) {
            const offset = after(input, () => EditorState.create({doc: input, extensions: select}));
            assert.equal(typeof offset, 'number');
            emit({kind: 'reject', name, caseName, source: input, offset});
        }
        const broken = mount('key extra', first, bad);
        const slot = new Compartment();
        const start = EditorState.create({doc: broken, extensions: slot.of(install(loose, loose))});
        const tree = syntaxTree(start);
        const strictEffect = slot.reconfigure(install(strict, loose));
        const rejected = after(broken, () => start.update({effects: strictEffect}).state);
        assert.equal(typeof rejected, 'number');
        assert.equal(syntaxTree(start), tree);
        const from = normalize(broken).lastIndexOf(bad);
        const changes = {from, to: from + bad.length, insert: second};
        const repair = start.update({changes, effects: strictEffect});
        const repaired = projectInput(broken, repair);
        const invalid = repair.state.update({changes: {from, to: from + second.length, insert: bad}});
        assert.equal(after(broken, () => invalid.state), rejected);
        const span = projectedOffsets(broken).byteAt;
        emit({kind: 'swap', name, broken, repaired, rejected, edit: [span(from), span(from + bad.length), second]});
    }
    const loneCR = '前 😀\r\r```lezer\r// 😀\r)\r```';
    assert.equal(lezer.configure({strict: true}).parser.parse('// 😀\r)').toString(), 'Grammar(LineComment)');
    const offset = after(loneCR, () => EditorState.create({doc: loneCR, extensions: markdown({defaultCodeLanguage: lezer.configure({strict: true})})}));
    assert.equal(typeof offset, 'number');
    emit({kind: 'projection', name: 'lezer', source: loneCR, offset});
    const quoted = '前 😀\r\n\r\n> ```\r\n> /* 😀\r\n> λ */\r\n> @top Main { "x" }\r\n> ```';
    const comments = [];
    const {byteAt} = projectedOffsets(quoted);
    const child = lezer.configure({strict: true});
    const dataSlot = new Compartment();
    const painted = EditorState.create({doc: quoted, extensions: [install(child, child),
        dataSlot.of(lezer.data.of({probe: 'before'}))]});
    highlightTree(syntaxTree(painted), tagHighlighter([{tag: tags.blockComment, class: 'comment'}]),
        (from, to) => comments.push([byteAt(from), byteAt(to)]));
    const at = painted.doc.toString().indexOf('λ');
    const switched = painted.update({effects: dataSlot.reconfigure(child.data.of({probe: 'after'}))}).state;
    assert.equal(syntaxTree(switched), syntaxTree(painted));
    emit({kind: 'highlight', name: 'lezer', source: quoted, at: byteAt(at), comments,
        before: painted.languageDataAt('probe', at), after: switched.languageDataAt('probe', at)});

    const empty = EditorState.create({doc: '```\r\n```', extensions: markdown({defaultCodeLanguage: json.configure({strict: true})})});
    assert.deepEqual(json.findRegions(empty), []);
    emit({kind: 'empty', name: 'json', source: '```\r\n```'});
} finally {
    rmSync(scratch, {recursive: true, force: true});
}
