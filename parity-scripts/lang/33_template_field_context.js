// A template literal's `${…}` field parses in its enclosing context: `await`
// and `yield` stay operators in an async or generator body, and a private
// name inside it resolves against the enclosing class (and a nested one).
async function f(x) { return `${await x}!` }
f(Promise.resolve(2)).then(v => console.log(v));
function* g() { const s = `<${yield 1}>`; return s }
const it = g(); it.next(); console.log(it.next('x'));
class T { #c; static #n = 0; constructor(c) { this.#c = c; T.#n++ } toString() { return `${this.#c}C of ${T.#n}` } inner() { return class { m(o) { return `${o.#c}` } } } }
const t = new T(5); console.log(`${t}`, new (t.inner())().m(t));
console.log(`${`${1 + 1}`}`, `a${[1, 2].map(x => `${x}`).join()}b`);
async function* ag() { yield `${await 3}`; } (async () => { for await (const v of ag()) console.log(v) })();
try { eval('`${this.#q}`') } catch (e) { console.log(e.name) }
