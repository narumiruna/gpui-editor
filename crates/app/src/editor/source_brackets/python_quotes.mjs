// From the repository root, using the same archive layout as lezer_oracle.mjs:
// node crates/app/src/editor/source_brackets/python_quotes.mjs ARCHIVE_DIR [HEADLESS_CHROME] \
//   > crates/app/src/editor/source_brackets/python_quotes.jsonl
// Pass a Chrome executable to also replay the fixture through EditorView input.

const byteLength = text => new TextEncoder().encode(text).length;
const position = text => text.replaceAll('\r\n', '\n').length;
const snapshot = state => ({
    body: state.sliceDoc(),
    selection: [state.selection.main.anchor, state.selection.main.head]
        .map(pos => byteLength(state.sliceDoc(0, pos))),
});

async function api(load) {
    const {EditorState} = await load('@codemirror/state');
    const {EditorView, keymap} = await load('@codemirror/view');
    const {python} = await load('@codemirror/lang-python');
    const {forceParsing} = await load('@codemirror/language');
    const {closeBrackets, closeBracketsKeymap, insertBracket, deleteBracketPair} =
        await load('@codemirror/autocomplete');
    function create(doc, anchor, head) {
        const extensions = [python(), closeBrackets(), keymap.of(closeBracketsKeymap)];
        if (doc.includes('\r\n')) extensions.push(EditorState.lineSeparator.of('\r\n'));
        return EditorState.create({doc, selection: {anchor, head}, extensions});
    }
    return {EditorView, forceParsing, insertBracket, deleteBracketPair, create};
}

function generate(assert, {forceParsing, insertBracket, deleteBracketPair, create}) {
    return [
        ['inside a double literal', '"| "', '"', ['ordinary']],
        ['inside a single literal', "'| '", "'", ['ordinary']],
        ['inside raw string after a non-BMP scalar and CRLF', 'x = 0\r\nr"😀| "', '"', ['ordinary']],
        ['inside raw format string', 'rf" | "', '"', ['ordinary']],
        ['inside format replacement expression', 'f"{( | )}"', '"', ['pair']],
        ['inside triple literal', '"""\r\n😀| """', '"', ['ordinary']],
        ['other quote inside a literal', '" | "', "'", ['pair']],
        ['after an escape inside a literal', '"\\"| "', '"', ['ordinary']],
        ['before a parsed opener', '|"x"', '"', ['pair']],
        ['before a second adjacent literal', '"a"|"b"', '"', ['pair']],
        ['after a prefix before a parsed opener', 'r|"x"', '"', ['ordinary']],
        ['before an untracked closer', '"x|"', '"', ['ordinary']],
        ['at the end of a complete literal', '"x"|', '"', ['ordinary']],
        ['after whitespace outside a literal', '"x" |', '"', ['pair']],
        ['unterminated literal with a native string node', '"x |', '"', ['ordinary']],
        ['comment with a quote is not a literal', '# " |', '"', ['pair']],
        ['mixed-case prefix not in closeBrackets data', 'fR|', '"', ['ordinary']],
        ['reverse selection with UTF-8 offsets', 'x = |字|', '"', ['pair']],
        ['single quote pair, skip, triple pair, triple skip', 'x = |', "''''", ['pair', 'skip', 'pair', 'skip']],
        ['prefixed pair, skip, triple pair, triple skip', 'r|', '""""', ['pair', 'skip', 'pair', 'skip']],
        ['untracked empty pair Backspace', 'x = r"|"', '\b', ['delete pair']],
        ['tracked pair Backspace', 'x = |', '"\b', ['pair', 'delete pair']],
        ['triple Backspace only deletes adjacent quotes', '"""|"""', '\b', ['delete pair']],
    ].map(([name, marked, keys, actions]) => {
        const [left, middle, right] = marked.split('|');
        const doc = left + middle + (right ?? '');
        const view = {
            state: create(doc, position(right === undefined ? left : left + middle), position(left)),
            dispatch(spec) { this.state = this.state.update(spec).state; },
        };
        const before = snapshot(view.state);
        const after = [...keys].map((key, i) => {
            assert.ok(forceParsing(view, view.state.doc.length, 5000), name);
            const {state} = view;
            if (key === '\b') {
                assert.ok(deleteBracketPair({state, dispatch: tr => view.dispatch(tr)}), name);
                assert.equal(actions[i], 'delete pair');
            } else {
                const tr = insertBracket(state, key);
                assert.equal(tr ? (tr.newDoc.eq(state.doc) ? 'skip' : 'pair') : 'ordinary', actions[i], name);
                view.state = (tr ?? state.update(state.replaceSelection(key))).state;
            }
            return [key, snapshot(view.state)];
        });
        return {name, before, after};
    });
}

async function replay({EditorView, create, forceParsing}, cases) {
    const pause = () => new Promise(resolve => setTimeout(resolve, 40));
    for (const {name, before, after} of cases) {
        const fromByte = byte => position(new TextDecoder('utf-8', {fatal: true})
            .decode(new TextEncoder().encode(before.body).slice(0, byte)));
        const view = new EditorView({
            state: create(before.body, ...before.selection.map(fromByte)), parent: document.body,
        });
        try {
            view.focus();
            await pause();
            let trusted = false;
            view.contentDOM.addEventListener('input', event => { trusted = event.isTrusted; });
            for (const [key, expected] of after) {
                if (!forceParsing(view, view.state.doc.length, 5000)) throw new Error('Incomplete parse');
                if (key === '\b') {
                    const event = new KeyboardEvent('keydown', {key: 'Backspace', bubbles: true, cancelable: true});
                    if (view.contentDOM.dispatchEvent(event)) throw new Error('Backspace not handled');
                } else {
                    trusted = false;
                    if (!document.execCommand('insertText', false, key) || !trusted) {
                        throw new Error('Expected trusted insertText input');
                    }
                }
                await pause();
                const actual = snapshot(view.state);
                if (JSON.stringify(actual) !== JSON.stringify(expected)) {
                    throw new Error(`${name}: ${JSON.stringify(actual)} != ${JSON.stringify(expected)}`);
                }
            }
        } finally {
            view.destroy();
        }
    }
}

async function main() {
    const {default: assert} = await import('node:assert/strict');
    const {execFile, execFileSync} = await import('node:child_process');
    const {createHash} = await import('node:crypto');
    const {readFileSync, realpathSync} = await import('node:fs');
    const {createServer} = await import('node:http');
    const {findPackageJSON} = await import('node:module');
    const {basename, dirname, join, resolve} = await import('node:path');
    const {pathToFileURL} = await import('node:url');
    const {promisify} = await import('node:util');
    const root = resolve(process.argv[2]);
    const manifest = JSON.parse(readFileSync(new URL('../../../../../docs/codemirror-upstream.json', import.meta.url)));
    const pins = [...manifest.packages, ...manifest.referenceDependencies];
    const entries = {};
    const packages = new Map();
    for (const name of ['@codemirror/lang-python', '@codemirror/autocomplete', '@codemirror/language',
                        '@codemirror/state', '@codemirror/view', '@codemirror/streamparser',
                        '@lezer/python', '@lezer/common', '@lezer/lr', '@lezer/highlight']) {
        const pin = pins.find(pin => pin.name === name);
        const archive = join(root, name.replace('/', '__'), basename(pin.tarball));
        const digest = createHash('sha512').update(readFileSync(archive)).digest('base64');
        assert.equal(`sha512-${digest}`, pin.integrity, name);
        const dir = realpathSync(join(root, 'node_modules', name));
        assert.equal(dir, realpathSync(join(dirname(archive), 'package')));
        for (const file of ['package.json', 'dist/index.js']) {
            assert.deepEqual(readFileSync(join(dir, file)),
                execFileSync('tar', ['-xOf', archive, `package/${file}`]), `${name}/${file}`);
        }
        const pkg = JSON.parse(readFileSync(join(dir, 'package.json')));
        assert.equal(pkg.name, name);
        assert.equal(pkg.version, pin.version);
        assert.equal(pkg.exports.import ?? pkg.exports['.'].import, './dist/index.js');
        packages.set(name, pkg);
        entries[name] = join(dir, 'dist/index.js');
        console.error(`${name} ${pin.version}: verified SHA-512 and runtime files`);
    }
    for (const [name, pkg] of packages) {
        for (const dep of Object.keys(pkg.dependencies ?? {})) {
            const file = realpathSync(findPackageJSON(dep, pathToFileURL(entries[name])));
            assert.equal(file, realpathSync(join(root, 'node_modules', dep, 'package.json')));
            if (!entries[dep]) {
                // These dependencies have no pin in the migration manifest.
                entries[dep] = resolve(dirname(file), JSON.parse(readFileSync(file)).exports.import);
            }
        }
    }
    const cases = generate(assert, await api(name => import(pathToFileURL(entries[name]))));
    const chrome = process.argv[3];
    if (chrome) {
        const imports = Object.fromEntries(Object.keys(entries).map((name, i) => [name, `/module/${i}.js`]));
        const files = new Map(Object.entries(imports).map(([name, url]) => [url, entries[name]]));
        files.set('/oracle.mjs', new URL(import.meta.url));
        const server = createServer((request, response) => {
            if (request.url === '/') {
                response.setHeader('Content-Type', 'text/html; charset=utf-8');
                response.end(`<!doctype html><meta charset=utf-8><pre id="result">pending</pre>
                    <script type="importmap">${JSON.stringify({imports})}</script>
                    <script type="module" src="/oracle.mjs"></script>`);
            } else if (request.url === '/cases') {
                response.end(JSON.stringify(cases));
            } else if (files.has(request.url)) {
                response.setHeader('Content-Type', 'text/javascript; charset=utf-8');
                response.end(readFileSync(files.get(request.url)));
            } else {
                response.writeHead(404).end();
            }
        });
        await new Promise(done => server.listen(0, '127.0.0.1', done));
        try {
            const {stdout} = await promisify(execFile)(chrome, ['--headless', '--dump-dom',
                '--virtual-time-budget=15000', `http://127.0.0.1:${server.address().port}/`], {timeout: 30000});
            assert.match(stdout, /<pre id="result">ok<\/pre>/);
            console.error(`Headless Chrome: ${cases.length} cases passed`);
        } finally {
            server.close();
        }
    }
    for (const row of cases) console.log(JSON.stringify(row));
}

if (typeof document === 'undefined') {
    await main();
} else {
    const result = document.querySelector('#result');
    try {
        await replay(await api(name => import(name)), await (await fetch('/cases')).json());
        result.textContent = 'ok';
    } catch (error) {
        result.textContent = String(error);
    }
}
