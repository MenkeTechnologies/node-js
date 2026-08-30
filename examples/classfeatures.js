// Modern class syntax and the generator protocols: private fields, methods and
// getters, `#x in obj` brand checks, private statics, static blocks, generator
// delegation with `yield*`, `return()`/`throw()` resumption and the `finally`
// they must run, plus async generators and `for await`.
class C {
  #n = 1;
  static #count = 0;
  static registry = [];
  static { C.registry.push('static-block'); }
  #priv() { return 'privMethod'; }
  get #hidden() { return 'privGetter'; }
  constructor() { C.#count++; }
  read() { return [this.#n, this.#priv(), this.#hidden].join('|'); }
  static count() { return C.#count; }
  static has(o) { return #n in o; }
}
const c = new C();
console.log('private ', c.read());
console.log('statics ', C.count(), C.registry.join(','), C.has(c), C.has({}));

// Generator delegation, return() and throw().
function* inner() { try { yield 'a'; yield 'b'; } finally { console.log('inner-finally'); } }
function* outer() { yield* inner(); yield 'c'; }
const g = outer();
console.log('gen     ', g.next().value, g.next().value);
console.log('genRet  ', JSON.stringify(g.return('early')));

function* thrower() { try { yield 1; } catch (e) { yield 'caught:' + e; } }
const t = thrower();
t.next();
console.log('genThrow', t.throw('boom').value);

// Async generators and for-await.
(async () => {
  async function* ag() { yield 1; yield 2; }
  const seen = [];
  for await (const v of ag()) seen.push(v);
  console.log('asyncGen', seen.join(','));
})();
