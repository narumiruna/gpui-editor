import assert from 'node:assert/strict';
import {existsSync, mkdtempSync, realpathSync, rmSync} from 'node:fs';
import {join, resolve} from 'node:path';
import {pathToFileURL} from 'node:url';
import {tmpdir} from 'node:os';

assert.equal(process.argv.length, 4, 'usage: node language_data_vue_angular.mjs REPOSITORY ARCHIVES');
const fixtures = resolve(process.argv[2], 'crates/codemirror/tests/fixtures');
const archives = resolve(process.argv[3]);
const {pinnedModules} = await import(pathToFileURL(join(fixtures, 'pinned_modules.mjs')));
const {projectedOffsets, projectInput} = await import(pathToFileURL(join(fixtures, 'editor_line_offsets.mjs')));
const scratch = mkdtempSync(join(realpathSync(tmpdir()), 'vue-angular-modules-'));
const out = row => console.log(JSON.stringify(row));
try {
    const p = pinnedModules(archives, scratch);
    const {languages} = await p.load('@codemirror/language-data');
    const packs = {};
    for (const slug of ['html', 'vue', 'angular', 'javascript', 'css']) packs[slug] = await p.load(`@codemirror/lang-${slug}`);
    const {EditorState, EditorSelection, Compartment, Prec, StateEffect, Transaction} = await p.load('@codemirror/state');
    const {EditorView} = await p.load('@codemirror/view');
    const {syntaxTree, language, languageDataProp} = await p.load('@codemirror/language');
    const {IterMode} = await p.load('@lezer/common');
    const {CompletionContext} = await p.load('@codemirror/autocomplete');
    const names = {H:'HTML', V:'Vue', A:'Angular Template', J:'JavaScript', C:'CSS'};
    const bundles = {};
    for (const [key, name] of Object.entries(names)) {
        const entry = languages.find(e => e.name === name);
        assert(entry, name);
        bundles[key] = await entry.load();
        assert.equal(await entry.load(), bundles[key]);
        assert.deepEqual(bundles[key].extension, [bundles[key].language, bundles[key].support]);
    }
    const {H, V, A, J, C} = bundles;
    assert.equal(V.language, packs.vue.vueLanguage);
    assert.equal(A.language, packs.angular.angularLanguage);
    assert.equal(H.language, packs.html.htmlLanguage);
    assert.equal(new Set([V.language, A.language, H.language]).size, 3);
    const autoCloseTags = packs.html.autoCloseTags;
    const ids = new Map(), inputs = new Set();
    const register = (input, id, facet) => {
        assert.equal(input.facet, facet);
        assert(!inputs.has(input));
        inputs.add(input);
        const fn = input.value.autocomplete;
        if (ids.has(fn)) assert.equal(ids.get(fn), id);
        else ids.set(fn, id);
        return fn;
    };
    for (const key of ['H','V','A']) {
        const b = bundles[key];
        assert.equal(b.language.data, H.language.data);
        assert.equal(b.language.parser.topNode.name, 'Document');
        assert.equal(b.language.parser.topNode.prop(languageDataProp), H.language.data);
        const [schema, handler, [fixed, local, jsx], css] = key === 'H' ? b.support : b.support[0];
        assert.equal(handler, autoCloseTags);
        assert.equal(handler.facet, EditorView.inputHandler);
        assert.deepEqual(jsx, []);
        register(schema, `${key}-schema`, H.language.data);
        register(fixed, `${key}-fixed`, J.language.data);
        assert.equal(register(local, 'local', J.language.data), packs.javascript.localCompletionSource);
        assert.equal(register(css, 'css', C.language.data), packs.css.cssCompletionSource);
        if (key !== 'H') {
            const fresh = packs[key === 'V' ? 'vue' : 'angular'][key === 'V' ? 'vue' : 'angular']();
            assert.equal(fresh.support[0], b.support[0]);
            assert.notEqual(fresh.support[1], b.support[1]);
            const overlay = b.support[1];
            assert.equal(overlay.facet, H.language.data);
            assert.deepEqual(overlay.value.closeBrackets.brackets, key === 'V' ? ['{','"'] : ['[','{','"']);
        }
    }
    register(J.support[0], 'J-fixed', J.language.data);
    register(J.support[1], 'local', J.language.data);
    register(C.support, 'css', C.language.data);
    assert.equal(ids.size, 9);
    const keys = ['zIndex', 'display', 'backgroundColor', 'cssFloat', 'cssText'];
    globalThis.document = {body: {style: Object.fromEntries(keys.map(k => [k, '']))}};
    const triggers = ['}', '  ]', '</div>', ' default:', ' {', ' abc'];
    const id = fn => { assert(ids.has(fn)); return ids.get(fn); };
    const extension = key => key.startsWith('high ') ? Prec.high(extension(key.slice(5))) :
        key.startsWith('bare ') ? bundles[key.slice(5)].language.extension : bundles[key].extension;
    const installed = state => Object.keys(bundles).find(k => bundles[k].language === state.facet(language)) ?? null;
    function topAt(tree, at, side) {
        let top = tree.topNode;
        for (let n = top; n; n = n.enter(at, side, IterMode.ExcludeBuffers | IterMode.EnterBracketed))
            if (n.type.isTop) top = n;
        return top;
    }
    const facetId = new Map([[H.language.data, 'html'], [J.language.data, 'js'], [C.language.data, 'css']]);
    const project = (source, tr) => {
        const {byteAt} = projectedOffsets(source), edits = [];
        tr.changes.iterChanges((from, to, _from, _to, insert) => edits.push([byteAt(from), byteAt(to), insert.toString()]));
        return [edits, projectInput(source, tr)];
    };
    function summary(fn, state, at, explicit, byteAt) {
        syntaxTree(state).resolveInner(0, -1);
        const r = fn(new CompletionContext(state, at, explicit));
        if (!r) return null;
        let hash = 2166136261;
        for (const {label, type, detail, boost, apply, ...rest} of r.options) {
            assert.deepEqual(rest, {});
            const text = typeof apply === 'function' ? '@snippet' : apply ?? '';
            for (const b of Buffer.from(`${label}\0${type ?? ''}\0${detail ?? ''}\0${boost ?? 0}\0${text}\n`)) hash = Math.imul(hash ^ b, 16777619) >>> 0;
        }
        return [byteAt(r.from), r.to == null ? null : byteAt(r.to), r.filter ?? true, r.options.length, hash];
    }
    function query(state, source, label, at) {
        const {byteAt} = projectedOffsets(source), tree = syntaxTree(state);
        tree.resolveInner(0, -1);
        return {label, at:byteAt(at), left:tree.resolveInner(at,-1).name, sides:[-1,1].map(side => {
            const top = topAt(tree, at, side);
            const data = key => state.languageDataAt(key, at, side);
            const files = data('autocomplete');
            return {top:top.name, facet:facetId.get(top.type.prop(languageDataProp)) ?? null,
                sources:files.map(id),
                comments:data('commentTokens').map(({line, block}) => [line ?? null, block ? [block.open, block.close] : null]),
                words:data('wordChars'), brackets:data('closeBrackets').map(({brackets,before,stringPrefixes}) => [brackets ?? null,before ?? null,stringPrefixes ?? []]),
                indent:data('indentOnInput').map(rx => triggers.map(s => rx.test(s))),
                results:[false,true].map(explicit => files.map(f => summary(f,state,at,explicit,byteAt)))};
        })};
    }
    const points = (text, part, locs = [0,1,part.length]) => {
        const at = text.indexOf(part);
        assert(at >= 0, part);
        return locs.map(offset => [`${part} @${offset}`,at+offset]);
    };
    function scenario(name, source, probes, steps) {
        const slot = new Compartment();
        let state = EditorState.create({doc:source, extensions:slot.of([])});
        let retained;
        out({kind:'start', name, source});
        for (const [step, exts, edit, head] of steps) {
            const before = syntaxTree(state);
            const from = edit && state.doc.toString().indexOf(edit[0]);
            assert(!edit || from >= 0, step);
            const tr = state.update({changes:edit && {from,to:from+edit[0].length,insert:edit[1]},
                selection:head == null ? undefined : {anchor:head}, effects:exts && slot.reconfigure(exts.map(extension))});
            const [edits,next] = project(source,tr);
            state = tr.state; source = next;
            const tree = syntaxTree(state), handlers = state.facet(EditorView.inputHandler);
            assert(handlers.length <= 1 && handlers.every(fn => fn === autoCloseTags.value));
            if (!retained && /catalog/.test(step)) retained = {state, source};
            const queries = probes(state.doc.toString()).map(([label,at]) => query(state,source,label,at));
            out({kind:'state', name, step, exts, edits, source, active:installed(state), root:tree.topNode.name,
                head:projectedOffsets(source).byteAt(state.selection.main.head), reused:tree === before, handlers:handlers.length, queries});
        }
        if (retained) out({kind:'retained', name, source:retained.source, active:installed(retained.state), queries:probes(retained.state.doc.toString()).map(([label,at]) => query(retained.state,retained.source,label,at))});
    }
    assert(!existsSync(join(scratch, 'archives')), 'expected only local files');
    out({kind:'identity', pins:[...p.installed].map(([n])=>[n,p.pins.get(n).version]), names, keys, triggers});
    scenario('Vue text & directives', '<div>plain</div>\r\n<p>字😀 lead {{ val }} tail</p><x :title="key" class="a"/>', text => [
        ...points(text,'plain'), ...points(text,'lead'), ...points(text,'{{ val }}',[0,1,2,3,4,6,7,8,9]), ...points(text,'tail'),
        ...points(text,':title="key"',[0,1,6,7,8,9,11,12]), ...points(text,'class',[3]),
    ], [['bare', ['bare V']],['catalog', ['V']],['selection only',undefined,undefined,3],['repeat',['V','V']],
        ['edit before ranges',undefined,['字😀','🧪文']],['competing order',['V','A','H','J','C']],
        ['Angular precedence',['high A','V']],['HTML precedence',['high H','V']],['detached',[]],['reattach',['V']]]);
    scenario('Angular entire attribute', '<div style="color: {{val}} red" onclick="pre {{key}} post" [title]="foo" #ref class="a">plain</div>', text => {
        const style = /style="[^\"]*"/.exec(text)[0], event = /onclick="[^\"]*"/.exec(text)[0];
        return [...points(text,style,[0,1,5,6,7,8,14,15,16,17,19,20,21,style.length-1,style.length]),
            ...points(text,event,[0,1,7,8,9,10,13,14,15,16,17,19,20,21,event.length-1,event.length]),
            ...points(text,'[title]="foo"',[0,1,7,8,9,10,12,13]), ...points(text,'#ref'), ...points(text,'class',[3]), ...points(text,'plain')];
    }, [['catalog',['A']],['repeat',['A','A']],['remove CSS mask',undefined,['{{val}}','initial']],['remove JS mask',undefined,['{{key}}','return 1']],['remask CSS',undefined,['initial','{{val}}']],['remask JS',undefined,['return 1','{{key}}']],['restock both', ['A','V','H','J','C']],['same source with Vue',['V']],['same source with HTML',['H']],['detach',[]]]);
    for (const key of ['V','A']) scenario(`${key} inherited children`, '<div style="color: red" onclick="let here=1; her"><script lang="ts">const local: number = 1; loc;</script><style>.a{bac}</style><script type="text/jsx">const tag = <Widget/>;</script></div>', text => [
        ...points(text,'color: red'), ...points(text,'let here=1; her',[0,1,15]), ...points(text,'const local: number = 1; loc;',[0,1,26,29]),
        ...points(text,'.a{bac}',[0,1,5,7]), ...points(text,'Widget',[2]),
    ], [['catalog',[key]],['standalone JS and CSS inputs',[key,'J','C']],['high JS wins',['high J',key]]]);
    for (const key of ['V','A']) scenario(`${key} unquoted child edges`, '<div style=bac onclick=handler></div>', text => [
        ...points(text,'bac'), ...points(text,'handler'),
    ], [['catalog',[key]]]);
    scenario('Angular statement binding', '<div (click)="let local=1; loc" [foo]="item" data="{{value}}">hi {{val}} tail</div>', text => [
        ...points(text,'(click)'), ...points(text,'let local=1; loc',[0,5,13,16]), ...points(text,'item'), ...points(text,'value'), ...points(text,'val}}'),
    ], [['catalog',['A']]]);
    for (const key of ['V','A']) scenario(`${key} comments and raw no-mount controls`, '<p> &amp; x<!-- comment -->\r\n </p><textarea>{{ raw }}</textarea><script type="text/plain">{{ raw }}</script><script></script>', text => [
        ...points(text,' &amp; x'), ...points(text,'comment'), ...points(text,'\n '), ...points(text,'{{ raw }}'), ...points(text,'</script><script>',[0,17]), ...points(text,'<script></script>',[8]),
    ], [['catalog',[key]]]);
    function input(key, name, marked, typed, mode = 'normal') {
        const chunks = marked.split('|'), source = chunks.join(''), {byteAt} = projectedOffsets(source);
        let pos = 0;
        const carets = chunks.slice(0,-1).map(s => pos += projectedOffsets(s).units);
        const [at] = carets;
        const marker = StateEffect.define({map:(pos, change) => change.mapPos(pos,1)});
        const state = EditorState.create({doc:source, selection:EditorSelection.create(carets.map(at => EditorSelection.cursor(at))),
            extensions:[extension(key),extension(key),extension(key === 'V' ? 'A' : 'V'),extension('H'),EditorState.allowMultipleSelections.of(true),
                EditorState.readOnly.of(mode === 'readonly'), EditorState.transactionFilter.of(tr => {
                    if (!tr.isUserEvent('input.type')) return tr;
                    if (mode === 'rewrite') return {changes:{from:0,to:at, insert:'<rewritten>'},selection:{anchor:11},userEvent:'input.type',scrollIntoView:true};
                    if (mode === 'omit') return {changes:{from:at,insert:'x'},selection:{anchor:at+1},userEvent:'input.type',scrollIntoView:true};
                    return tr;
                })]});
        const handlers = state.facet(EditorView.inputHandler);
        assert.deepEqual(handlers,[autoCloseTags.value]);
        const spec = {...state.replaceSelection(typed),userEvent:'input.type',scrollIntoView:true};
        if (mode === 'marker') spec.effects = [marker.of(at + 1)];
        let steps = [], calls = 0, cached;
        const base = () => cached ??= state.update(spec);
        const view = {state,composing:mode === 'composition',dispatch(batch) { steps = batch; view.state = batch.at(-1).state; }};
        const handled = handlers[0](view,at,at,typed,() => { calls++; return base(); });
        if (!handled && !view.composing && !state.readOnly) view.dispatch([base()]);
        let after = source;
        out({kind:'input', key, name, source, carets:carets.map(byteAt), typed, mode, calls, handled, transactions:steps.map(tr => {
            const [edits,next] = project(after,tr); after = next;
            const {byteAt} = projectedOffsets(after);
            return {edits, after, ranges:tr.state.selection.ranges.map(r => [byteAt(r.anchor),byteAt(r.head),r.assoc]), event:tr.annotation(Transaction.userEvent),
                scroll:tr.scrollIntoView, markers:tr.effects.filter(f=>f.is(marker)).map(f=>byteAt(f.value))};
        })});
    }
    for (const key of ['V','A']) {
        input(key,'close after Unicode and CRLF','<!-- 字😀 -->\r\n<section|','>','marker');
        input(key,'slash replaces adjacent >','<section><|>after','/');
        input(key,'greater than replaces adjacent >','<section|>after','>');
        input(key,'accepted rewrite','<old|','>','rewrite');
        input(key,'accepted omission','<old|','>','omit');
        input(key,'plain text gate even when rewritten','<p>pl|ain</p>','>','rewrite');
        input(key,'Text start left-side gate','<p>|plain</p>','>','rewrite');
        input(key,'Text end left-side gate','<p>plain|</p>','>','rewrite');
        input(key,'attribute gate even when rewritten',key === 'V' ? '<p :tit|le="foo">' : '<p style="co|lor: {{val}} red">','>','rewrite');
        input(key,'expression gate','<p>{{ va|l }}</p>','>','rewrite');
        input(key,'raw script endpoint','<script>|co</script>','>','rewrite');
        input(key,'quoted style endpoint','<p style="|co">','>','rewrite');
        input(key,'incomplete raw tag','<script|','>');
        input(key,'incomplete raw tag with rewritten default','<script|','>','rewrite');
        input(key,'unmounted raw MIME control','<script type="text/plain"|','>','rewrite');
        input(key,'multiple carets incl raw','<div|\r\n<script|','>');
        input(key,'no JSX input support','<script type="text/jsx">const x = <Foo|;</script>','>');
        input(key,'read only','<div|','>','readonly');
        input(key,'host composition','<div|','>','composition');
    }
} finally {
    rmSync(scratch, {recursive:true,force:true});
}
