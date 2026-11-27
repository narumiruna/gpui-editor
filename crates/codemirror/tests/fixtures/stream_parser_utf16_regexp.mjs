// Verify the tarball against docs/codemirror-upstream.json before extracting it.
// Run with Node 26.10.0 (V8 14.6.202.34-node.34).
// Pass the verified package/dist/index.js as the argument.
import { pathToFileURL } from 'node:url';

const { StringStream } = await import(pathToFileURL(process.argv[2]).href);

const R = String.raw;
const cases = [];
const add = (source, text = '', flags = 'du', steps = [['match']]) => cases.push({source, flags, text, steps});
for (const flags of ['d', 'du', 'dv']) {
  for (const [source, text] of [
    [R`^(?:(?<x>a)|(?<\u0078>b))(?:\k<x>){2}$`, 'bbb'],
    [R`^(?:(?<x>a)|(?<x>b))\k<x>2$`, 'bb2'],
    [R`^(?:(?<x>a)|(?<x>b))\k<x>2$`, 'ba2'],
    [R`^(?:(?<x>a)|(?<x>b)){2}\k<x>$`, 'aba'],
    [R`^(?:(?<x>a)|(?<x>b)){2}\k<x>$`, 'abb'],
    [R`^(?:(?<x>a)|(?<x>b)){2}\k<x>$`, 'baa'],
    [R`^\k<x>(?:(?<x>a)|(?<x>b))$`, 'b'],
    [R`(?:(?<x>a)(?<z>z))|(?<y>y)(?<x>b)`, 'yb'],
    [R`^(?:(?<x>a)|(?<x>b)|c)(?<rest>.)?\k<x>$`, 'c'],
    [R`^(?:(?<x>a)|(?<x>))\k<x>$`, ''],
    [R`^(?:(?<x>a)|(?<x>b\k<x>))$`, 'b'],
    [R`^.(?<=\k<x>(?:(?<x>a)|(?<x>b)))`, 'b'],
    [R`^..(?<=\k<x>(?:(?<x>a)|(?<x>b)))$`, 'bb'],
    [R`^(?!(?<x>a)|(?<x>b))\k<x>c$`, 'c'],
    [R`^(?:(?=(?<x>a))|(?=(?<x>b)))\k<x>$`, 'b'],
    [R`^(?:(?<𐐀>.)(?=!)|(?<\uD801\uDC00>.))\k<\u{10400}>$`, 'bb'],
    [R`^(?<a\u003e)\k<a\u{3e}$`, ''],
    [R`^(?<b\u200c>q)(?<ñ>e)\k<\u0062\u200C>\k<\u{f1}>$`, 'qeqe'],
    [R`^(?=(a))(?:a)(?<x>b)\1\k<x>$`, 'abab'],
    [R`^(?:a)(b)\1$`, 'abb'],
    [R`^\2(?:a)(b)(c)$`, 'abc'],
    [R`^(x)(?:(?<n>a)|(?<n>b))\3\2\1$`, 'xbbx'],
    [R`(?<x>a)(?<x>b)`, 'ab'],
    [R`(?<x>a|(?<x>b))`, 'b'],
    [R`(?:(?<x>a)|(?<x>b))?(?:(?<x>c)|d)`, 'c'],
    [R`(?!(?<x>a))(?<x>b)`, 'b'],
    [R`(?<x>a)[\k]`, 'ak'],
    [R`\k<y>(?<x>a)`, 'a'],
    [R`\k(?<x>a)`, 'ka'],
    [R`(?<\uD801\u{DC00}>a)`, 'a'],
    [R`(?<\u{+61}>a)`, 'a'],
    [R`(?<\u200c>a)`, 'a'],
  ]) add(source, text, flags);
}
for (const [source, text, flags] of [
  [R`^()\11$`, '\t', 'd'],
  [R`^()()()()()()()()()()()\11$`, '', 'd'],
  [R`^(?:a)\1$`, 'a\u0001', 'd'],
  [R`^(?:a)\1$`, 'a\u0001', 'du'],
  [R`^(?:a)(x)\2$`, 'ax\u0002', 'd'],
  [R`^(?:a)(x)\2$`, 'ax\u0002', 'du'],
  [R`^(?:a)(x)\2$`, 'ax\u0002', 'dv'],
  [R`^(?=a)\1a$`, '\u0001a', 'd'],
  [R`^(?=a)\1a$`, '\u0001a', 'du'],
  [R`^()()()()()()()()(?:a)\9$`, 'a9', 'd'],
  [R`^()()()()()()()()(?:a)\9$`, 'a9', 'du'],
  [R`^(?:)\3778\477\899$`, 'ÿ8\'7899', 'd'],
  [R`^\k<\u{2}>[\k]$`, 'k<uu>k', 'd'],
  [R`^[()|\[\]](?<x>a)\k<x>$`, '|aa', 'du'],
  [R`^[[\q{\(\|\)}]&&[\q{\(\|\)}]](?<x>a)\k<x>$`, '(|)aa', 'dv'],
]) add(source, text, flags);
for (const flags of ['dv', 'div']) {
  for (const [source, text] of [
    [R`^([a&b]+)$`, 'a&ba'],
    [R`^([a&]+)$`, 'a&aa'],
    [R`^([&-a]+)$`, '&0a'],
    [R`^([^a&b])$`, 'a'],
    [R`^([^a&b])$`, 'c'],
    [R`^([[a&b]&&[a&c]]+)$`, 'a&a'],
    [R`^([a&&\&])$`, '&'],
    [R`^([\q{a&b}])$`, 'a&b'],
    [R`[a&&&]`, 'a'],
    [R`[a&b&&b]`, 'a'],
    [R`[\q{&&}]`, 'a'],
  ]) add(source, text, flags);
}
add(R`(?:(?<x>a)|(?<x>b))\k<x>`, 'abbbaa', 'dgu', [['setLast',99],['match',false],['match'],['match'],['match']]);
add(R`(?:(?<x>a)|(?<x>b))\k<x>`, 'abbbaa', 'dyu', [['setLast',1],['match',false],['match'],['setPos',1],['match',false],['match']]);
add(R`(?:(?<x>a)|(?<x>b))\k<x>`, 'aabbcc', 'dgyv', [['setLast',2],['match'],['match',false],['setPos',4],['match']]);
add(R`(?:(?<s>🦀)|(?<s>\uDD80))?`, '🦀\uDD80!', 'dg', [['match',false],['match']]);
add(R`(?:(?<s>🦀)|(?<s>\uDD80))?`, '🦀\uDD80!', 'dgu', [['match',false],['match']]);
add(R`(?:(?<s>🦀)|(?<s>\uDD80))?`, '🦀\uDD80!', 'dgv', [['match',false],['match']]);
add(R`(?:(?<s>🦀)|(?<s>\uDD80))`, '🦀\uDD80!', 'dyu', [['setLast',1],['match'],['match'],['match'],['setPos',1],['match']]);
add(R`(?:(?<s>🦀)|(?<s>\uDD80))`, '🦀\uDD80!', 'dgu', [['setLast',1],['test'],['test'],['test'],['test']]);
add(R`(?:(?<s>🦀)|(?<s>[\uD800-\uDFFF]))`, '🦀\uDD80!', 'dgv', [['eatWhile'],['eat'],['eatWhile'],['eat'],['setPos',4],['setLast',34],['eat']]);
add(R`(?:(?<e>)q|(?<e>))`, '🦀', 'yu', [['setLast',1],['test'],['test'],['match'],['setLast',3],['match'],['setLast',-9],['match',false]]);
add(R`(?:(?<x>a)|(?<x>b))\k<x>`, 'bb', '', [['setLast',63],['test'],['match'],['match']]);
for (const flags of ['dv', 'div']) {
  for (const source of [
    R`[^\q{}]`,
    R`[^\q{a|}]`,
    R`[^\q{|a}]`,
    R`[^\q{a||b}]`,
    R`[^\q{a|ab}]`,
    R`[^\q{\u{D83E}\uDD80}]`,
    R`[x[^\q{a|}]]`,
    R`[x[^\q{🦀a}]]`,
    R`[^[\q{a|}]]`,
    R`[^x\q{ab}]`,
    R`[^\q{ab}x]`,
    R`[^\q{a|}&&\q{b|}]`,
    R`[^\q{ab}&&\q{xy}&&\q{z|}]`,
    R`[^\q{a|}--[a]--[b]]`,
    R`[^[\q{a|}&&\q{b|}]&&[\q{ab}--[a]]]`,
    R`[^[x\q{a|}]\&]`,
    R`[^\p{RGI_Emoji}]`,
    R`[^\p{RGI_Emoji}&&\q{ab}]`,
    R`[^\p{RGI_Emoji}--\p{ASCII}]`,
    R`[^[\p{Basic_Emoji}]]`,
  ]) add(source, '', flags);

  for (const [source, samples] of [
    [R`[^\q{a|b}]`, ['a', 'b', 'z', 'A']],
    [R`[^\q{🦀|\uD83E\uDD80|\u{1f980}}]`, ['🦀', 'z']],
    [R`[^\q{\||\}|\\|\u{5b}|\cA|\x22|\0|\b}]`, ['|', '}', '\\', '[', '\x01', '"', '\0', '\b', 'z']],
    [R`[^[\q{a|b}]]`, ['b', 'z']],
    [R`[\q{a|b}&&[b]]`, ['b', 'a']],
    [R`[\q{m|n}--\q{n|o}]`, ['m', 'n']],
    [R`[^\q{\uD83E|A}]`, ['\uD83E', 'A', 'z']],
  ]) {
    if (flags === 'dv') {
      for (const text of samples) add(source, text, flags);
    } else {
      // Case-folded string operands still use the backend implementation.
      add(source, '', flags, []);
    }
  }

  // These valid negations still combine with a string operand in the backend.
  for (const source of [
    R`[^\q{a|}&&[a]]`,
    R`[^\q{ab}&&[a]]`,
    R`[^\q{a|}&&\q{b|}&&[a]]`,
    R`[^[\q{a|}&&[b]]]`,
    R`[^[a]--[\q{a|}--[b]]]`,
    R`[^[\p{Basic_Emoji}]&&\p{ASCII}]`,
    R`[^\p{ASCII}--[\p{Basic_Emoji}]]`,
    R`[^\q{a\&}&&[a]]`,
  ]) add(source, '', flags, []);
  add(R`[^[b]--\q{a|}]`, 'a', flags);
  add(R`[\[^\q{ab}]`, 'ab', flags);
}

for (const [source, text] of [
  [R`^([\q{}])$`, ''],
  [R`^([\q{a|}])$`, ''],
  [R`^([\q{|a||b}])`, 'b'],
  [R`^([\q{a|ab}])`, 'ab'],
  [R`^([\q{a|ab}])b$`, 'ab'],
  [R`^([\q{a\-|a\-b|}])$`, 'a-b'],
  [R`^([\q{🦀|🦀é|e\|x}])`, '🦀é'],
  [R`^([\q{\uD83E\uDD80|\uD83E\uDD80z}])`, '🦀z'],
  [R`^([\q{\uD83E\u{41}|\uD83E\u{41}x}])`, '\uD83EAx'],
  [R`^([\q{a\b|a\}\||\x61\u{5b}}])`, 'a}|'],
  [R`^([\q{\cA|\cAz|\cA\x7f}])`, '\x01z'],
  [R`^([\q{a|ab}])+$`, 'abab'],
  [R`^([\q{a|ab}])+b$`, 'abab'],
  [R`^ab(?<=([\q{a|ab}]))$`, 'ab'],
  [R`^[\[\q{a|ab}]$`, '['],
]) add(source, text, 'dv');
add(R`^(?-i:([\q{a|ab}]))`, 'ab', 'div');
for (const [source, text] of [
  [R`^([\q{k|ke|}])`, 'KE'],
  [R`^([\q{k|ke|}])$`, ''],
  [R`^([\q{ſ|ſs}])`, 'SS'],
  [R`^(?i:([\q{ß|ße}]))`, 'ẞE'],
  [R`^([\q{𐐀|𐐀a}])`, '𐐨A'],
  [R`^([\q{a|ab}])b$`, 'AB'],
  [R`^([\q{K|ſ}])$`, 'k'],
  [R`^([^\q{K|ſ}])$`, 'K'],
  [R`^([^\q{K|ſ}])$`, 's'],
  [R`^([^\q{K|ſ}])$`, 'z'],
  [R`^([^\q{𐐀}])$`, '𐐨'],
  [R`^([^\q{a|b}])$`, 'A'],
  [R`^([^\q{a|b}])$`, 'Z'],
]) add(source, text, 'div');

for (const flags of ['du', 'dv']) {
  for (const [source, text] of [
    [R`^x(?i:(a)\1)y$`, 'xaAy'],
    [R`^x(?i:(a)\1)y$`, 'XaAy'],
    [R`^x(?i:(a)\1)y$`, 'xaAY'],
    [R`^x(?i:(?<x>a)|(?<x>b))(?i:\k<x>)y$`, 'xbBy'],
    [R`^(a)(?i:\1\1)$`, 'aAA'],
    [R`^(a)(?i:\1\1)$`, 'AAA'],
    [R`^(?i:([ſ])\1)$`, 'ſS'],
    [R`)(?i:(a)\1)(`, 'aA'],
    [R`^(?i:(?<x>[\q{a|ab|}])\k<x>)$`, 'abab'],
    [R`^(?i:(?<x>[\q{a|ab|}])\k<x>)$`, 'aA'],
  ]) add(source, text, flags);
}
for (const flags of ['diu', 'div']) {
  for (const [source, text] of [
    [R`^(?-i:(?<word>a)\k<word>)z$`, 'aaZ'],
    [R`^(?-i:(?<word>a)\k<word>)z$`, 'aAZ'],
    [R`^(?-i:(?<x>a)|(?<x>b))(?-i:\k<x>)$`, 'bB'],
    [R`^(?-i:(?<x>a)|(?<x>b))(?-i:\k<x>)$`, 'bb'],
    [R`^(a)(?-i:\1)$`, 'Aa'],
    [R`^(a)(?-i:\1)$`, 'AA'],
  ]) add(source, text, flags);
}

for (const flags of ['d', 'du', 'dv', 'di', 'diu', 'div']) {
  for (const [source, text] of [
    [R`^(a)(?i:\1)(?-i:\1)$`, 'aAa'],
    [R`^(a)(?i:\1)(?-i:\1)$`, 'aAA'],
    [R`^(?<word>a)(?-i:\k<word>)(?i:\k<word>)$`, 'aaA'],
    [R`^(?<word>a)(?-i:\k<word>)(?i:\k<word>)$`, 'aAa'],
    [R`^(?i:(a)\1)(?-i:(b)\2)$`, 'aAbb'],
    [R`^(?i:(a)\1)(?-i:(b)\2)$`, 'aAbB'],
  ]) add(source, text, flags);
}

// A simple ASCII letter range must not gain non-ASCII members in legacy i mode.
// These rows also match Headless Chromium 159.0.8154.6 (V8 15.9.128.1).
for (const [source, flags, text, steps] of [
  [R`([a-z]+)`, 'diy', 'aZſ', [['match',false],['match'],['setLast',0],['match'],['match']]],
  [R`([A-Z]+)`, 'diy', '🦀kK', [['setPos',2],['match',false],['setLast',0],['match']]],
  [R`([^A-Z]+)`, 'diy', 'ſKz'],
  [R`([^a-z]+)`, 'diy', 'Zſ'],
  [R`([A-Z])`, 'diy', 'ı'],
  [R`((?i:[j-m])+)`, 'dy', 'KK'],
  [R`((?i:[J-M])+)`, 'dy', 'JjLlmM'],
  [R`((?-i:[j-m])+)([j-m]+)`, 'diy', 'kKK'],
  [R`(?i:([a-z]))([a-z]+)`, 'dy', 'ZaZ'],
  [R`(?<=([a-z]))(!)`, 'diy', 'ſ!', [['setLast',1],['match']]],
  [R`(?<=([a-z]))(!)`, 'diy', 'S!', [['setLast',1],['match']]],
  [R`([a-z]{1,2})([^a-z])`, 'diy', 'Zſ'],
  [R`([a-z]+)`, 'diyu', 'aZſ'],
  [R`([A-Z]+)`, 'diyv', 'aZK'],
  [R`([z-a])`, 'diy', 'z'],
]) add(source, text, flags, steps);

function units(str) {return str.split('').map(u => u.charCodeAt(0));}
for (const {source, flags, text, steps} of cases) {
  const row = {source, flags, text: text.isWellFormed() ? text : units(text)};
  try {
    const regex = new RegExp(source, flags);
    const stream = new StringStream(text, 4, 2);
    row.steps = steps.map(([op, arg]) => {
      let result;
      switch (op) {
        case 'setLast': regex.lastIndex = arg; return [op, arg];
        case 'setPos': stream.pos = arg; return [op, arg];
        case 'test': result = regex.test(text); break;
        case 'eat': { const eaten = stream.eat(regex); result = eaten == null ? null : units(eaten); break; }
        case 'eatWhile': result = stream.eatWhile(regex); break;
        case 'match': {
          const match = stream.match(regex, arg);
          result = match && (regex.global ? match.map(units) : {
            captures: Array.from(match, cap => cap == null ? null : units(cap)),
            index: match.index,
            input: units(match.input),
            names: Object.entries(match.groups ?? {}).map(([name, cap]) => [name, cap == null ? null : units(cap)]),
            indices: match.indices ?? null,
            namedIndices: match.indices?.groups ? Object.entries(match.indices.groups) : null,
          });
          break;
        }
      }
      return [op, arg ?? null, result, stream.pos, regex.lastIndex];
    });
  } catch (e) {
    if (!(e instanceof SyntaxError)) throw e;
  }
  console.log(JSON.stringify(row));
}
