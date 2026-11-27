import assert from 'node:assert/strict';

// Map positions in a default CodeMirror document to its original UTF-8 bytes.
// Reject positions inside a supplementary character.
export function projectedOffsets(source) {
  const offsets = [0];
  let byte = 0;
  for (const [part] of source.matchAll(/\r\n|[^]/gu)) {
    if (part.length === 2 && part !== '\r\n') offsets.push(null);
    offsets.push(byte += Buffer.byteLength(part));
  }
  return {units: offsets.length - 1, byteAt(unit) {
    const at = offsets[unit];
    assert(Number.isSafeInteger(at), 'expected an original UTF-8 boundary');
    return at;
  }};
}

// Inputs in these fixtures do not insert line breaks. Keep the original breaks
// outside the modified ranges rather than applying a new separator convention.
export function projectInput(source, transaction) {
  const {byteAt} = projectedOffsets(source);
  const changes = [];
  transaction.changes.iterChanges((from, to, _fromB, _toB, insert) => {
    assert(!insert.toString().includes('\n'));
    changes.push([byteAt(from), byteAt(to), insert.toString()]);
  });
  let result = Buffer.from(source);
  for (const [from, to, insert] of changes.reverse()) {
    result = Buffer.concat([result.subarray(0, from), Buffer.from(insert), result.subarray(to)]);
  }
  const projected = result.toString();
  assert.equal(projected.replace(/\r\n?/g, '\n'), transaction.newDoc.toString());
  return projected;
}
