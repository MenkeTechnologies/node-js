// GetTemplateObject (13.2.8.4) caches by Parse Node: one SITE evaluated twice
// hands back the same frozen object, which is what lets a tag memoize on its
// strings array instead of re-parsing the template on every call. Two sites
// with identical text are still distinct objects.
function tag(strings, ...values) {
  return JSON.stringify([strings, strings.raw, values]);
}
const idOf = (s) => s;
const site = () => idOf`a${1}b`;
const twin = () => idOf`a${1}b`;
console.log(site() === site(), site() === twin(), site().raw === site().raw);

const once = () => idOf`x`;
console.log(Object.isFrozen(once()), Object.isFrozen(once().raw), Object.isExtensible(once()));
console.log(JSON.stringify(Object.getOwnPropertyDescriptor(once(), '0')));
console.log(JSON.stringify(Object.getOwnPropertyDescriptor(once(), 'raw')));
console.log(JSON.stringify(Object.getOwnPropertyDescriptor(once(), 'length')));

// A tagged template is a call: the tag is evaluated as a MemberExpression and
// the reference's base is the `this` argument.
const holder = {
  label: 'H:',
  render(strings, ...values) { return this.label + strings.join('|') + '<' + values.join(',') + '>'; },
};
console.log(holder.render`a${1}b${2}c`);
console.log(tag`\n\t${'x'}`);
console.log(String.raw`a\nb${1}c`, String.raw({ raw: ['x', 'y', 'z'] }, 1, 2));

// SetIntegrityLevel walks an array's ELEMENTS and its `length`, not only the
// keys of a plain object.
const frozen = Object.freeze(['x', 'y']);
console.log(Object.isFrozen(frozen), Object.isSealed(frozen), Object.isExtensible(frozen));
console.log(JSON.stringify(Object.getOwnPropertyDescriptor(frozen, '0')));
console.log(JSON.stringify(Object.getOwnPropertyDescriptor(frozen, 'length')));
frozen[0] = 'z';
console.log(frozen.join(','), frozen.length);
const sealed = Object.seal(['x']);
console.log(JSON.stringify(Object.getOwnPropertyDescriptor(sealed, '0')));
sealed[0] = 'w';
console.log(sealed.join(','));

// Every scheduling entry point validates its callback synchronously, so a
// caller can catch the rejection.
for (const [label, run] of [
  ['queueMicrotask', () => queueMicrotask(1)],
  ['nextTick', () => process.nextTick('s')],
  ['setTimeout', () => setTimeout(null, 0)],
  ['setImmediate', () => setImmediate({})],
  ['setInterval', () => setInterval([], 1)],
]) {
  try { run(); console.log(label, 'accepted'); }
  catch (e) { console.log(label, e.constructor.name, e.code, e.message); }
}
