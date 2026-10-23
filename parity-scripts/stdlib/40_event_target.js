// Event / EventTarget / CustomEvent globals, and AbortSignal as an EventTarget.
// Deterministic output only: timeStamp is never printed, and no listener here
// is nullish (that path writes a warning to stderr, outside this corpus).
const et = new EventTarget();
console.log(Object.prototype.toString.call(et), et, Object.keys(et), typeof Event, typeof CustomEvent);
console.log(Event.NONE, Event.CAPTURING_PHASE, Event.AT_TARGET, Event.BUBBLING_PHASE);

// Registration order, duplicate suppression, stopImmediatePropagation.
const log = [];
const h1 = e => log.push(['h1', e.type, e.target === et, e.currentTarget === et, e.eventPhase,
  e.bubbles, e.cancelable, e.defaultPrevented, e.composed, e.isTrusted, typeof e.timeStamp,
  e.returnValue, e.cancelBubble, e.composedPath().length]);
et.addEventListener('x', h1);
et.addEventListener('x', h1);
et.addEventListener('x', e => { log.push('h2'); e.stopImmediatePropagation(); });
et.addEventListener('x', () => log.push('h3-never'));
const ev = new Event('x');
console.log(et.dispatchEvent(ev), JSON.stringify(log), ev.eventPhase, ev.currentTarget, ev.target === et, ev.composedPath().length);

// once, handleEvent objects, removeEventListener, capture flag identity.
log.length = 0;
et.addEventListener('y', () => log.push('once'), { once: true });
et.addEventListener('y', { handleEvent(e) { log.push('obj ' + e.type + ' ' + (this.handleEvent !== undefined)); } });
et.addEventListener('y', { noHandleEvent: true });
et.dispatchEvent(new Event('y'));
et.dispatchEvent(new Event('y'));
console.log(JSON.stringify(log));
log.length = 0;
et.removeEventListener('x', h1);
et.dispatchEvent(new Event('x'));
console.log(JSON.stringify(log), et.dispatchEvent(new Event('nobody')));
log.length = 0;
const f = () => log.push('f');
et.addEventListener('z', f, { once: true });
et.addEventListener('z', f);
et.dispatchEvent(new Event('z'));
et.dispatchEvent(new Event('z'));
et.addEventListener('capt', () => log.push('capt'), true);
et.addEventListener('capt', () => log.push('capt2'), { capture: true });
et.addEventListener('capt', () => log.push('capt3'));
et.removeEventListener('capt', () => {});
et.dispatchEvent(new Event('capt'));
console.log(JSON.stringify(log));

// A listener added DURING dispatch does not run in that dispatch.
let n = 0;
et.addEventListener('a', () => { n++; et.addEventListener('a', () => (n += 10)); });
et.dispatchEvent(new Event('a'));
console.log(n);

// preventDefault / cancelable / returnValue / cancelBubble / initEvent.
const c = new Event('c', { cancelable: true, bubbles: true });
et.addEventListener('c', e => e.preventDefault());
console.log(et.dispatchEvent(c), c.defaultPrevented, c.returnValue, c.bubbles);
const nc = new Event('c');
console.log(et.dispatchEvent(nc), nc.defaultPrevented, nc.returnValue);
const rv = new Event('s', { cancelable: true });
rv.returnValue = false;
console.log(rv.defaultPrevented, rv.returnValue);
rv.cancelBubble = true;
console.log(rv.cancelBubble);
const ie = new Event('s');
ie.initEvent('t', true, true);
console.log(ie.type, ie.bubbles, ie.cancelable, new Event(123).type, typeof new Event(123).type, new Event('u').target, new Event('u').srcElement);

// Argument validation.
try { new Event(); } catch (e) { console.log(e.name, e.code, e.message); }
try { et.dispatchEvent({}); } catch (e) { console.log(e.name, e.code, e.message); }
try { et.dispatchEvent(); } catch (e) { console.log(e.name, e.code, e.message); }
try { et.addEventListener('q', 5); } catch (e) { console.log(e.name, e.code, e.message); }
try { et.addEventListener('q'); } catch (e) { console.log(e.name, e.code, e.message); }
try { et.removeEventListener('q'); } catch (e) { console.log(e.name, e.code, e.message); }
try { Event.prototype.preventDefault.call({}); } catch (e) { console.log(e.name, e.code, e.message); }
try { EventTarget.prototype.dispatchEvent.call({}, new Event('x')); } catch (e) { console.log(e.name, e.code, e.message); }
try { Event(); } catch (e) { console.log(e.name, e.message); }
try { EventTarget(); } catch (e) { console.log(e.name, e.message); }
const reent = new Event('r');
et.addEventListener('r', () => { try { et.dispatchEvent(reent); } catch (e) { log.push(e.name + ' ' + e.code + ': ' + e.message); } });
log.length = 0;
et.dispatchEvent(reent);
console.log(JSON.stringify(log));

// CustomEvent.
const ce = new CustomEvent('c', { detail: { a: 1 } });
console.log(ce.detail, new CustomEvent('c').detail, ce instanceof Event, Object.prototype.toString.call(ce), Object.getOwnPropertyNames(CustomEvent.prototype));

// Subclasses, declared before any base instance exists.
class MyT extends EventTarget { constructor() { super(); this.v = 1; } }
class MyE extends Event { constructor() { super('mine'); this.extra = 2; } }
const m = new MyT();
m.addEventListener('k', e => console.log('sub', e.type, e.target === m, e.target.v, e instanceof Event));
m.dispatchEvent(new Event('k'));
m.addEventListener('mine', e => console.log(e.extra, e instanceof MyE, e instanceof Event, e.type));
m.dispatchEvent(new MyE());
console.log(m instanceof EventTarget, m, m.constructor.name, Object.keys(m), JSON.stringify(m));

// AbortSignal is an EventTarget: listeners get the abort Event; `onabort` keeps
// the position of its FIRST assignment; `{ signal }` removes a listener.
const sig = AbortSignal.abort('why');
console.log(sig.aborted, sig.reason, sig instanceof EventTarget, sig instanceof AbortSignal);
const ac = new AbortController();
ac.signal.onabort = 5;
console.log(ac.signal.onabort);
ac.signal.onabort = () => console.log('fn ran');
ac.signal.addEventListener('abort', function (e) { console.log('listener', e.type, e.target === ac.signal, this === ac.signal, e instanceof Event, ac.signal.aborted, ac.signal.reason.name); }, { once: true });
ac.signal.onabort = function (e) { console.log('onabort', e.type, e.target === ac.signal, this === ac.signal); };
ac.abort();
ac.abort();
console.log(ac.signal.onabort !== null);
const ac2 = new AbortController();
et.addEventListener('s', () => log.push('signal-listener'), { signal: ac2.signal });
et.addEventListener('s', () => log.push('plain-listener'));
ac2.abort();
log.length = 0;
et.dispatchEvent(new Event('s'));
console.log(JSON.stringify(log));
et.addEventListener('s2', () => log.push('never'), { signal: AbortSignal.abort() });
log.length = 0;
et.dispatchEvent(new Event('s2'));
console.log(JSON.stringify(log));
class AC extends AbortController { constructor() { super(); this.z = 3; } }
const sub = new AC();
console.log(sub.signal.aborted, sub.z, sub instanceof AbortController, sub);
sub.abort();
console.log(sub.signal.aborted);

// events.addAbortListener and util.aborted resolve through the same listener list.
try { util.aborted(new AbortController().signal); } catch (e) { console.log(e.name, e.code, e.message); }
const { addAbortListener } = require('events');
const util = require('util');
const ac3 = new AbortController();
addAbortListener(ac3.signal, e => console.log('addAbortListener', e.type));
util.aborted(ac3.signal, {}).then(v => console.log('util.aborted resolved with', v instanceof Event, v.type));
ac3.abort();

// A throwing listener does not stop the dispatch or reach the caller.
process.on('uncaughtException', e => console.log('uncaught:', e.message));
const thrower = new EventTarget();
thrower.addEventListener('t', () => { throw new Error('in listener'); });
thrower.addEventListener('t', () => console.log('second still runs'));
console.log('dispatch returns', thrower.dispatchEvent(new Event('t')));
setTimeout(() => console.log('after'), 0);
