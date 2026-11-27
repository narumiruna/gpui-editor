// Replay with Node and the directory of SHA-512-pinned archives.
import assert from 'node:assert/strict';
import {mkdtempSync, realpathSync, rmSync} from 'node:fs';
import {tmpdir} from 'node:os';
import {join, resolve} from 'node:path';
import {pinnedModules} from './pinned_modules.mjs';

const scratch = mkdtempSync(join(realpathSync(tmpdir()), 'gpui-html-bracket-options.'));
try {
  const {load} = pinnedModules(resolve(process.argv[2]), scratch);
  const {EditorState} = await load('@codemirror/state');
  const {EditorView, ViewPlugin} = await load('@codemirror/view');
  const {bracketMatching, forceParsing, syntaxTree} = await load('@codemirror/language');
  const {html} = await load('@codemirror/lang-html');
  const {javascriptLanguage} = await load('@codemirror/lang-javascript');

  const custom = {brackets: '«»', maxScanDistance: 1};
  const beforeOnly = {...custom, afterCursor: false};
  const defaultPairs = {brackets: '()[]{}'};
  const nested = {nestedLanguages: [{tag: 'x-body', parser: javascriptLanguage.parser}],
    nestedAttributes: [{name: 'data-code', parser: javascriptLanguage.parser}]};
  // These stock-plugin observations document differences from native policy.
  const policy = new Set(['attr-start-right-mount', 'mount-eof-at-exact-cap',
    'two-scripts-upstream-crosses', 'two-event-attrs-upstream-crosses', 'supplementary-native-only']);
  const all = process.argv.includes('--all');

  function probe(name, markedSource, options, htmlOptions = {}) {
    const at = markedSource.indexOf('|');
    assert(at >= 0 && at === markedSource.lastIndexOf('|'));
    const source = markedSource.slice(0, at) + markedSource.slice(at + 1);
    const pos = source.slice(0, at).replace(/\r\n?/g, '\n').length;
    const bytes = [0];
    let offset = 0;
    for (const ch of source.match(/\r\n|[^]/gu)) {
      if (ch.length === 2 && ch !== '\r\n') bytes.push(null);
      bytes.push(offset += Buffer.byteLength(ch));
    }
    const matching = bracketMatching(options);
    const plugin = matching.flat(Infinity).find(ext => ext instanceof ViewPlugin);
    assert(plugin);
    const view = {
      state: EditorState.create({doc: source, selection: {anchor: pos}, extensions: [html(htmlOptions), matching]}),
      dispatch(spec) { this.state = this.state.update(spec).state; },
    };
    assert(forceParsing(view, view.state.doc.length, 5000));
    const instance = plugin.create(view);
    assert(view.state.facet(EditorView.decorations).some(set => typeof set === 'function' &&
      set({plugin: selected => selected === plugin ? instance : null}) === instance.decorations));
    const marks = [];
    for (let mark = instance.decorations.iter(); mark.value; mark.next()) {
      const name = mark.value.spec.class;
      assert(name === 'cm-matchingBracket' || name === 'cm-nonmatchingBracket');
      marks.push([bytes[mark.from], bytes[mark.to], name === 'cm-matchingBracket' ? 'M' : 'X']);
    }
    const sides = [-1, 1].map(dir => syntaxTree(view.state).resolveInner(pos, dir).name);
    if (all || !policy.has(name)) console.log(JSON.stringify({name, source, head: bytes[pos], options, marks, sides}));
  }

  probe('host-open-name', '😀\r\n<scr|ipt>(1)</script>', custom);
  probe('host-close-name', '😀\r\n<script>(1)</scr|ipt>', custom);
  probe('host-open-punctuation', '😀\r\n<|style>a{color:red}</style>', custom);
  probe('host-close-punctuation', '😀\r\n<style>a{color:red}</|style>', custom);
  probe('unmounted-script-mouth', '<script type="text/plain">|«😀»</script>', custom);
  probe('unmounted-script-text', '<script type="text/plain">«|😀»</script>', beforeOnly);
  probe('script-body-custom', '<script>/*|«😀»*/</script>', custom);
  probe('script-body-default-removed', '<script>/*|(😀)*/</script>', custom);
  probe('style-body-custom', `<style>a{--v:'|«😀»'}</style>`, custom);
  probe('style-attribute-custom', `<p style="--v:'|«😀»'"></p>`, custom);
  probe('event-attribute-custom', `<p onclick="f('|«😀»')"></p>`, custom);
  probe('style-grammar-precedence', '<style>a|{color:red}</style>', custom);
  probe('event-attribute-grammar-precedence', '<p onclick="f|(x)"></p>', custom);
  probe('typed-script-angle', '<script lang="ts">function f|<\r\nT\r>(t: T) {}</script>', custom);
  probe('json-like-script-array', '<script type="application/ld+json">{"x":|[0]}</script>', custom);
  probe('json-like-script-string', '<script type="application/ld+json">{"x":"|«😀»"}</script>', custom);
  probe('script-start-left-host', '<script>|(1)</script>', beforeOnly);
  probe('script-end-left-mount', '<script>(1)|</script>', beforeOnly);
  probe('script-end-right-host', '<script>0|</script>', custom);
  probe('script-end-no-right', '<script>0|</script>', beforeOnly);
  probe('script-end-left-unmatched', '<script>//«|</script>', {brackets: '«»<>'});
  probe('style-start-left-host', '<style>|a{color:red}</style>', custom);
  probe('style-end-left-mount', '<style>a{color:red}|</style>', custom);
  probe('attr-start-right-mount', '<p onclick="|(x)"></p>', custom);
  probe('attr-start-no-right', '<p onclick="|(x)"></p>', beforeOnly);
  probe('attr-end-left-mount', '<p onclick="(x)|"></p>', beforeOnly);
  probe('outside-braces', '{|<script>/*}*/</script>}', defaultPairs);
  probe('outside-custom', '«|<style>/*»*/</style>»', custom);
  probe('host-exact-cap', '<!--(|😀\r\n)-->', {maxScanDistance: 4});
  probe('host-cutoff-cap', '<!--(|😀\r\n)-->', {maxScanDistance: 3});
  probe('script-exact-forward', '<script>/*(|😀\r\n)*/</script>', {maxScanDistance: 4});
  probe('script-cutoff-forward', '<script>/*(|😀\r\n)*/</script>', {maxScanDistance: 3});
  probe('script-exact-backward', '<script>/*(\r\n😀)|*/</script>', {maxScanDistance: 4});
  probe('script-cutoff-backward', '<script>/*(\r\n😀)|*/</script>', {maxScanDistance: 3});
  probe('mount-eof-at-exact-cap', '<script>/*(|😀\r\n</script>', {maxScanDistance: 3});
  probe('mount-before-eof-cutoff', '<script>/*(|😀\r\n</script>', {maxScanDistance: 2});
  probe('two-scripts-upstream-crosses', '<script>/*|«*/</script><script>/*»*/</script>', custom);
  probe('two-event-attrs-upstream-crosses', '<p onclick="/*|«*/" onblur="/*»*/"></p>', custom);
  probe('custom-body-probe', '<x-body>/*|«😀»*/</x-body>', custom, nested);
  probe('custom-attr-probe', '<p data-code="/*|«😀»*/"></p>', custom, nested);
  probe('left-unmatched-wins', '<script>/*)|«x»*/</script>', {brackets: '()«»'});
  probe('left-mismatch-wins', '<script>/*(|«x»]*/</script>', {brackets: '()[]«»'});
  probe('left-cutoff-checks-right', '<script>/*(|«x»\r\n)*/</script>', {brackets: '()«»', maxScanDistance: 1});
  probe('left-cutoff-no-right', '<script>/*(|«x»\r\n)*/</script>', {brackets: '()«»', maxScanDistance: 1, afterCursor: false});
  probe('supplementary-native-only', '<script>/*|🙂x🙃*/</script>', {brackets: '🙂🙃'});
} finally {
  rmSync(scratch, {recursive: true, force: true});
}
