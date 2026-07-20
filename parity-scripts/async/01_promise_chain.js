// Promise then/catch/finally chaining with deterministic resolved values.
const log = [];

Promise.resolve(1)
  .then((v) => v + 1)
  .then((v) => v * 10)
  .then((v) => {
    log.push('then=' + v);
    throw new Error('boom');
  })
  .catch((e) => {
    log.push('catch=' + e.message);
    return 'recovered';
  })
  .then((v) => log.push('after-catch=' + v))
  .finally(() => log.push('finally'))
  .then(() => {
    console.log(log.join('\n'));
  });
