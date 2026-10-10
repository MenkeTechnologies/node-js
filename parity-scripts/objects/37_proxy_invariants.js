const P = (f) => { try { return String(f()); } catch (e) { return e.name + ': ' + e.message; } };
const r1 = Proxy.revocable({ a: 1 }, {});
r1.revoke();
console.log(P(() => r1.proxy()), P(() => new r1.proxy()), P(() => typeof r1.proxy), P(() => Array.isArray(r1.proxy)), P(() => Object.prototype.toString.call(r1.proxy)));
const r2 = Proxy.revocable(function () { return 1; }, {});
console.log(P(() => r2.proxy()));
r2.revoke();
console.log(P(() => r2.proxy()), P(() => new r2.proxy()));
const frozen = new Proxy(Object.freeze({ k: 1 }), {});
console.log(P(() => Object.setPrototypeOf(frozen, {})), P(() => Reflect.setPrototypeOf(frozen, {})), P(() => Reflect.setPrototypeOf(frozen, Object.prototype)));
const log = [];
const h = new Proxy({}, { get(_, trap) { log.push(trap); return undefined; } });
const p = new Proxy({ a: 1 }, h);
for (const k in p) {}
try { p(); } catch (e) {}
try { new p(); } catch (e) {}
console.log(log.join());
log.length = 0;
const pf = new Proxy(function () {}, h);
try { new pf(); } catch (e) {}
console.log(log.join(), JSON.stringify(new Proxy(function () {}, {})), JSON.stringify({ f: new Proxy(function () {}, {}) }));
