// node crates/codemirror/tests/fixtures/language_data_php.mjs <archive-root> [--emit]
import assert from 'node:assert/strict';
import {mkdtempSync, readFileSync, realpathSync, rmSync} from 'node:fs';
import {tmpdir} from 'node:os';
import {join, resolve} from 'node:path';
import {projectedOffsets, projectInput} from './editor_line_offsets.mjs';
import {pinnedModules} from './pinned_modules.mjs';

assert(process.argv.length === 3 || process.argv.length === 4 && process.argv[3] === '--emit');
const root = mkdtempSync(join(realpathSync(tmpdir()), 'language-data-php-'));
try {
    const {load, pins, installed} = pinnedModules(resolve(process.argv[2]), root);
    const {languages} = await load('@codemirror/language-data');
    const {php, phpLanguage} = await load('@codemirror/lang-php');
    const {html, htmlLanguage, autoCloseTags} = await load('@codemirror/lang-html');
    const {javascriptLanguage, localCompletionSource} = await load('@codemirror/lang-javascript');
    const {cssLanguage, cssCompletionSource} = await load('@codemirror/lang-css');
    const {CompletionContext} = await load('@codemirror/autocomplete');
    const {Compartment, EditorSelection, EditorState, Prec, StateEffect, Transaction} = await load('@codemirror/state');
    const {EditorView} = await load('@codemirror/view');
    const {language, languageDataProp, syntaxTree} = await load('@codemirror/language');
    const {IterMode, NodeProp} = await load('@lezer/common');
    const bundles = new Map();
    for (const name of ['PHP', 'HTML', 'JavaScript', 'CSS']) {
        const entry = languages.find(entry => entry.name === name);
        const bundle = await entry.load();
        assert.equal(await entry.load(), bundle);
        bundles.set(name, bundle);
    }
    const stock = bundles.get('PHP'), stockHtml = bundles.get('HTML');
    const [[schema, handler, [fixed, local, jsx], css]] = stock.support;
    assert.equal(stock.language.name, 'php');
    assert.notEqual(stock.language, phpLanguage);
    assert.equal(stock.language.data, phpLanguage.data);
    assert.deepEqual(stock.extension, [stock.language, stock.support]);
    assert.equal(stock.language.parser.topNode.name, 'Template');
    assert.equal(stock.language.parser.topNode.prop(languageDataProp), phpLanguage.data);
    assert.equal(handler, autoCloseTags);
    assert.deepEqual(jsx, []);
    assert.equal(schema.facet, htmlLanguage.data);
    assert.equal(fixed.facet, javascriptLanguage.data);
    assert.equal(local.facet, javascriptLanguage.data);
    assert.equal(css.facet, cssLanguage.data);
    const get = input => input.value.autocomplete;
    const [htmlSchema, htmlHandler, [htmlFixed, htmlLocal], htmlCss] = stockHtml.support;
    const [jsFixed, jsLocal] = bundles.get('JavaScript').support;
    const standaloneCss = bundles.get('CSS').support;
    assert.equal(htmlHandler, handler);
    assert.notEqual(get(schema), get(htmlSchema));
    assert.notEqual(get(fixed), get(htmlFixed));
    assert.notEqual(get(fixed), get(jsFixed));
    assert.notEqual(htmlLocal, local);
    assert.notEqual(jsLocal, local);
    assert.notEqual(htmlCss, css);
    assert.notEqual(standaloneCss, css);
    assert([local, htmlLocal, jsLocal].every(input => get(input) === localCompletionSource));
    assert([css, htmlCss, standaloneCss].every(input => get(input) === cssCompletionSource));
    const ids = new Map([[get(schema), 'php-html'], [get(htmlSchema), 'html'], [get(fixed), 'php-fixed'],
        [get(htmlFixed), 'html-fixed'], [get(jsFixed), 'js-fixed'], [localCompletionSource, 'local'],
        [cssCompletionSource, 'css']]);

    // Compare bare and borrowed parsers without making them catalog factories.
    const mounted = tree => tree.prop(NodeProp.mounted);
    const create = (doc, extensions = stock.extension) => EditorState.create({doc, extensions});
    const probe = '<x-box/>\r\n<main></wrong></main>';
    const base = html({matchClosingTags: true, selfClosingTags: true});
    const compactHtml = html({matchClosingTags: false});
    const borrowed = php({baseLanguage: base.language});
    const borrowedNoMatch = php({baseLanguage: compactHtml.language});
    const noBase = php({baseLanguage: null});
    const plain = php({plain: true});
    const stockOverlay = mounted(syntaxTree(create(probe)));
    assert.equal(stockOverlay.tree.type.prop(languageDataProp), htmlLanguage.data);
    assert.equal(mounted(syntaxTree(create(probe, borrowed.extension))).parser, base.language.parser);
    assert.equal(mounted(syntaxTree(create(probe, borrowedNoMatch.extension))).parser, compactHtml.language.parser);
    assert.equal(mounted(syntaxTree(create(probe, noBase.extension))), undefined);
    assert.equal(mounted(syntaxTree(create('echo $x;', plain.extension))), undefined);
    assert.equal(plain.language.parser.topNode.prop(languageDataProp), phpLanguage.data);
    assert.deepEqual(borrowed.support, []);
    assert.deepEqual(borrowedNoMatch.support, []);
    assert.deepEqual(noBase.support, []);
    assert.equal(plain.support[0][1], handler);
    const compact = [compactHtml.language, stock.support]; // Same inputs, to isolate the effect of gaps.
    const extensions = new Map([...bundles].map(([name, bundle]) => [name, bundle.extension]));
    for (const [name, extension] of [
        ['bare PHP', stock.language.extension], ['borrowed', borrowed.extension],
        ['borrowed noMatch', borrowedNoMatch.extension], ['no base', noBase.extension], ['plain', plain.extension],
        ['high JavaScript', Prec.high(bundles.get('JavaScript').extension)],
    ]) extensions.set(name, extension);

    const keys = ['zIndex', 'display', 'backgroundColor', 'cssFloat', 'cssText'];
    const triggers = ['  case ', 'default:', 'endif', 'endforeach', 'elseif', '}', '  } ', '</div>', 'case', 'else x'];
    globalThis.document = {body: {style: Object.fromEntries(keys.map(key => [key, '']))}};
    const rows = [{kind: 'pins', verified: installed.size, pins: [...installed.keys()].sort().map(name => {
        const {version, integrity} = pins.get(name);
        return [name, version, integrity];
    }), keys, triggers}];
    const languagesAt = [['PHP', phpLanguage], ['HTML', htmlLanguage], ['JS', javascriptLanguage], ['CSS', cssLanguage]];
    function data(state, at, side) {
        const active = languagesAt.filter(([, lang]) => lang.isActiveAt(state, at, side)).map(([name]) => name);
        assert(active.length <= 1);
        const sources = state.languageDataAt('autocomplete', at, side).map(fn => {
            assert(ids.has(fn));
            return ids.get(fn);
        });
        const comments = state.languageDataAt('commentTokens', at, side).map(({line, block}) =>
            [line ?? null, block ? [block.open, block.close] : null]);
        const brackets = state.languageDataAt('closeBrackets', at, side).map(({brackets, before, stringPrefixes}) =>
            [brackets ?? null, before ?? null, stringPrefixes ?? []]);
        const indent = state.languageDataAt('indentOnInput', at, side).map(regex => triggers.map(t => regex.test(t)));
        return {active: active[0] ?? null, sources, comments, brackets, words: state.languageDataAt('wordChars', at, side), indent};
    }
    function summarize(result, byteAt) {
        if (!result) return null;
        let hash = 2166136261;
        for (const {label, type, detail, boost, apply, ...rest} of result.options) {
            assert.deepEqual(rest, {});
            for (const byte of Buffer.from(`${label}\0${type ?? ''}\0${detail ?? ''}\0${boost ?? 0}\0${typeof apply === 'function' ? '@snippet' : apply ?? ''}\n`))
                hash = Math.imul(hash ^ byte, 16777619) >>> 0;
        }
        const splitNames = result.options.filter(({label, apply}) => label.includes('<?') ||
            typeof apply === 'string' && apply.includes('<?')).map(({label, apply}) => [label, apply ?? null]);
        return {range: [byteAt(result.from), result.to == null ? null : byteAt(result.to)],
            filter: result.filter ?? true, count: result.options.length, hash, splitNames};
    }
    function results(state, at, byteAt) {
        return [false, true].map(explicit => state.languageDataAt('autocomplete', at, -1).map(fn => {
            // Reset the Lezer resolve cursor before each source as in language_data_html.mjs.
            syntaxTree(state).resolveInner(0, -1);
            return summarize(fn(new CompletionContext(state, at, explicit)), byteAt);
        }));
    }
    function locate(text, word, delta = 0) {
        const at = text.indexOf(word);
        assert(at >= 0, `missing ${word}`);
        return at + delta;
    }
    const ends = (label, text, word) => [0, 1, word.length].map((delta, i) =>
        [`${label} ${['start', 'inside', 'end'][i]}`, locate(text, word, delta)]);
    function runState(name, original, points, steps = [['install', ['PHP']]]) {
        const slot = new Compartment();
        let state = create(original, slot.of([]));
        rows.push({kind: 'start', name, source: original});
        for (const [step, use, edit, select] of steps) {
            const before = syntaxTree(state), {byteAt: oldByteAt} = projectedOffsets(original);
            const text = state.doc.toString();
            const from = edit && locate(text, edit[0]), to = edit && from + edit[0].length;
            const tr = state.update({changes: edit && {from, to, insert: edit[1]},
                selection: select == null ? undefined : {anchor: select},
                effects: use && slot.reconfigure(use.map(name => extensions.get(name)))});
            original = projectInput(original, tr);
            state = tr.state;
            const tree = syntaxTree(state), {byteAt, units} = projectedOffsets(original), top = state.facet(language);
            if (top) assert.equal(tree.length, units);
            const outer = top?.name === 'php' && mounted(tree);
            const dialect = [];
            outer?.tree?.iterate({mode: IterMode.IgnoreMounts, enter(node) {
                if (['SelfClosingTag', 'MismatchedCloseTag'].includes(node.name)) dialect.push(node.name);
            }});
            const handlers = state.facet(EditorView.inputHandler);
            assert(handlers.every(fn => fn === handler.value));
            const queries = points(state.doc.toString()).map(([label, at]) => ({label, at: byteAt(at),
                data: [-1, 1].map(side => data(state, at, side)), results: results(state, at, byteAt)}));
            rows.push({kind: 'state', name, step, source: original, active: top?.name ?? null,
                root: tree.topNode.name, reused: !!top && before === tree, handlers: handlers.length,
                overlay: outer ? outer.overlay.map(({from, to}) => [byteAt(from), byteAt(to)]) : [], dialect,
                extensions: use, edit: edit && [oldByteAt(from), oldByteAt(to), edit[1]],
                select: select == null ? undefined : oldByteAt(select), head: byteAt(state.selection.main.head), queries});
        }
    }
    runState('catalog composition', ["<?php /* é😀 */ $php = b'x'; ?>", '<div class="name">',
        '<script lang="ts">const local: number = 1; loc;</script>',
        '<style>.card { bac }</style>', '</div>'].join('\r\n'), text => [
        ['PHP variable', locate(text, '$php', 2)], ['schema', locate(text, 'class', 3)],
        ['typed script locals', locate(text, 'loc;', 3)], ['CSS property', locate(text, 'bac', 3)],
    ], [
        ['install', ['PHP']], ['selection only', undefined, undefined, 3], ['repeat cached bundle', ['PHP', 'PHP']],
        ['edit PHP prefix', undefined, ['é😀', '🧪長文']], ['edit child scope', undefined, ['local:', 'other:']],
        ['HTML and children after PHP', ['PHP', 'HTML', 'JavaScript', 'CSS']],
        ['other providers first', ['HTML', 'JavaScript', 'CSS', 'PHP']],
        ['high JavaScript wins', ['PHP', 'high JavaScript', 'HTML']], ['detach', []], ['restore', ['PHP']],
    ]);
    runState('factory boundaries', probe + '\r\n<script>co</script>', text => [
        ['BOF', 0], ['host', locate(text, 'main', 2)], ...ends('script', text, 'co'), ['EOF', text.length],
    ], ['PHP', 'bare PHP', 'borrowed', 'borrowed noMatch', 'no base', 'PHP'].map(name => [name, [name]]));
    runState('route switches with the same compact text', "<?= 'é😀' ?>\r\n<script type=\"text/javascript\">const value = 1; va;</script>",
        text => [['script callable', locate(text, 'va;', 2)]],
        [['install', ['PHP']], ['split MIME', undefined, ['javascript', 'java<?= $mode ?>script']],
            ['restore MIME', undefined, ['java<?= $mode ?>script', 'javascript']]]);
    runState('plain factory', "/* 字😀 */\r\necho $name;", text => [['plain PHP', locate(text, '$name', 2)]], [['plain', ['plain']]]);
    runState('stock PHP without Text', "<?php /* é😀 */\r\n$only = B'x';",
        text => [['BOF', 0], ['PHP variable', locate(text, '$only', 2)], ['EOF', text.length]]);
    for (const [name, htmlSource, word] of [
        ['script body', '<script>co</script>', 'co'], ['style body', '<style>.x{bac}</style>', '.x{bac}'],
        ['quoted style', '<p style="bac"></p>', 'bac'], ['unquoted style', '<p style=bac></p>', 'bac'],
        ['quoted event', '<p onclick="return co"></p>', 'return co'], ['unquoted event', '<p onclick=co></p>', 'co'],
        ['JSX', '<script type="text/jsx">const x = <Foo/>;</script>', 'Foo'],
        ['TSX', '<script type="text/typescript-jsx">const x = <Foo/>;</script>', 'Foo'],
        ['import map', '<script type="importmap">{"imports":{}}</script>', 'imports'],
        ['unknown script type', '<script type="text/plain">words</script>', 'words'],
        ['empty style', '<p style=""></p>', '""'], ['empty script', '<script></script>', '><'],
        ['open script EOF', '<script>co', 'co'], ['missing style quote EOF', '<p style="co', 'co'],
    ]) runState(name, `<?= 'é😀' ?>\r\n${htmlSource}`, text => ends(name, text, word));

    // Compare the real overlay with the same noMatch HTML parser on concatenated
    // Text. Compact positions are mapped only within their selected segment.
    const plainState = create('echo $x;', plain.language);
    const phpData = data(plainState, 1, -1);
    function textSegments(state) {
        const {overlay} = mounted(syntaxTree(state));
        let length = 0;
        return overlay.map(({from, to}) => {
            const start = length;
            length += to - from;
            return {from, to, start, end: length};
        });
    }
    function gapState(name, source, extra) {
        const state = create(source), text = state.doc.toString(), {byteAt} = projectedOffsets(source);
        const segments = textSegments(state);
        const shortText = segments.map(s => text.slice(s.from, s.to)).join('');
        const short = create(shortText, compact);
        const bytes = Buffer.from(source);
        const rawText = segments.map(s => bytes.subarray(byteAt(s.from), byteAt(s.to)).toString()).join('');
        const questions = segments.slice(1).flatMap((segment, i) => {
            const start = segments[i].to, end = segment.from;
            assert(start < end);
            return [['before gap', start - 1], ['gap start', start], ['inside gap', start + 1],
                ['gap end', end], ['after gap', end + 1]];
        }).concat(extra(text));
        const segmentAt = (at, side) => segments.find(s => side < 0 ? s.from < at && at <= s.to : s.from <= at && at < s.to);
        rows.push({kind: 'gap', name, source, text: shortText,
            rawText, logicalLines: [short.doc.lines, create(rawText, []).doc.lines],
            segments: segments.map(s => [byteAt(s.from), byteAt(s.to)]),
            queries: questions.map(([label, at]) => {
                const packedData = [-1, 1].map(side => {
                    const s = segmentAt(at, side);
                    return s ? data(short, s.start + at - s.from, side) : phpData;
                });
                const left = segmentAt(at, -1);
                const packed = left ? results(short, left.start + at - left.from, pos => pos).map(mode => mode.map(result => {
                    if (!result) return null;
                    const map = n => n == null ? null : left.start <= n && n <= left.end ? byteAt(left.from + n - left.start) : null;
                    const {range: compactUnits, ...summary} = result;
                    const [from, to] = compactUnits;
                    return {...summary, compactUnits, sourceRange: [map(from), map(to)],
                        blocked: map(from) == null || to != null && map(to) == null};
                })) : [[], []];
                const pin = results(state, at, byteAt).map(mode => mode.map(result => {
                    if (!result) return null;
                    const [from, to] = result.range;
                    return {...result, crossesGap: !segments.some(s => byteAt(s.from) <= from &&
                        (to ?? byteAt(at)) <= byteAt(s.to))};
                }));
                return {label, at: byteAt(at), data: [-1, 1].map(side => data(state, at, side)),
                    results: pin, compactData: packedData, compactResults: packed};
            })});
    }
    gapState('split schema tag and open name', "<!-- 字😀 -->\r\n<sec<?= 'é' ?>tion>words\r\n</", text => [
        ['split word replacement', locate(text, 'tion', 2)], ['close suggestion', text.length],
    ]);
    gapState('split schema attribute', "<button aria-la<?= 'é😀' ?>bel=\"a\">", text => [
        ['split attribute replacement', locate(text, 'bel=', 3)],
    ]);
    gapState('script hole and cross-gap local', "<script>\r\nconst local = 'é😀';\r\nlo<?= $body ?>cal;</script>", text => [
        ['split JS replacement', locate(text, 'cal;', 2)], ['body start', locate(text, '>\n', 1)],
        ['body end', locate(text, '</script>')],
    ]);
    gapState('inline CSS with split property', "<p style=\"back<?= $body ?>ground: red\">", text => [
        ['split CSS replacement', locate(text, 'ground:', 3)], ['quote end', locate(text, '\">')],
    ]);
    gapState('gap before raw body', "<script><?= 'é😀' ?>co</script>\r\n<style><?= $gap ?>.x{bac}</style>", text => [
        ['CSS property', locate(text, 'bac', 3)],
    ]);
    gapState('gap at end of raw body', "<script>co<?= 'é' ?></script>\r\n<style>.x{}<?= $gap ?></style>", () => []);
    gapState('gap inside script type', "<script type=\"text/java<?= $mode ?>script\">const x = 1; x</script>", text => [
        ['original route sees PHP', locate(text, 'const', 2)],
    ]);
    gapState('gap inside event name', "<p on<?= $name ?>click=\"return foo\"></p>", text => [
        ['original route sees PHP', locate(text, 'return', 2)],
    ]);
    gapState('quoted event split at quote', "<p onclick=\"<?= $value ?>co<?= $tail ?>\"></p>", text => [
        ['both bracketed edges', locate(text, 'co<?=')],
    ]);
    gapState('lone CR and LF meet only in the projection', "<div>\r<?= 'é😀' ?>\ntext</div>", () => []);
    gapState('empty raw projection after PHP', "<script><?= $code ?>", text => [
        ['gap start at raw boundary', locate(text, '<?=')], ['inside trailing gap', locate(text, '$code', 2)],
        ['PHP at EOF', text.length],
    ]);

    const marker = StateEffect.define({map: (pos, changes) => changes.mapPos(pos, 1)});
    const selection = (state, byteAt) => state.selection.ranges.map(r => [byteAt(r.anchor), byteAt(r.head), r.assoc]);
    function runInputState(source, ranges, main, typed, mode, extension) {
        const {byteAt} = projectedOffsets(source), at = ranges[main].from, to = ranges[main].to;
        const state = EditorState.create({doc: source, selection: EditorSelection.create(ranges, main),
            extensions: [extension, extension, EditorState.allowMultipleSelections.of(true),
                EditorState.readOnly.of(mode === 'read only'), EditorState.transactionFilter.of(tr => {
                    if (!tr.isUserEvent('input.type')) return tr;
                    if (['rewrite', 'rewrite to raw', 'omit typed'].includes(mode)) {
                        const insert = {'rewrite': 'Élément>', 'rewrite to raw': 'script>', 'omit typed': 'x'}[mode];
                        const from = mode === 'omit typed' ? at : at - 'old'.length;
                        return {changes: {from, to, insert}, selection: {anchor: from + insert.length},
                            userEvent: 'input.type', scrollIntoView: true};
                    }
                    return tr;
                })]});
        let steps = [], calls = 0, cached;
        const view = {state, composing: mode === 'composition', dispatch(batch) { steps = batch; }};
        const spec = {...state.replaceSelection(typed), userEvent: 'input.type', scrollIntoView: true};
        if (mode === 'marker') spec.effects = [marker.of(at + typed.length)];
        const base = () => cached ??= state.update(spec);
        const handlers = state.facet(EditorView.inputHandler);
        assert.deepEqual(handlers, [handler.value]);
        const handled = handlers[0](view, at, to, typed, () => { calls++; return base(); });
        if (!handled && !view.composing && !state.readOnly) view.dispatch([base()]);
        let after = source;
        const transactions = steps.map(tr => {
            const old = projectedOffsets(after).byteAt;
            const changes = [];
            tr.changes.iterChanges((from, to, _fromB, _toB, insert) => changes.push([old(from), old(to), insert.toString()]));
            after = projectInput(after, tr);
            const current = projectedOffsets(after).byteAt;
            return {changes, after, selection: selection(tr.state, current), main: tr.state.selection.mainIndex,
                event: tr.annotation(Transaction.userEvent), scroll: tr.scrollIntoView,
                markers: tr.effects.filter(effect => effect.is(marker)).map(effect => current(effect.value))};
        });
        const markers = steps.flatMap((tr, i) => tr.effects.filter(effect => effect.is(marker)).map(effect => {
            let pos = effect.value;
            for (const later of steps.slice(i + 1)) pos = later.changes.mapPos(pos, 1);
            return projectedOffsets(after).byteAt(pos);
        }));
        return {selection: selection(state, byteAt), main, calls, handled, transactions, markers, after};
    }
    function input(name, marked, typed, {mode = 'normal', main = 0, span = false, compare = false} = {}) {
        const parts = marked.split('|'), source = parts.join('');
        let pos = 0;
        const cursors = parts.slice(0, -1).map(part => pos += projectedOffsets(part).units);
        const ranges = cursors.map(at => EditorSelection.cursor(at));
        if (span) ranges.splice(main, 2, EditorSelection.range(cursors[main + 1], cursors[main]));
        const detail = runInputState(source, ranges, main, typed, mode, stock.extension);
        let compactInput;
        if (compare) {
            const state = create(source), segments = textSegments(state);
            const text = segments.map(s => state.doc.sliceString(s.from, s.to)).join('');
            const packedRanges = cursors.map(at => {
                const segment = segments.find(s => s.from <= at && at <= s.to);
                assert(segment);
                return EditorSelection.cursor(segment.start + at - segment.from);
            });
            const {byteAt} = projectedOffsets(source);
            compactInput = {source: text, segments: segments.map(s => [byteAt(s.from), byteAt(s.to)]),
                ...runInputState(text, packedRanges, main, typed, mode, compact)};
        }
        rows.push({kind: 'input', name, source, typed, mode, ...detail, compactInput});
    }
    input('accepted default effect', "😀\r\n<?= 'é' ?><section|", '>', {mode: 'marker'});
    input('accepted default rewrites name', "<?= 'é' ?>\r\n<old|", '>', {mode: 'rewrite'});
    input('accepted default rewrites HTML to a raw tag', "<?= 'é' ?>\r\n<old|", '>', {mode: 'rewrite to raw'});
    input('accepted default removes typed token', "<?= 'é' ?>\r\n<old|", '>', {mode: 'omit typed'});
    input('slash association and adjacent angle', "<Élément><?= 'é' ?>😀<|>tail", '/');
    input('ordinary tag with PHP in attribute', "<div title=\"<?= 'é' ?>\"|", '>');
    input('void', "<?= 'é' ?><img|", '>');
    input('source-spelled uppercase void', "<?= 'é' ?><IMG|", '>');
    input('duplicate close across gap', "<div|<?= 'é' ?></div>", '>');
    input('noMatch mismatch across gap', "<div|<?= 'é' ?></wrong>", '>');
    input('noMatch slash and mismatch', "<div><|<?= 'é' ?></wrong>", '/');
    input('unfinished raw script', "<?= 'é' ?><script|", '>');
    input('unfinished raw style', "<?= 'é' ?><style|", '>');
    input('unsupported raw type', "<?= 'é' ?><script type=\"text/plain\"|", '>');
    input('split raw MIME does not select a mount', "<script type=\"text/java<?= $mime ?>script\"|", '>', {compare: true});
    input('raw body endpoint', "<?= 'é' ?><script>|</script>", '>');
    input('quoted style endpoint', "<?= 'é' ?><div style=\"co|\">", '>');
    input('no JSX input support', "<?= 'é' ?><script type=\"text/jsx\">const x = <Foo|;\r\n</script>", '>');
    input('two HTML carets with second main', "<div|\r\n<?= 'é' ?>\r\n<em|", '>', {main: 1});
    input('PHP secondary receives only default', "<div|\r\n<?php $x = '<em|'; ?>", '>');
    input('PHP main blocks HTML secondary', "<div|\r\n<?php $x = '<em|'; ?>", '>', {main: 1});
    input('raw tag as secondary', "<div|\r\n<?= 'é' ?><script|", '>');
    input('main selection blocks handler', "<div|? |\r\n<?= 'é' ?>", '>', {span: true});
    input('split open name', "<sec<?= 'é' ?>tion|", '>', {compare: true});
    input('split slash name', "<sec<?= 'é' ?>tion>text<|", '/', {compare: true});
    input('gap before adjacent angle', "<div|<?= 'é' ?>>tail", '>', {compare: true});
    // The stock scanner treats the adjacent `<<` spelling as Text, not as a PHP gap.
    input('less-than before a PHP-looking Text token', "<div><<?= 'é' ?>|tail", '/', {compare: true});
    input('secondary less-than before PHP-looking Text', "<span><|\r\n<div><<?= 'é' ?>|tail", '/', {compare: true});
    input('start of Text after gap', "<div <?= 'é' ?>| ", '>');
    input('composition', "<?= 'é' ?><div|", '>', {mode: 'composition'});
    input('read only', "<?= 'é' ?><div|", '>', {mode: 'read only'});

    const find = name => rows.find(row => (row.kind === 'gap' || row.kind === 'input') && row.name === name);
    assert.deepEqual(find('lone CR and LF meet only in the projection').logicalLines, [3, 2]);
    assert.equal(find('split open name').compactInput.segments.length, 2);
    assert.equal(find('less-than before a PHP-looking Text token').compactInput.segments.length, 1);
    assert.equal(find('gap before adjacent angle').transactions[1].changes[0][0], 5);
    assert.equal(find('gap before adjacent angle').transactions[1].changes[0][1], 5);
    assert.deepEqual(find('gap before adjacent angle').compactInput.transactions[1].changes, [[5, 6, '</div>']]);

    const generated = `${rows.map(row => JSON.stringify(row)).join('\n')}\n`;
    if (process.argv[3] === '--emit') process.stdout.write(generated);
    else assert.equal(generated, readFileSync(new URL('./language_data_php.jsonl', import.meta.url), 'utf8'));
} finally {
    rmSync(root, {recursive: true, force: true});
}
