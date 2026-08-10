// `process.stdout.write` puts BYTES on the wire.
//
// A Buffer/TypedArray chunk goes through untouched — including bytes that are
// not valid UTF-8, which a String round-trip would replace with U+FFFD — and a
// string chunk is decoded with its encoding. `run.sh` compares stdout with
// `cmp`, so a byte difference here is a real failure rather than a rendering
// artifact.
process.stdout.write(Buffer.from([0x41, 0x42, 0x0a]));
process.stdout.write(new Uint8Array([0x43, 0x44, 0x0a]));
process.stdout.write('4546\n', 'hex');
process.stdout.write('R0hJ\n', 'base64');
process.stdout.write('é', 'latin1');
process.stdout.write('\n');

// No trailing newline: two writes must concatenate with nothing between them.
process.stdout.write('a');
process.stdout.write('b');
process.stdout.write('\n');

// Ordering with console.log has to interleave correctly (same fd, same stream).
console.log('one');
process.stdout.write('two\n');
console.log('three');

// A chunk that is neither a string nor a byte view is a TypeError.
for (const bad of [[65, 66], 65, undefined, null]) {
  try {
    process.stdout.write(bad);
    console.log('no throw');
  } catch (e) {
    console.log(e.constructor.name, e.message);
  }
}
