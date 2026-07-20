// timers/promises setTimeout(ms, value) resolves to the passed value.
const { setTimeout: delay } = require('timers/promises');

async function main() {
  const a = await delay(0, 'alpha');
  console.log('a=' + a);

  const b = await delay(1, 'beta').then((v) => v.toUpperCase());
  console.log('b=' + b);

  const values = await Promise.all([
    delay(0, 10),
    delay(0, 20),
    delay(0, 30),
  ]);
  console.log('sum=' + values.reduce((x, y) => x + y, 0));
}
main();
