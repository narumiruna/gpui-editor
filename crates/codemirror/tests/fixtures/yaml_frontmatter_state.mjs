// Regenerate with Node 22.14+ from the repository root:
// node crates/codemirror/tests/fixtures/yaml_frontmatter_state.mjs > crates/codemirror/tests/fixtures/yaml_frontmatter_state.jsonl
import assert from 'node:assert/strict';
import {execFileSync} from 'node:child_process';
import {createHash} from 'node:crypto';
import {mkdirSync, mkdtempSync, readFileSync, realpathSync, rmSync} from 'node:fs';
import {findPackageJSON} from 'node:module';
import {tmpdir} from 'node:os';
import {join} from 'node:path';
import {pathToFileURL} from 'node:url';

const manifest = JSON.parse(readFileSync(new URL('../../../../docs/codemirror-upstream.json', import.meta.url)));
const pins = new Map([...manifest.packages, ...manifest.referenceDependencies].map(pin => [pin.name, pin]));
// Runtime dependencies of @codemirror/view not listed in the manifest.
for (const [name, version, hash] of [
    ['@marijn/find-cluster-break', '1.0.4', 'Wy0V7+SGUjnF9/TkiM1hKVDPj7jKXduPNboMVtHTA8dySMURWqfg/JZ9E2Sq8JgSJmkl7k7Qe9FLeMSrSraWmQ=='],
    ['crelt', '1.0.7', 'aK6BbWfhf4U/wCcLHKPJl/xa6VkVstRaPywWtMKGwuOLc/wZTyQYuoxgvZnNsBvv7Kg3YTBQYYBCggcviQczuA=='],
    ['style-mod', '4.1.5', 'WQ2llCZa9sZSkJA+aOwpg5DkfbAXFKGh//t3MRTBeruFiSBC+GXUmbBzrw3bXyboCWAbtCW9InDfBsXrRD/rAg=='],
    ['w3c-keyname', '2.2.8', 'dpojBhNsCNN7T82Tm7k26A6G9ML3NkhDsnw9n/eoxSRlVBB4CEtIQ/KTCLI2Fwf3ataSXRhYFkQi3SlnFwPvPQ=='],
]) {
    if (!pins.has(name)) pins.set(name, {
        name, version, integrity: `sha512-${hash}`,
        tarball: `https://registry.npmjs.org/${name}/-/${name.split('/').at(-1)}-${version}.tgz`,
    });
}

const root = realpathSync(mkdtempSync(join(tmpdir(), 'yaml-frontmatter-state-')));
const installed = new Map();
function install(name) {
    if (installed.has(name)) return installed.get(name);
    const pin = pins.get(name);
    assert.ok(pin, `missing pin for ${name}`);
    const dir = join(root, 'node_modules', name);
    const archive = join(root, `${name.replace('/', '-')}.tgz`);
    execFileSync('curl', ['-fsSL', '--retry', '2', '-o', archive, pin.tarball]);
    const sha512 = createHash('sha512').update(readFileSync(archive)).digest('base64');
    assert.equal(`sha512-${sha512}`, pin.integrity, name);
    mkdirSync(dir, {recursive: true});
    execFileSync('tar', ['-xf', archive, '--strip-components=1', '-C', dir]);
    const file = join(dir, 'package.json');
    const pkg = JSON.parse(readFileSync(file));
    assert.equal(pkg.name, name);
    assert.equal(pkg.version, pin.version);
    installed.set(name, {dir, file, pkg});
    for (const dependency of Object.keys(pkg.dependencies ?? {})) {
        const dep = install(dependency);
        assert.equal(realpathSync(findPackageJSON(dependency, pathToFileURL(file))), dep.file);
    }
    console.error(`${name} ${pkg.version}: ${pin.integrity}`);
    return installed.get(name);
}

function load(name) {
    const {dir, pkg} = install(name);
    const entry = (pkg.exports['.'] ?? pkg.exports).import;
    return import(pathToFileURL(join(dir, entry)));
}

try {
    const {yamlFrontmatter} = await load('@codemirror/lang-yaml');
    const {html} = await load('@codemirror/lang-html');
    const {jsonLanguage} = await load('@codemirror/lang-json');
    const {EditorState, Compartment, Facet, StateField} = await load('@codemirror/state');
    const {LanguageSupport, syntaxTreeAvailable} = await load('@codemirror/language');

    const label = Facet.define();
    const editCount = StateField.define({create: () => 0, update: (value, tr) => value + Number(tr.docChanged)});
    const content = html();
    const support = new LanguageSupport(content.language, [content.support, label.of('forwarded'), editCount]);
    const wrapped = yamlFrontmatter({content: support});
    const bare = yamlFrontmatter({content: jsonLanguage});
    assert.equal(wrapped.support, support.support);
    assert.deepEqual(bare.support, []);
    const compartment = new Compartment();

    const normalize = text => text.replace(/\r\n?/g, '\n');
    const byteAt = (text, at) => Buffer.byteLength(text.slice(0, at));
    const editorAt = (text, at) => normalize(text.slice(0, at)).length;
    function metadata(state, position, side) {
        const at = key => state.languageDataAt(key, position, side);
        return [at('commentTokens').map(({line, block}) => [line ?? null, block ? [block.open, block.close] : null]),
            at('closeBrackets').map(({brackets}) => brackets ?? null),
            at('wordChars'), at('indentOnInput').length];
    }
    function replay(source, config, steps = []) {
        const configs = {html: wrapped, json: bare, off: []};
        let state = EditorState.create({doc: source, extensions: compartment.of(configs[config])});
        function emit(step) {
            assert.equal(state.doc.toString(), normalize(source));
            if (config !== 'off') assert.ok(syntaxTreeAvailable(state, state.doc.length));
            const data = new Map();
            for (let at = 0; at <= source.length;) {
                const pos = editorAt(source, at);
                const pair = [-1, 1].map(side => metadata(state, pos, side));
                if (config === 'html' && source.includes('<script>') &&
                    (at === source.indexOf('<script>') + '<script>'.length || at === source.indexOf('</script>'))) {
                    // Lezer enters this bracketed nested mount from both sides.
                    // Native HTML retains its own side rules; compare the wrapper edges instead.
                    assert.deepEqual(pair.map(value => value[2]), [['$'], ['$']]);
                } else {
                    const key = JSON.stringify(pair);
                    if (!data.has(key)) data.set(key, [...pair, []]);
                    data.get(key)[2].push(byteAt(source, at));
                }
                if (at === source.length) break;
                at += source.startsWith('\r\n', at) || source.codePointAt(at) > 0xffff ? 2 : 1;
            }
            console.log(JSON.stringify({...step, source, label: state.facet(label),
                editCount: state.field(editCount, false) ?? null, data: [...data.values()]}));
        }
        emit({start: config});
        for (const step of steps) {
            if (step.selection) {
                const at = source.indexOf(step.selection);
                assert.ok(at >= 0);
                state = state.update({selection: {anchor: editorAt(source, at)}}).state;
                emit({selection: byteAt(source, at)});
                continue;
            }
            let changes, edit;
            if (step.edit) {
                const [before, insert] = step.edit;
                const from = before === null ? source.length : source.indexOf(before);
                assert.ok(from >= 0, before);
                const to = from + (before?.length ?? 0);
                changes = {from: editorAt(source, from), to: editorAt(source, to), insert: normalize(insert)};
                edit = [byteAt(source, from), byteAt(source, to), insert];
                source = source.slice(0, from) + insert + source.slice(to);
            }
            if (step.config) config = step.config;
            state = state.update({changes, effects: step.config ? compartment.reconfigure(configs[config]) : []}).state;
            emit({edit, config: step.config});
        }
    }

    replay('---\r\ntitle: 😀\r\n---\r\n<script>/* probe */</script>\r\n', 'html', [
        {selection: 'probe'},
        {edit: ['title: 😀', 'title: [新文']},
        {edit: ['<script>', '<script type="text/plain">']},
        {edit: ['\r\n---\r\n<script', '\r\n<script']},
        {edit: ['\r\n<script', '\r\n---\r\n<script']},
        {edit: ['---\r\ntitle', '---x\r\ntitle']},
        {edit: ['---x\r\ntitle', '---\r\ntitle']},
        {config: 'json', edit: ['<script type="text/plain">/* probe */</script>', '{"body": true}']},
        {config: 'off'},
        {config: 'html'},
    ]);
    replay('', 'json', [
        {edit: [null, '---\n']},
        {edit: [null, '---']},
        {edit: [null, '\n']},
        {edit: [null, '[]']},
        {edit: ['\n---\n', '\n\n---\n']},
    ]);
} finally {
    rmSync(root, {recursive: true, force: true});
}
