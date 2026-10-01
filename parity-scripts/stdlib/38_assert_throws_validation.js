// assert.throws/doesNotThrow/rejects/doesNotReject validate the error argument
// as node's expectsError/expectedException do (class, RegExp, validation object
// with the Comparison diff, validation function), and assert.match reports
// node's multi-line message. The error argument used to be ignored.
const assert = require('assert');
const show = (f) => { try { f(); console.log('ok') } catch (e) { console.log(JSON.stringify(e.message), e.code, e.name, e.generatedMessage, e.operator, typeof e.actual, e.expected === undefined ? 'undef' : typeof e.expected) } };
class MyErr extends Error {}
show(() => assert.throws(() => { throw new TypeError('t') }, RangeError));
show(() => assert.throws(() => { throw new TypeError('t') }, TypeError));
show(() => assert.throws(() => { throw new TypeError('t') }, Error));
show(() => assert.throws(() => { throw new Error('abc') }, /x/));
show(() => assert.throws(() => { throw new Error('abc') }, /abc/));
show(() => assert.throws(() => { throw new Error('abc') }, { message: 'nope' }));
show(() => assert.throws(() => { throw new Error('abc') }, { message: 'abc', name: 'Error' }));
show(() => assert.throws(() => { throw new Error('abc') }, { message: /b/ }));
show(() => assert.throws(() => { const e = new Error('abc'); e.code = 'E1'; throw e }, { code: 'E2', message: 'abc' }));
show(() => assert.throws(() => { throw new Error('abc') }, { message: 'nope' }, 'custom'));
show(() => assert.throws(() => { throw new Error('abc') }, new Error('abc')));
show(() => assert.throws(() => { throw new Error('abc') }, new TypeError('abc')));
show(() => assert.throws(() => { throw new Error('abc') }, {}));
show(() => assert.throws(() => { throw 5 }, RangeError));
show(() => assert.throws(() => { throw 5 }, { a: 1 }));
show(() => assert.throws(() => { throw 'str' }, /x/));
show(() => assert.throws(() => { throw new TypeError('t') }, (e) => false));
show(() => assert.throws(() => { throw new TypeError('t') }, function check(e) { return 'yes' }));
show(() => assert.throws(() => { throw new TypeError('t') }, (e) => e instanceof TypeError));
show(() => assert.throws(() => { throw new Error('t') }, MyErr));
show(() => assert.throws(() => { throw new (class MyErr extends Error {})('t') }, MyErr));
show(() => assert.throws(() => {}, TypeError));
show(() => assert.throws(() => {}, TypeError, 'msg'));
show(() => assert.throws(() => {}, 'just a message'));
show(() => assert.throws(() => {}));
show(() => assert.throws(() => { throw new Error('same') }, 'same'));
show(() => assert.throws(() => { throw new Error('x') }, 'str', 'msg'));
show(() => assert.throws(() => { throw new Error('x') }, 5));
show(() => assert.throws('notfn'));
show(() => assert.doesNotThrow(() => { throw new Error('x') }));
show(() => assert.doesNotThrow(() => { throw new Error('x') }, 'mm'));
show(() => assert.doesNotThrow(() => { throw new Error('x') }, TypeError));
show(() => assert.doesNotThrow(() => { throw new Error('x') }, /x/));
show(() => assert.doesNotThrow(() => { throw new Error('x') }, /y/));
show(() => assert.doesNotThrow(() => {}));
show(() => assert.match('abc', /x/));
show(() => assert.match(5, /x/));
show(() => assert.doesNotMatch('axb', /x/));
show(() => assert.match('abc', /x/, 'mine'));
show(() => assert.match('abc', 'x'));
(async () => {
  const sa = async (p) => { try { await p; console.log('ok') } catch (e) { console.log(JSON.stringify(e.message), e.code, e.name, e.generatedMessage) } };
  await sa(assert.rejects(Promise.reject(new TypeError('t')), RangeError));
  await sa(assert.rejects(Promise.reject(new TypeError('t')), TypeError));
  await sa(assert.rejects(async () => { throw new Error('abc') }, { message: 'abc' }));
  await sa(assert.rejects(Promise.resolve(1), Error));
  await sa(assert.rejects(Promise.resolve(1)));
  await sa(assert.doesNotReject(Promise.reject(new Error('x'))));
  await sa(assert.doesNotReject(Promise.reject(new Error('x')), TypeError));
  await sa(assert.doesNotReject(Promise.resolve(2)));
})();
