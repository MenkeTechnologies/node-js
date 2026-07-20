// async/await: sequential, parallel via Promise.all await, try/catch, arrow.
const double = async (n) => n * 2;

async function sequential() {
  let sum = 0;
  for (const n of [1, 2, 3, 4]) {
    sum += await double(n);
  }
  return sum;
}

async function parallel() {
  const results = await Promise.all([1, 2, 3].map(double));
  return results.reduce((a, b) => a + b, 0);
}

async function guarded() {
  try {
    await Promise.reject(new Error('nope'));
    return 'unreached';
  } catch (e) {
    return 'caught:' + e.message;
  }
}

async function main() {
  console.log('sequential=' + (await sequential()));
  console.log('parallel=' + (await parallel()));
  console.log('guarded=' + (await guarded()));
}
main();
