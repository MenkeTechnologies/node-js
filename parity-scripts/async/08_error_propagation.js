// Async error propagation: throw crosses await boundaries and rejects.
async function level3() {
  throw new TypeError('deep failure');
}
async function level2() {
  return level3();
}
async function level1() {
  await level2();
}

async function main() {
  try {
    await level1();
  } catch (e) {
    console.log('name=' + e.constructor.name);
    console.log('message=' + e.message);
  }

  const rejected = level1().then(
    () => 'ok',
    (e) => 'rejected:' + e.message,
  );
  console.log('handler=' + (await rejected));
}
main();
