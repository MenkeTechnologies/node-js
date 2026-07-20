// Promise.all / allSettled / race / any with fixed, already-resolved inputs.
async function main() {
  const all = await Promise.all([1, Promise.resolve(2), 3]);
  console.log('all=' + JSON.stringify(all));

  const settled = await Promise.allSettled([
    Promise.resolve('ok'),
    Promise.reject(new Error('bad')),
  ]);
  console.log('settled=' + JSON.stringify(settled.map((s) =>
    s.status === 'fulfilled' ? ['f', s.value] : ['r', s.reason.message])));

  const race = await Promise.race([Promise.resolve('first'), 'second']);
  console.log('race=' + race);

  const any = await Promise.any([Promise.reject(new Error('x')), Promise.resolve('winner')]);
  console.log('any=' + any);

  try {
    await Promise.any([Promise.reject(new Error('a')), Promise.reject(new Error('b'))]);
  } catch (e) {
    console.log('any-agg=' + e.errors.map((x) => x.message).join(','));
  }
}
main();
