// node language_data_jinja_liquid.mjs REPOSITORY PIN_ARCHIVES > language_data_jinja_liquid.jsonl
import assert from 'node:assert/strict';
import {createHash} from 'node:crypto';
import {existsSync, mkdtempSync, realpathSync, rmSync} from 'node:fs';
import {registerHooks} from 'node:module';
import {tmpdir} from 'node:os';
import {join, resolve} from 'node:path';
import {pathToFileURL} from 'node:url';

assert.equal(process.argv.length, 4, 'usage: node language_data_jinja_liquid.mjs REPOSITORY PIN_ARCHIVES');
const fixtures = resolve(process.argv[2], 'crates/codemirror/tests/fixtures');
const archives = resolve(process.argv[3]);
const {pinnedModules} = await import(pathToFileURL(join(fixtures, 'pinned_modules.mjs')));
const {projectedOffsets, projectInput} = await import(pathToFileURL(join(fixtures, 'editor_line_offsets.mjs')));
const scratch = mkdtempSync(join(realpathSync(tmpdir()), 'gpui-jinja-liquid-pins.'));
const out = row => console.log(JSON.stringify(row));
try {
    const p = pinnedModules(archives, scratch);
    const viewURL = p.install('@codemirror/view').url.href;
    // Export private functions in memory only, after archive verification. No function body is rewritten.
    let instrumented = 0;
    const hook = registerHooks({load(url, context, next) {
        const result = next(url, context);
        if (url !== viewURL) return result;
        instrumented++;
        return {...result, source: `${result.source}\nexport { DOMChange, applyDOMChange, applyDOMChangeInner, applyDefaultInsert };\n`};
    }});
    const {EditorView, DOMChange, applyDOMChange, applyDOMChangeInner, applyDefaultInsert} = await p.load('@codemirror/view');
    hook.deregister();
    assert.equal(instrumented, 1);
    const {EditorState, EditorSelection, Compartment, Prec, StateEffect, Transaction} = await p.load('@codemirror/state');
    const {language, syntaxTree, ensureSyntaxTree, languageDataProp} = await p.load('@codemirror/language');
    const {IterMode, NodeProp} = await p.load('@lezer/common');
    const {CompletionContext} = await p.load('@codemirror/autocomplete');
    const {languages} = await p.load('@codemirror/language-data');
    const packages = {};
    for (const key of ['jinja', 'liquid', 'html', 'javascript', 'css']) packages[key] = await p.load(`@codemirror/lang-${key}`);
    const names = {jinja:'Jinja', liquid:'Liquid', html:'HTML', javascript:'JavaScript', css:'CSS'};
    const bundles = {}, entries = {};
    for (const [key, name] of Object.entries(names)) {
        const entry = languages.find(e => e.name === name);
        assert(entry, name);
        entries[key] = entry;
        bundles[key] = await entry.load();
        assert.equal(await entry.load(), bundles[key]);
        assert.deepEqual(bundles[key].extension, [bundles[key].language, bundles[key].support]);
    }
    const H = bundles.html.language, J = bundles.javascript.language, C = bundles.css.language;
    assert.equal(H, packages.html.htmlLanguage);
    assert.equal(J, packages.javascript.javascriptLanguage);
    assert.equal(C, packages.css.cssLanguage);
    assert.equal(packages.javascript.typescriptLanguage.data, J.data);
    assert.equal(packages.javascript.jsxLanguage.data, J.data);
    const handlers = new Map([[packages.html.autoCloseTags.value, 'html-close']]);
    const facets = new Map([[H.data, 'html'], [J.data, 'js'], [C.data, 'css']]);
    const parsers = new Map([[H.parser, 'html'], [J.parser, 'js'], [C.parser, 'css'],
        [packages.javascript.typescriptLanguage.parser, 'ts'], [packages.javascript.jsxLanguage.parser, 'jsx']]);
    const sources = new Map(), inputs = new Set();
    const id = (ids, value) => { assert(ids.has(value)); return ids.get(value); };
    function register(input, facet, name) {
        assert.equal(input.facet, facet);
        assert(!inputs.has(input), name);
        inputs.add(input);
        const fn = input.value.autocomplete;
        assert.equal(typeof fn, 'function');
        if (sources.has(fn)) assert.equal(sources.get(fn), name);
        else sources.set(fn, name);
        return fn;
    }
    function htmlSupport(support, key) {
        const [schema, handler, [fixed, local, jsx], css] = support;
        assert.equal(handler, packages.html.autoCloseTags);
        assert.equal(handler.facet, EditorView.inputHandler);
        assert.deepEqual(jsx, []);
        register(schema, H.data, `${key}:schema`);
        register(fixed, J.data, `${key}:fixed-js`);
        assert.equal(register(local, J.data, 'local-js'), packages.javascript.localCompletionSource);
        assert.equal(register(css, C.data, 'css'), packages.css.cssCompletionSource);
    }
    htmlSupport(bundles.html.support, 'html');
    register(bundles.javascript.support[0], J.data, 'standalone:fixed-js');
    assert.equal(register(bundles.javascript.support[1], J.data, 'local-js'), packages.javascript.localCompletionSource);
    assert.equal(register(bundles.css.support, C.data, 'css'), packages.css.cssCompletionSource);
    for (const key of ['jinja', 'liquid']) {
        const pkg = packages[key], bundle = bundles[key], lang = bundle.language;
        assert.equal(lang, pkg[`${key}Language`]);
        assert.equal(lang.parser.topNode.name, 'Template');
        assert(!facets.has(lang.data));
        assert.equal(lang.parser.topNode.prop(languageDataProp), lang.data);
        facets.set(lang.data, key);
        const [base, completion, brackets, percent] = bundle.support;
        htmlSupport(base, key);
        register(completion, lang.data, `${key}:template`);
        assert.equal(brackets.facet, H.data);
        assert.deepEqual(brackets.value, {closeBrackets:{brackets:['{']}});
        assert.equal(percent, pkg.closePercentBrace);
        assert.equal(percent.facet, EditorView.inputHandler);
        handlers.set(percent.value, `${key}:percent`);
        const fresh = pkg[key]();
        assert.equal(fresh.language, lang);
        assert.equal(fresh.support[0], base);
        assert.notEqual(fresh.support[1].value.autocomplete, completion.value.autocomplete);
        assert.notEqual(fresh.support[2], brackets);
        const supplied = packages.html.html();
        const withBase = pkg[key]({base: supplied});
        assert.equal(withBase.language, lang);
        assert.equal(withBase.support[0], supplied.support);
    }
    assert.equal(sources.size, 11);
    assert.equal(handlers.size, 3);
    assert(!existsSync(join(scratch, 'archives')), 'expected all archives to be local');
    // A small deterministic CSSOM property surface. No claim about the browser property list.
    const cssKeys = ['zIndex', 'display', 'backgroundColor', 'cssFloat', 'cssText'];
    globalThis.document = {body:{style:Object.fromEntries(cssKeys.map(k => [k, '']))}};
    const triggers = [' </div>', '  }', ' case:', '{%', '{%- end', '  {% else', '{% elif', '{% elsif', '{% when', '{% endif'];
    const keys = ['autocomplete', 'closeBrackets', 'commentTokens', 'wordChars', 'indentOnInput'];
    const handlerIds = state => state.facet(EditorView.inputHandler).map(fn => id(handlers, fn));
    const rootName = state => state.facet(language)?.name ?? null;
    const fingerprint = options => createHash('sha256').update(JSON.stringify(options,
        (_key, value) => typeof value === 'function' ? `@${value.name}` : value)).digest('hex');
    function convert(key, value) {
        if (key === 'autocomplete') return id(sources, value);
        if (key === 'indentOnInput') return {regex:String(value), matches:triggers.map(text => value.test(text))};
        return value;
    }
    function dataRows(state, facet) {
        return facet ? state.facet(facet).map(row => Object.fromEntries(Object.entries(row).map(([key, value]) => [key, convert(key, value)]))) : [];
    }
    function topAt(tree, at, side) {
        let top = tree.topNode;
        for (let node = top; node; node = node.enter(at, side, IterMode.ExcludeBuffers | IterMode.EnterBracketed))
            if (node.type.isTop) top = node;
        return top;
    }
    function treeFor(state) {
        if (!state.facet(language)) return syntaxTree(state);
        const tree = ensureSyntaxTree(state, state.doc.length, 1000);
        assert(tree && tree === syntaxTree(state));
        return tree;
    }
    function pieces(source, tree) {
        const {byteAt} = projectedOffsets(source);
        const mount = tree.prop(NodeProp.mounted);
        if (!mount) return null;
        assert.equal(mount.parser, H.parser);
        const html = mount.overlay.map(({from, to}) => [byteAt(from), byteAt(to)]);
        const origin = mount.overlay[0].from, children = [];
        mount.tree.iterate({mode:IterMode.IgnoreMounts, enter(node) {
            const child = node.tree?.prop(NodeProp.mounted);
            if (!child) return;
            const bounds = child.overlay ?? [{from:0, to:node.to - node.from}];
            const ranges = bounds.flatMap(({from,to}) => mount.overlay.flatMap(r => {
                const a = Math.max(r.from, origin + node.from + from), b = Math.min(r.to, origin + node.from + to);
                return a < b ? [[byteAt(a),byteAt(b)]] : [];
            }));
            const type = child.tree.type, facet = type.prop(languageDataProp);
            children.push({host:node.name, top:type.name, parser:parsers.get(child.parser) ?? `configured:${type.name}`,
                facet:facet ? id(facets, facet) : null, bracketed:child.bracketed ?? false, ranges});
        }});
        return {html, children};
    }
    function result(fn, state, at, explicit, byteAt, tree, html) {
        // Reset Lezer's last resolveInner node. Each call starts at the same root,
        // independent of which side or source the preceding probe visited.
        tree.resolveInner(0,-1);
        const r = fn(new CompletionContext(state, at, explicit));
        if (!r) return null;
        const from = byteAt(r.from), end = byteAt(r.to ?? at);
        const crossesHtmlGap = html && end > from && html.some(([a,b],i) =>
            i > 0 && from < a && html[i-1][1] < end && html[i-1][1] < a);
        return {from, to:r.to == null ? null : end, ...(crossesHtmlGap ? {crossesHtmlGap:true} : {}), filter:r.filter ?? true,
            ...(id(sources,fn).endsWith(':template') ? {templateOptions:r.options.map(({label,type}) => [label,type])} : {}),
            validFor:r.validFor ? String(r.validFor) : null, count:r.options.length,
            head:r.options.slice(0,3).map(o=>o.label), tail:r.options.slice(-2).map(o=>o.label), hash:fingerprint(r.options)};
    }
    function queries(state, source, tree, points, html) {
        const {byteAt} = projectedOffsets(source);
        return points.map(([name, at]) => {
            const l = tree.topNode.resolveInner(at, -1), r = tree.topNode.resolveInner(at, 1);
            const path = node => {
                const names = [];
                for (; node; node=node.parent) names.unshift(node.name);
                return names.join('/');
            };
            const unfinished = l.enterUnfinishedNodesBefore(at);
            const sides = [-1,1].map(side => {
                const top = topAt(tree, at, side), facet = top.type.prop(languageDataProp);
                const data = Object.fromEntries(keys.map(key => [key,state.languageDataAt(key, at, side).map(x => convert(key,x))]));
                const originals = state.languageDataAt('autocomplete', at, side);
                const results = [false,true].map(explicit => originals.map(fn => result(fn, state, at, explicit, byteAt, tree, ['html','css','js'].includes(facets.get(facet)) ? html : null)));
                return {top:top.name, facet:facet ? id(facets,facet) : null, htmlActive:H.isActiveAt(state,at,side), data, results};
            });
            return {name, at:byteAt(at), paths:[path(l),path(r)], unfinished:path(unfinished), sides};
        });
    }
    const project = (source, tr) => {
        const {byteAt} = projectedOffsets(source), edits = [];
        tr.changes.iterChanges((from, to, _from, _to, insert) => edits.push([byteAt(from),byteAt(to),insert.toString()]));
        return [edits, projectInput(source, tr)];
    };
    const point = (state, text, offset = 0) => {
        const at = state.doc.toString().indexOf(text);
        assert(at >= 0, text);
        return at + offset;
    };
    function marked(text) {
        const source = text.replaceAll('§','');
        let pos = 0;
        const points = text.split('§').slice(0,-1).map(chunk => pos += projectedOffsets(chunk).units);
        return [source, points];
    }
    out({kind:'capture', text:'Pinned upstream runtime; offsets/ranges are original UTF-8; sides are [-1,1]; results are [implicit,explicit]; option hashes are SHA-256 of ordered upstream options',
        packages:[...p.installed.keys()].sort().map(name => {const {version,tarball,integrity}=p.pins.get(name);return {name,version,tarball,integrity};}),
        cssKeys, triggers,
        sourceInventory:{catalog:'language-data calls jinja() and liquid() with no arguments',
            defaults:'Both modules allocate one html() base per module; Text/RawText overlay; HTML receives default dialect/routes, stock schema, tag handler, non-JSX javascript().support, and css().support',
            config:'Jinja: base/tags/variables/properties. Liquid: base/filters/tags/variables/properties. No options supplied by catalog.',
            input:'Actual input handlers and defaults; private view helpers exposed in memory; DOM diff detection and IME events not simulated.'}});
    for (const key of ['jinja','liquid']) {
        const bundle = bundles[key];
        const solo = EditorState.create({extensions:[bundle.extension,bundle.extension]});
        assert.deepEqual(handlerIds(solo), ['html-close',`${key}:percent`]);
        out({kind:'identity', key, catalog:{name:entries[key].name, aliases:entries[key].alias, extensions:entries[key].extensions},
            root:[rootName(solo), treeFor(solo).topNode.name], templateRows:dataRows(solo, bundle.language.data),
            htmlRows:dataRows(solo,H.data), jsRows:dataRows(solo,J.data), cssRows:dataRows(solo,C.data), handlers:handlerIds(solo),
            checked:'Cached catalog bundle, exported root; own base per module, reused by fresh factory; fresh schema and fixed JS per base; distinct input wrappers; shared local/CSS callables; same HTML handler.'});
    }
    const main = '😀\r\n{% if enabled %}<div style="col{{ tone }}or: red" onclick="let local=1; lo{{ arg }}cal">a{{ name | up }}z</div>' +
        '<script lang="ts">const one: number = 1; on{{ val }}e;</script><style>.a { bac }</style>' +
        '<script type="text/jsx">const tag = <Widget/>;</script><script type="application/json">{"n":1}</script>' +
        '<script type="text/plain">plain</script>{% raw %}<i>{{ literal }}</i>{% endraw %}\r\n{% endif %}';
    function snapshot(key, name, state, source, points, extra = {}) {
        const tree = treeFor(state), regions=pieces(source, tree);
        out({kind:'state', key, name, source, active:rootName(state), root:tree.topNode.name, handlers:handlerIds(state),
            regions, ...extra, queries:queries(state,source,tree,points,regions?.html)});
    }
    for (const key of ['jinja','liquid']) {
        const bundle = bundles[key];
        const slot = new Compartment();
        let state = EditorState.create({doc:main,extensions:slot.of(bundle.extension)}), source = main;
        const pnt = (name, text, offset) => [name, point(state,text,offset)];
        snapshot(key,'catalog mixed',state,source,[
            pnt('template word','enabled',4), pnt('HTML left edge','<div'), pnt('HTML tag','<div',3),
            pnt('style mount open','col{{',0), pnt('start of CSS gap','{{ tone }}'), pnt('inside CSS gap','tone',2),
            pnt('end of CSS gap','}}or:',2), pnt('CSS after gap','or: red',2), pnt('style mount close','red"',3),
            pnt('JS local definition','let local',9), pnt('JS local before gap','lo{{ arg',2), pnt('JS after gap','}}cal',5), pnt('filter','name | up',9),
            pnt('end of body gap','}}z</div>',2), pnt('TS body open','const one'), pnt('TS body close','e;</script>',2),
            pnt('CSS body','bac',3), pnt('JSX parser, JS inputs','Widget',3), pnt('JSON top','"n":1',4),
            pnt('raw MIME HTML','>plain',3), pnt('RawText is HTML','literal',3), pnt('template EOF','endif',8)]);
        const retained = state, oldTree = treeFor(state);
        let tr = state.update({selection:{anchor:point(state,'tone',2)},effects:slot.reconfigure([bundle.extension,bundle.extension])});
        state = tr.state;
        assert.equal(treeFor(state), oldTree);
        assert.deepEqual(handlerIds(state), ['html-close',`${key}:percent`]);
        // Observe another factory's support without adding another language-data provider.
        const other = bundles[key === 'jinja' ? 'liquid' : 'jinja'];
        tr = state.update({effects:slot.reconfigure([bundle.extension, other.support, bundles.html.support, bundles.javascript.support, bundles.css.support])});
        state = tr.state;
        snapshot(key,'combined support only',state,source,[pnt('HTML sources','<div',3),pnt('JS sources','let local',9),pnt('CSS sources','bac',3)]);
        const from = point(state,'text/plain');
        tr = state.update({changes:{from,to:from+'text/plain'.length,insert:'text/javascript'},effects:slot.reconfigure(bundle.extension)});
        const [edits, next] = project(source,tr);
        state=tr.state; source=next;
        snapshot(key,'MIME edit and reconfigure', state,source,[pnt('new JS mount','>plain',3)],{edits, retainedTree:treeFor(retained) === oldTree});
        tr = state.update({effects:slot.reconfigure(bundle.language.extension)});
        state=tr.state;
        snapshot(key,'bare root',state,source,[pnt('bare HTML facet','<div',3),pnt('bare template facet','enabled',4),pnt('bare JS facet','let local',9)]);
        tr = state.update({effects:slot.reconfigure([])});
        state=tr.state;
        snapshot(key,'detached',state,source,[pnt('no data','<div',3)]);
        tr=state.update({effects:slot.reconfigure([Prec.high(H.data.of({closeBrackets:{brackets:['[']}})), bundle.extension,
            Prec.low(H.data.of({closeBrackets:{brackets:['(']}}))])});
        state=tr.state;
        const at=point(state,'<div',3);
        assert.deepEqual(state.languageDataAt('closeBrackets',at).map(o=>o.brackets), [['['],['{'],['(']]);
        snapshot(key,'reattached with bracket precedence',state,source,[['HTML bracket order',at],pnt('template not HTML','enabled',4)]);

        for (const [name,text] of [
            ...(key === 'jinja' ? [['unfinished-node descent', '{{ {{x}}§']] : []),
            ['stock template sources','{% giz§mo %}{{ va§r }}{{ word|§ up§ }}{{ person.na§me }}{{ forloop.in§d }}'],
            ['empty, incomplete tag and trailing whitespace','{% §'],
            ['incomplete nested template','{% if enabled %}{% en§'],
            ['incomplete filter and trailing whitespace','{{ user | §'],
            ['incomplete HTML before vs after whitespace','<div class=§ §'],
            ['split closing name, incomplete HTML','<div><d{{ gap }}iv><§/§  §'],
            ['replacement spans a template gap','<a hre{{ gap }}f§=x style="co{{ gap }}l§"><script>pre{{ x }}fix§</script></a>'],
            ['unfinished interpolation versus closed','{{ "<em§'],
            ['closed interpolation control','{{ "<em§" }}'],
        ]) {
            const [doc, positions] = marked(text);
            const s = EditorState.create({doc,extensions:bundle.extension});
            snapshot(key,name,s,doc,positions.map((at,i)=>[`site ${i}`,at]));
        }
    }

    const marker = StateEffect.define({map:(at, changes) => changes.mapPos(at,1)});
    function makeView(state, composing) {
        const batches = [];
        const v = {state, inputState:{composing:composing ? 0 : -1, compositionFirstChange:composing},
            get composing() {return this.inputState.composing > 0;}, dispatch:EditorView.prototype.dispatch,
            dispatchTransactions(batch, self) {
                assert.equal(self,v);
                batches.push(batch);
                for (const tr of batch) {
                    assert.equal(tr.startState, v.state);
                    v.state=tr.state;
                }
            }};
        return [v,batches];
    }
    function selectionData(selection, source) {
        const {byteAt} = projectedOffsets(source);
        return {main:selection.mainIndex,ranges:selection.ranges.map(r=>[byteAt(r.anchor),byteAt(r.head),r.assoc])};
    }
    function input(key, name, text, typed, mode = 'normal', mainIndex=0, pairLast=false, bare=false) {
        const [source, points] = marked(text);
        const reported=mode === 'reported' ? points.pop() : null;
        const ranges=points.map(at=>EditorSelection.cursor(at));
        if (pairLast) {const head=points.at(-2), anchor=points.at(-1);ranges.splice(-2,2,EditorSelection.range(anchor,head));}
        const slot = new Compartment(), bundle = bundles[key];
        const extensions = [slot.of(bare ? bundle.language.extension : bundle.extension), EditorState.allowMultipleSelections.of(true),
            EditorState.readOnly.of(mode === 'readonly')];
        if (mode === 'protect') extensions.push(EditorState.changeFilter.of(tr =>
            !tr.isUserEvent('input.type') || tr.newDoc.toString().includes('%%') === false));
        if (mode === 'protect-close') extensions.push(EditorState.changeFilter.of(tr => !tr.isUserEvent('input.complete')));
        if (mode === 'rewrite' || mode === 'omit') extensions.push(EditorState.transactionFilter.of(tr => {
            if (!tr.isUserEvent('input.type')) return tr;
            const main=tr.startState.selection.main;
            const insert = mode === 'omit' ? 'x' : '<rewritten>';
            const from = mode === 'omit' ? main.from : 0;
            return {changes:{from,to:main.to,insert},selection:{anchor:from+insert.length},userEvent:'input.type',scrollIntoView:true};
        }));
        if (mode === 'effect' || mode === 'detach') extensions.push(EditorState.transactionExtender.of(tr => {
            if (!tr.isUserEvent('input.type')) return null;
            return {effects:mode === 'effect' ? marker.of(tr.newDoc.length) : slot.reconfigure([])};
        }));
        const state=EditorState.create({doc:source,selection:EditorSelection.create(ranges,mainIndex),extensions});
        const from = reported ?? state.selection.main.from, to = reported ?? state.selection.main.to;
        const insert = state.toText(typed), composing = mode === 'compose';
        const change = {from,to,insert}, newSel = composing ? EditorSelection.single(from + insert.length) : null;
        const [view,batches] = makeView(state,composing), [actual,actualBatches] = makeView(state,composing);
        // Mirror only the offer loop so we can count which handler requested the real default.
        // Compare the dispatch with the actual upstream loop below.
        let built=0, cached, offered=[];
        if (view.inputState.composing >= 0) view.inputState.composing++;
        const base=()=>cached ??= (built++,applyDefaultInsert(view,change,newSel));
        for (const handler of state.facet(EditorView.inputHandler)) {
            const row={handler:id(handlers,handler),calls:0};
            row.handled=handler(view,from,to,typed,()=>{row.calls++;return base();});
            offered.push(row);
            if (row.handled) break;
        }
        if (!offered.some(row=>row.handled)) view.dispatch(base());
        assert.equal(applyDOMChangeInner(actual,change,newSel),true);
        function trace(list) {
            let before=source;
            return list.map(batch=>batch.map(tr=>{
                const [edits,next]=project(before,tr); before=next;
                return {edits, source:next, selection:selectionData(tr.state.selection,next), scroll:tr.scrollIntoView,
                    event:tr.annotation(Transaction.userEvent) ?? null, root:rootName(tr.state),
                    effects:tr.effects.map(e=>e.is(marker) ? ['marker',projectedOffsets(next).byteAt(e.value)] : ['reconfigure',handlerIds(tr.state)])};
            }));
        }
        const traced = trace(batches);
        assert.deepEqual(trace(actualBatches),traced);
        let accepted=traced;
        if (state.readOnly) {
            const [guarded, already] = makeView(state,false);
            guarded.docView={};
            const dom=new DOMChange(guarded,0,1,false);
            assert.equal(applyDOMChange(guarded,dom),false);
            assert.deepEqual(already,[]);
            accepted=[];
        }
        if (typed === '%' && offered.some(row=>row.handled)) {
            assert.equal(built,0);
            assert.deepEqual(traced.map(batch=>batch.map(tr=>tr.event)), [['input.type']]);
        }
        if (offered.some(row=>row.handled && row.handler==='html-close')) {
            assert.equal(built,1);
            assert.deepEqual(traced.map(batch=>batch.map(tr=>tr.event)), [['input.type','input.complete']]);
        }
        const {byteAt}=projectedOffsets(source), tree=treeFor(state);
        out({kind:'input',key,name,typed,mode,bare,source,selection:selectionData(state.selection,source),
            reported:[byteAt(from),byteAt(to)], starting:[-1,1].map(side=>id(facets,topAt(tree,from,side).type.prop(languageDataProp))),
            offered:state.readOnly ? [] : offered, built:state.readOnly ? 0 : built, batches:accepted,
            ...(state.readOnly ? {forcedOffer:{offered,built,batches:traced}} : {})});
    }
    for (const key of ['jinja','liquid']) {
        input(key,'percent: empty main in braces','{§}','%');
        input(key,'percent: empty main not in braces','a§b','%');
        input(key,'percent: nonempty main in braces','{§x§}','%', 'normal',0,true);
        input(key,'percent: pair at secondary does not enable main','a§b {§}','%');
        input(key,'percent: reported insertion gate differs from selection','a§b{§}','%', 'reported');
        input(key,'percent: reported insertion still replaces a nonempty main','§él§{§}','%', 'reported',0,true);
        input(key,'percent: absent support fallback','{§}','%', 'normal',0,false,true);
        input(key,'percent: Unicode CRLF multicaret, main is second, reverse secondary','😀§\r\n{§} §élé§','%', 'effect',1,true);
        input(key,'percent: readonly gate vs raw handler','{§}','%', 'readonly');
        input(key,'percent: template string is not a gate','{{ "{§}" }}','%');
        input(key,'percent: composing in braces','{§}','%', 'compose');
        input(key,'percent: composing fallback','a§b','%', 'compose');
        input(key,'percent: blocked pair stays handled','{§}','%', 'protect');
        input(key,'percent: ordinary fallback reconfigures','a§b','%', 'detach');
        input(key,'percent: handled pair reconfigures','{§}','%', 'detach');
        input(key,'greater-than: default with effect','😀\r\n{{ a }}<section§','>', 'effect');
        input(key,'greater-than: blocked close preserves accepted default','😀\r\n{{ a }}<section§','>', 'protect-close');
        input(key,'greater-than: unfinished stock raw script body','{{ a }}<script§','>');
        input(key,'greater-than: unfinished unknown MIME stays HTML','{{ a }}<script type="text/plain"§','>');
        input(key,'slash: default then replace existing angle','<section>{{ a }}<§>tail','/');
        input(key,'greater-than: reads accepted rewrite','<old§','>', 'rewrite');
        input(key,'greater-than: accepted default omitted typed char','<old§','>', 'omit');
        input(key,'greater-than: composition gate','<section§','>', 'compose');
        input(key,'greater-than: default detaches syntax','<section§','>', 'detach');
        input(key,'greater-than: no inherited JSX closer','<script type="text/jsx">const v = <Widget§;</script>','>');
        input(key,'greater-than: gate in template string','{{ "<em§" }}','>');
        input(key,'greater-than: upstream recovery in missing interpolation','{{ "<em§','>');
        input(key,'greater-than: preserve angle beyond gap','<div§{{ a }}>tail','>');
        input(key,'greater-than: source name may span protected gap','<d{{ a }}iv§','>');
        input(key,'slash: no synthetic adjacency across gap','<div><{{ a }}§tail','/');
    }
} finally {
    rmSync(scratch, {recursive:true,force:true});
}
