// toJSON receives the property key it was reached by.
console.log(JSON.stringify({ a: { toJSON(k) { return 'K:' + k; } } }));
console.log(JSON.stringify([{ toJSON(k) { return k; } }]));
console.log(JSON.stringify({ toJSON(k) { return JSON.stringify(k); } }));
// toJSON is applied ONCE: the object it returns is serialized as data, so its
// own `toJSON` method is just an (unserializable) function property.
console.log(JSON.stringify({ toJSON() { return { toJSON() { return 1; } }; } }));
// The replacer runs AFTER toJSON, on its result.
console.log(JSON.stringify({ d: { toJSON() { return 'x'; } } }, (k, v) => (v === 'x' ? 'y' : v)));
// A cycle is still reported rather than walked.
const c = {}; c.self = c;
try { JSON.stringify(c); } catch (e) { console.log(e.constructor.name + ': ' + e.message.split('\n')[0]); }
const g = { get a() { return g; } };
try { JSON.stringify(g); } catch (e) { console.log(e.constructor.name + ': ' + e.message.split('\n')[0]); }
