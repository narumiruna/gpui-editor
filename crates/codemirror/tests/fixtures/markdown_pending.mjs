// Run with Node and a directory of SHA-512-pinned npm archives.
import assert from 'node:assert/strict';
import {mkdtempSync, realpathSync, rmSync} from 'node:fs';
import {tmpdir} from 'node:os';
import {join, resolve} from 'node:path';
import {projectedOffsets, projectInput} from './editor_line_offsets.mjs';
import {pinnedModules} from './pinned_modules.mjs';

assert.equal(process.argv.length, 3, 'provide the directory of local npm archives');
const scratch = mkdtempSync(join(realpathSync(tmpdir()), 'markdown-pending.'));
try {
    const {load} = pinnedModules(resolve(process.argv[2]), scratch);
    const {EditorState} = await load('@codemirror/state');
    const {Language, LanguageDescription, LanguageSupport, ensureSyntaxTree, syntaxTree} = await load('@codemirror/language');
    const {IterMode, NodeProp} = await load('@lezer/common');
    const {classHighlighter, highlightTree} = await load('@lezer/highlight');
    const {markdown} = await load('@codemirror/lang-markdown');
    const {jsonLanguage: json} = await load('@codemirror/lang-json');
    const {wastLanguage: wast} = await load('@codemirror/lang-wast');
    const source = '# 😀 猫\r\n\r\n> ```LaZy\t meta\r\n> {"😀":\r\n>   "雪"}\r\n> ```\r\n\r\n```unknown\r\n(module)\r\n```\r\n\r\n```\r\n(func)\r\n```\r\n\r\n    (nop)\r\n';
    const child = new LanguageSupport(json);
    const emit = row => console.log(JSON.stringify(row));
    function gate() {
        const attempts = [];
        const description = LanguageDescription.of({name: 'Deferred', alias: ['lazy'], load() {
            const attempt = Promise.withResolvers();
            attempts.push(attempt);
            return attempt.promise;
        }});
        return {description, attempts};
    }
    function install({description}, seen, fallback) {
        return [markdown({codeLanguages(name) {
            seen.push(name);
            return LanguageDescription.matchLanguageName([description], name);
        }, defaultCodeLanguage: fallback}), json.data.of({probe: 'json'}), wast.data.of({probe: 'fallback'})];
    }
    function fences(tree) {
        const found = [];
        tree.iterate({mode: IterMode.IgnoreMounts, enter(node) {
            if (node.name === 'FencedCode') found.push(node.node);
        }});
        return found;
    }
    const waiting = gate();
    const seen = [];
    const pending = EditorState.create({doc: source, extensions: install(waiting, seen, wast)});
    assert.deepEqual(seen, ['LaZy', 'unknown']);
    function record(step, state, raw, extra = {}) {
        const tree = syntaxTree(state), doc = state.doc.toString();
        const {byteAt} = projectedOffsets(raw);
        const bytes = ({from, to}) => [byteAt(from), byteAt(to)];
        const fence = fences(tree)[0];
        const mount = fence.tree.prop(NodeProp.mounted);
        const input = mount.overlay.map(({from, to}) => ({from: fence.from + from, to: fence.from + to}));
        const [{from}, {to}] = input;
        const point = label => doc.indexOf(label) + 2;
        const probes = [from, doc.indexOf('雪'), input[0].to, input[0].to + 1, to,
            point('(module)'), point('(func)'), point('(nop)')];
        const route = (pos, side) => {
            const [value] = state.languageDataAt('probe', pos, side);
            if (value) return value;
            assert.deepEqual(state.languageDataAt('commentTokens', pos, side),
                [{block: {open: '<!--', close: '-->'}}]);
            return 'markdown';
        };
        let bodyPainted = false;
        const childPaint = [], gapPaint = [];
        highlightTree(tree, classHighlighter, (start, end, cls) => {
            if (input.some(({from, to}) => start < to && from < end)) {
                bodyPainted = true;
                if (cls === 'tok-propertyName' || cls === 'tok-string')
                    childPaint.push([byteAt(start), byteAt(end)]);
            } else {
                gapPaint.push([byteAt(start), byteAt(end)]);
            }
        }, from, to);
        const mounted = step === 'ready' || step === 'refresh';
        assert.equal(mount.tree.type.isAnonymous, !mounted);
        assert.equal(bodyPainted, mounted);
        assert.equal(gapPaint.length, 1);
        emit({step, source: raw, attempts: waiting.attempts.length, segments: input.map(bytes),
            json: json.findRegions(state).map(bytes), fallback: wast.findRegions(state).map(bytes),
            probes: probes.map(pos => [byteAt(pos), route(pos, -1), route(pos, 1)]),
            bodyPainted, childPaint, gapPaint, ...extra});
    }
    const tree = syntaxTree(pending);
    record('pending', pending, source, {seen});
    const withoutFallback = EditorState.create({doc: source, extensions: install(waiting, [], undefined)});
    assert.equal(waiting.attempts.length, 1);
    record('without-fallback', withoutFallback, source);

    // A headless state has no view to consume its parser promise rejection.
    function observeRejection(state) {
        state.field(Language.state).context.scheduleOn?.catch(error => {
            assert.match(error.message, /controlled rejection/);
        });
    }
    observeRejection(pending);
    observeRejection(withoutFallback);
    const failed = waiting.description.load();
    waiting.attempts[0].reject(Error('controlled rejection'));
    await assert.rejects(failed, /controlled rejection/);
    assert.equal(waiting.description.support, undefined);
    assert.equal(syntaxTree(pending.update({}).state), tree);
    record('failed', pending, source);

    function edit(state, raw, from, insert) {
        const transaction = state.update({changes: {from, insert}});
        return [transaction.state, projectInput(raw, transaction), [projectedOffsets(raw).byteAt(from), insert]];
    }
    const [retry, retryRaw, retryEdit] = edit(pending, source, 2, '!');
    observeRejection(retry);
    assert.equal(waiting.attempts.length, 2);
    record('retry', retry, retryRaw, {edit: retryEdit});
    waiting.attempts[1].resolve(child);
    assert.equal(await waiting.description.load(), child);
    assert.equal(syntaxTree(pending.update({selection: {anchor: 1}}).state), tree);
    record('retained', pending, source);
    const [ready, readyRaw, readyEdit] = edit(retry, retryRaw, 2, '+');
    record('ready', ready, readyRaw, {edit: readyEdit});
    assert.deepEqual(json.findRegions(retry), []);

    const staged = ensureSyntaxTree(pending, pending.doc.length, 1000);
    assert.ok(staged && staged !== tree);
    assert.equal(syntaxTree(pending), tree);
    const refreshed = pending.update({}).state;
    assert.equal(syntaxTree(refreshed), staged);
    assert.equal(refreshed.doc, pending.doc);
    assert.ok(refreshed.selection.eq(pending.selection));
    record('refresh', refreshed, source);

    const empty = gate();
    const raw = '```lazy\r\n```';
    const emptyState = EditorState.create({doc: raw, extensions: install(empty, [], wast)});
    assert.equal(empty.attempts.length, 1);
    assert.equal(fences(syntaxTree(emptyState))[0].tree.prop(NodeProp.mounted), undefined);
    emit({step: 'empty', source: raw, attempts: empty.attempts.length,
        json: json.findRegions(emptyState), fallback: wast.findRegions(emptyState)});
} finally {
    rmSync(scratch, {recursive: true, force: true});
}
