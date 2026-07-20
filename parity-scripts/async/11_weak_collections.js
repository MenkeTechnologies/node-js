// WeakMap / WeakSet: has/get/set/delete on object keys (deterministic booleans).
const k1 = { id: 1 };
const k2 = { id: 2 };
const k3 = { id: 3 };

const wm = new WeakMap();
wm.set(k1, 'one');
wm.set(k2, 'two');

console.log('wm-has-k1=' + wm.has(k1));
console.log('wm-get-k2=' + wm.get(k2));
console.log('wm-has-k3=' + wm.has(k3));
wm.delete(k1);
console.log('wm-has-k1-after=' + wm.has(k1));

const ws = new WeakSet();
ws.add(k1);
ws.add(k2);
console.log('ws-has-k1=' + ws.has(k1));
console.log('ws-has-k3=' + ws.has(k3));
ws.delete(k1);
console.log('ws-has-k1-after=' + ws.has(k1));
