// [[Set]] on a trapless Proxy runs OrdinarySet with the PROXY as receiver, so
// getOwnPropertyDescriptor/defineProperty traps observe every assignment; a
// set trap inherited through Object.create(proxy) fires with the child.
for (const target of [{a: 1}, [1, 2]]) {
  const log = [];
  const H = {
    getOwnPropertyDescriptor(t, k) { log.push('gopd:' + String(k)); return Reflect.getOwnPropertyDescriptor(t, k); },
    defineProperty(t, k, d) { log.push('def:' + String(k) + ':' + JSON.stringify(d)); return Reflect.defineProperty(t, k, d); },
  };
  const p = new Proxy(target, H);
  p.a = 5; p.newKey = 1; p[0] = 9;
  console.log(log.join(' '), JSON.stringify(target));
}
const log2 = [];
const p2 = new Proxy({}, { defineProperty(t, k, d) { log2.push(k); return false; } });
p2.x = 1; console.log(log2, Object.keys(p2));
(function () { 'use strict'; try { p2.y = 2; } catch (e) { console.log(e.constructor.name, e.message); } })();
const child = Object.create(new Proxy({}, { set(t, k, v, r) { console.log('set trap', k, r === child); return Reflect.set(t, k, v, r); } }));
child.z = 3; console.log(Object.keys(child));
