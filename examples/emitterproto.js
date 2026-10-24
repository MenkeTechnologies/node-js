// EventEmitter.prototype's members read back (they were undefined while the
// key listing named them), in node's order with its three field defaults, as
// ONE object whichever is touched first — the prototype or an emitter — and an
// emitter-backed native (a ChildProcess, a net.Server) inherits its emitter
// methods from it: `child.on === EventEmitter.prototype.on`. A Readable-based
// one (net.Socket) overrides on/addListener/removeListener/off, as in node
// (measured: EMITTER_OVERRIDES).
const EventEmitter = require('events');
const util = require('util');
const P = EventEmitter.prototype;
console.log(Object.getOwnPropertyNames(P).join(','));
console.log(Object.keys(P).join(','));
console.log(typeof P.on, P.on === P.addListener, P.off === P.removeListener, util.inspect(P.on), util.inspect(P.off));
console.log(['emit', 'on', 'once', 'listenerCount', 'eventNames', 'setMaxListeners'].map((k) => `${k}/${P[k].length}`).join(' '));
console.log(JSON.stringify(Object.getOwnPropertyDescriptor(P, '_eventsCount')), P._events, P._maxListeners);
const e = new EventEmitter();
console.log(Object.getPrototypeOf(e) === P, e.on === P.on, e.emit === P.emit);
class X extends EventEmitter {}
console.log(new X().once === P.once);
const net = require('net');
const child = require('child_process').spawn('true');
const server = net.createServer();
const socket = new net.Socket();
for (const [name, o] of [['child', child], ['server', server], ['socket', socket]]) {
  console.log(name, ['on', 'addListener', 'emit', 'once', 'removeListener', 'listenerCount'].map((m) => o[m] === P[m]).join(','), o instanceof EventEmitter);
  let got = 0;
  P.on.call(o, 'zz', () => got++);
  o.emit('zz');
  P.emit.call(o, 'zz');
  console.log(' ', got, o.listenerCount('zz'), P.listenerCount.call(o, 'zz'));
}
console.log(Object.getPrototypeOf(Object.getPrototypeOf(child)) === P);
// ChildProcess.prototype owns none of the emitter methods: they are inherited.
console.log(Object.getOwnPropertyNames(Object.getPrototypeOf(child)).filter((k) => k in P && k !== 'constructor'));
server.close();
