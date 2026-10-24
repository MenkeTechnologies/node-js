// The WHATWG Streams API — `ReadableStream`, `WritableStream`,
// `TransformStream`, their readers, writers and controllers, the queuing
// strategies, `TextEncoderStream`/`TextDecoderStream` and
// `CompressionStream`/`DecompressionStream` — ported from node v26.12.1's
// `lib/internal/webstreams/` (MIT): `util.js`, `queuingstrategies.js`,
// `writablestream.js`, `readablestream.js`, `transformstream.js` and
// `encoding.js` are node's files unchanged, each wrapped as a module below.
// Evaluated once by `src/stdlib/stream_web.rs`; the file is a function of
// `nodeError` (`util.@@nodeError`, node's coded errors) and `promiseState`
// (`util.@@promiseState`, V8's `getPromiseDetails`) returning the
// `readablestream`/`writablestream`/`transformstream`/`queuingstrategies`/
// `encoding`/`compression` exports merged into one object.
//
// What this prelude supplies in place of node's internals:
//
// * `primordials` — the intrinsics node's files destructure, captured here
//   before any user code runs. Prototype methods are uncurried; the getters
//   (`TypedArrayPrototypeGetLength`, …) read the accessor directly.
// * `internal/errors` — the seven codes the streams raise, built through
//   `nodeError` so `code`, the bracketed `name [code]` and the class match;
//   `ERR_INVALID_ARG_TYPE`'s expected-type phrase is node's formatter.
// * `internal/validators`, `internal/util`, `internal/util/types`,
//   `internal/streams/utils` (the shared `Symbol.for` keys),
//   `internal/events/abort_listener` and the `internal/webidl` enum
//   converter — node's code, trimmed to what the streams call.
// * The transfer symbols (`kTransfer`, `kDeserialize`) are defined, but no
//   stream is transferable here: `structuredClone`/`postMessage` never call
//   them, and `markTransferMode` is a no-op.
// * `compression.js` builds its streams over node's Duplex adapters
//   (`internal/webstreams/adapters`), which are not ported: the
//   `CompressionStream`/`DecompressionStream` below keep node's class shape,
//   format converter and chunk check, but run the codec over the whole
//   input when the writable closes, so the output arrives as ONE chunk where
//   node's zlib handle may emit several.
(function (nodeError, promiseState) {
'use strict';

const uncurryThis = (fn) => (self, ...args) => Reflect.apply(fn, self, args);

const primordials = {
  __proto__: null,
  Array,
  ArrayBuffer,
  ArrayBufferPrototypeGetByteLength: (buffer) => buffer.byteLength,
  ArrayBufferPrototypeGetDetached: (buffer) => buffer.detached,
  ArrayBufferPrototypeSlice: uncurryThis(ArrayBuffer.prototype.slice),
  ArrayBufferPrototypeTransfer: uncurryThis(ArrayBuffer.prototype.transfer),
  ArrayPrototypePush: uncurryThis(Array.prototype.push),
  AsyncIteratorPrototype: Object.getPrototypeOf(
    Object.getPrototypeOf(async function* () {}).prototype),
  DataView,
  DataViewPrototypeGetBuffer: (view) => view.buffer,
  DataViewPrototypeGetByteLength: (view) => view.byteLength,
  DataViewPrototypeGetByteOffset: (view) => view.byteOffset,
  FunctionPrototypeBind: uncurryThis(Function.prototype.bind),
  FunctionPrototypeCall: (fn, thisArg, ...args) => Reflect.apply(fn, thisArg, args),
  MathMax: Math.max,
  MathMin: Math.min,
  NumberIsInteger: Number.isInteger,
  NumberIsNaN: Number.isNaN,
  ObjectDefineProperties: Object.defineProperties,
  ObjectDefineProperty: Object.defineProperty,
  ObjectFreeze: Object.freeze,
  ObjectSetPrototypeOf: Object.setPrototypeOf,
  Promise,
  PromisePrototypeThen: uncurryThis(Promise.prototype.then),
  PromiseReject: (reason) => Promise.reject(reason),
  PromiseResolve: (value) => Promise.resolve(value),
  PromiseWithResolvers: () => Promise.withResolvers(),
  // node's `SafePromiseAll(promises, mapFn)`.
  SafePromiseAll: (promises, mapFn) =>
    Promise.all(mapFn ? promises.map(mapFn) : promises),
  String,
  StringPrototypeCharCodeAt: uncurryThis(String.prototype.charCodeAt),
  StringPrototypeSlice: uncurryThis(String.prototype.slice),
  Symbol,
  SymbolAsyncIterator: Symbol.asyncIterator,
  SymbolDispose: Symbol.dispose,
  SymbolIterator: Symbol.iterator,
  SymbolToStringTag: Symbol.toStringTag,
  TypedArrayPrototypeGetBuffer: (view) => view.buffer,
  TypedArrayPrototypeGetByteLength: (view) => view.byteLength,
  TypedArrayPrototypeGetByteOffset: (view) => view.byteOffset,
  TypedArrayPrototypeGetLength: (view) => view.length,
  Uint8Array,
};

const globalRequire = require;
const modules = { __proto__: null };
const cache = { __proto__: null };

function define(id, body) {
  modules[id] = body;
}

function requireInternal(id) {
  const cached = cache[id];
  if (cached !== undefined) return cached.exports;
  const body = modules[id];
  if (body === undefined) return globalRequire(id);
  const module = { exports: {} };
  cache[id] = module;
  body(module, module.exports, requireInternal, primordials, internalBinding);
  return module.exports;
}

function internalBinding(name) {
  switch (name) {
    case 'messaging':
      return { DOMException };
    case 'buffer':
      return {
        // `copyArrayBuffer(destination, destinationOffset, source,
        // sourceOffset, bytesToCopy)`.
        copyArrayBuffer(destination, destinationOffset, source, sourceOffset, count) {
          new Uint8Array(destination, destinationOffset, count)
            .set(new Uint8Array(source, sourceOffset, count));
        },
      };
    case 'util':
      return {
        constants: { kPending: 0 },
        getPromiseDetails: (promise) => [promiseState(promise)],
      };
  }
  throw new Error(`internalBinding('${name}') is not available`);
}

// ── internal/errors ─────────────────────────────────────────────────────────

// The error a `nodeError` call throws, returned instead: node constructs
// these with `new` and throws or rejects with them later.
function made(kind, ...args) {
  try {
    nodeError(kind, ...args);
  } catch (err) {
    return err;
  }
  throw new Error('unreachable');
}

// `E(code, '…%s…', Base, ...OtherBases)`: a constructor per class, the
// first one also carrying the others as `.TypeError` / `.RangeError`.
function simpleCode(code, template, ...classes) {
  const build = (Base) => function (...args) {
    let i = 0;
    const message = template.replace(/%s/g, () => String(args[i++]));
    return made(`code:${Base}`, code, message);
  };
  const ctor = build(classes[0]);
  for (const Base of classes.slice(1)) ctor[Base] = build(Base);
  return ctor;
}

const classRegExp = /^[A-Z][a-zA-Z0-9]*$/;
const kTypes = [
  'string',
  'function',
  'number',
  'object',
  'Function',
  'Object',
  'boolean',
  'bigint',
  'symbol',
];

function formatList(array, type = 'and') {
  switch (array.length) {
    case 0: return '';
    case 1: return `${array[0]}`;
    case 2: return `${array[0]} ${type} ${array[1]}`;
    case 3: return `${array[0]}, ${array[1]}, ${type} ${array[2]}`;
    default:
      return `${array.slice(0, -1).join(', ')}, ${type} ${array[array.length - 1]}`;
  }
}

// node's `ERR_INVALID_ARG_TYPE` message builder, up to `. Received …`, which
// `nodeError('type', …)` appends.
function expectedPhrase(expected) {
  if (!Array.isArray(expected)) expected = [expected];
  let msg = '';
  const types = [];
  const instances = [];
  const other = [];
  for (const value of expected) {
    if (kTypes.includes(value)) {
      types.push(value.toLowerCase());
    } else if (classRegExp.exec(value) !== null) {
      instances.push(value);
    } else {
      other.push(value);
    }
  }
  if (instances.length > 0) {
    const pos = types.indexOf('object');
    if (pos !== -1) {
      types.splice(pos, 1);
      instances.push('Object');
    }
  }
  if (types.length > 0) {
    msg += `${types.length > 1 ? 'one of type' : 'of type'} ${formatList(types, 'or')}`;
    if (instances.length > 0 || other.length > 0) msg += ' or ';
  }
  if (instances.length > 0) {
    msg += `an instance of ${formatList(instances, 'or')}`;
    if (other.length > 0) msg += ' or ';
  }
  if (other.length > 0) {
    if (other.length > 1) {
      msg += `one of ${formatList(other, 'or')}`;
    } else {
      if (other[0].toLowerCase() !== other[0]) msg += 'an ';
      msg += `${other[0]}`;
    }
  }
  return msg;
}

function ERR_INVALID_ARG_TYPE(name, expected, actual) {
  return made('type', name, expectedPhrase(expected), actual);
}

function ERR_INVALID_ARG_VALUE(name, value, reason) {
  return made('value', name, value, reason);
}
ERR_INVALID_ARG_VALUE.RangeError = function (name, value, reason) {
  return made('value:RangeError', name, value, reason);
};

function ERR_OUT_OF_RANGE(name, range, input) {
  return made('range', name, range, input);
}

class AbortError extends Error {
  constructor(message = 'The operation was aborted', options = undefined) {
    if (options !== undefined && typeof options !== 'object') {
      throw ERR_INVALID_ARG_TYPE('options', 'Object', options);
    }
    super(message, options);
    this.code = 'ABORT_ERR';
    this.name = 'AbortError';
  }
}

define('internal/errors', (module) => {
  module.exports = {
    AbortError,
    codes: {
      ERR_ARG_NOT_ITERABLE: simpleCode('ERR_ARG_NOT_ITERABLE', '%s must be iterable', 'TypeError'),
      ERR_ILLEGAL_CONSTRUCTOR: simpleCode('ERR_ILLEGAL_CONSTRUCTOR', 'Illegal constructor', 'TypeError'),
      ERR_INVALID_ARG_TYPE,
      ERR_INVALID_ARG_VALUE,
      ERR_INVALID_STATE: simpleCode('ERR_INVALID_STATE', 'Invalid state: %s', 'Error', 'TypeError', 'RangeError'),
      ERR_INVALID_THIS: simpleCode('ERR_INVALID_THIS', 'Value of "this" must be of type %s', 'TypeError'),
      ERR_MISSING_OPTION: simpleCode('ERR_MISSING_OPTION', '%s is required', 'TypeError'),
      ERR_OUT_OF_RANGE,
    },
  };
});

define('internal/assert', (module) => {
  module.exports = function assert(value, message) {
    if (!value) {
      throw made('code:Error', 'ERR_INTERNAL_ASSERTION', `${message ?? 'assertion failed'}`);
    }
  };
});

// ── internal/util, internal/util/types ──────────────────────────────────────

const kEnumerableProperty = { __proto__: null };
kEnumerableProperty.enumerable = true;
Object.freeze(kEnumerableProperty);

define('internal/util', (module) => {
  module.exports = {
    customInspectSymbol: Symbol.for('nodejs.util.inspect.custom'),
    kEmptyObject: Object.freeze({ __proto__: null }),
    kEnumerableProperty,
    SideEffectFreeRegExpPrototypeSymbolReplace: (regexp, string, replacement) =>
      (typeof string === 'string' ? string.replace(regexp, replacement) : string),
  };
});

define('internal/util/types', (module) => {
  const { types } = globalRequire('util');
  module.exports = {
    isArrayBufferView: ArrayBuffer.isView,
    isDataView: types.isDataView,
    // No `SharedArrayBuffer` exists in this runtime.
    isSharedArrayBuffer: () => false,
  };
});

// ── internal/validators ─────────────────────────────────────────────────────

const kValidateObjectNone = 0;
const kValidateObjectAllowNullable = 1 << 0;
const kValidateObjectAllowArray = 1 << 1;
const kValidateObjectAllowFunction = 1 << 2;
const kValidateObjectAllowObjects = kValidateObjectAllowArray |
  kValidateObjectAllowFunction;
const kValidateObjectAllowObjectsAndNull = kValidateObjectAllowNullable |
  kValidateObjectAllowArray |
  kValidateObjectAllowFunction;

const validateObject = (value, name, options = kValidateObjectNone) => {
  if (options === kValidateObjectNone) {
    if (value === null || Array.isArray(value)) {
      throw ERR_INVALID_ARG_TYPE(name, 'Object', value);
    }
    if (typeof value !== 'object') {
      throw ERR_INVALID_ARG_TYPE(name, 'Object', value);
    }
  } else {
    const throwOnNullable = (kValidateObjectAllowNullable & options) === 0;
    if (throwOnNullable && value === null) {
      throw ERR_INVALID_ARG_TYPE(name, 'Object', value);
    }
    const throwOnArray = (kValidateObjectAllowArray & options) === 0;
    if (throwOnArray && Array.isArray(value)) {
      throw ERR_INVALID_ARG_TYPE(name, 'Object', value);
    }
    const throwOnFunction = (kValidateObjectAllowFunction & options) === 0;
    const typeofValue = typeof value;
    if (typeofValue !== 'object' && (throwOnFunction || typeofValue !== 'function')) {
      throw ERR_INVALID_ARG_TYPE(name, 'Object', value);
    }
  }
};

const validateFunction = (value, name) => {
  if (typeof value !== 'function') throw ERR_INVALID_ARG_TYPE(name, 'Function', value);
};

const validateAbortSignal = (signal, name) => {
  if (signal !== undefined &&
      (signal === null ||
       typeof signal !== 'object' ||
       !('aborted' in signal))) {
    throw ERR_INVALID_ARG_TYPE(name, 'AbortSignal', signal);
  }
};

const validateBuffer = (buffer, name = 'buffer') => {
  if (!ArrayBuffer.isView(buffer)) {
    throw ERR_INVALID_ARG_TYPE(name, ['Buffer', 'TypedArray', 'DataView'], buffer);
  }
};

const validateNumber = (value, name, min = undefined, max) => {
  if (typeof value !== 'number') throw ERR_INVALID_ARG_TYPE(name, 'number', value);
  if ((min != null && value < min) || (max != null && value > max) ||
    ((min != null || max != null) && Number.isNaN(value))) {
    throw ERR_OUT_OF_RANGE(
      name,
      `${min != null ? `>= ${min}` : ''}${min != null && max != null ? ' && ' : ''}${max != null ? `<= ${max}` : ''}`,
      value);
  }
};

define('internal/validators', (module) => {
  module.exports = {
    kValidateObjectAllowObjects,
    kValidateObjectAllowObjectsAndNull,
    validateAbortSignal,
    validateBuffer,
    validateFunction,
    validateNumber,
    validateObject,
  };
});

// ── transfer, task queues, stream symbols, abort listener, webidl ──────────

define('internal/worker/js_transferable', (module) => {
  module.exports = {
    kDeserialize: Symbol('messaging_deserialize_symbol'),
    kTransfer: Symbol('messaging_transfer_symbol'),
    kTransferList: Symbol('messaging_transfer_list_symbol'),
    markTransferMode() {},
    structuredClone,
  };
});

define('internal/worker/io', (module) => {
  module.exports = { MessageChannel: globalThis.MessageChannel };
});

define('internal/abort_controller', (module) => {
  module.exports = { AbortController };
});

define('internal/process/task_queues', (module) => {
  module.exports = { queueMicrotask };
});

define('internal/streams/utils', (module) => {
  module.exports = {
    kIsDisturbed: Symbol.for('nodejs.stream.disturbed'),
    kIsErrored: Symbol.for('nodejs.stream.errored'),
    kIsReadable: Symbol.for('nodejs.stream.readable'),
    kIsWritable: Symbol.for('nodejs.stream.writable'),
    kIsClosedPromise: Symbol.for('nodejs.webstream.isClosedPromise'),
    kControllerErrorFunction: Symbol.for('nodejs.webstream.controllerErrorFunction'),
  };
});

// node's `addAbortListener`. An already-aborted signal is answered on a
// microtask with an `abort` Event, as node's is; the event is not marked
// trusted or targeted, since `internal/event_target`'s symbols are not
// reachable here (the streams never read either).
define('internal/events/abort_listener', (module) => {
  function addAbortListener(signal, listener) {
    if (signal === undefined) {
      throw ERR_INVALID_ARG_TYPE('signal', 'AbortSignal', signal);
    }
    validateAbortSignal(signal, 'signal');
    validateFunction(listener, 'listener');
    let removeEventListener;
    if (signal.aborted) {
      let disposed = false;
      queueMicrotask(() => {
        if (disposed) return;
        listener(new Event('abort'));
      });
      removeEventListener = () => {
        disposed = true;
      };
    } else {
      signal.addEventListener('abort', listener, { __proto__: null, once: true });
      removeEventListener = () => {
        signal.removeEventListener('abort', listener);
      };
    }
    return {
      __proto__: null,
      [Symbol.dispose]() {
        removeEventListener?.();
      },
    };
  }
  module.exports = { __proto__: null, addAbortListener };
});

// `internal/webidl`'s `createEnumConverter` and the exception it raises.
define('internal/webidl', (module) => {
  function makeException(message, options = { __proto__: null }) {
    const prefix = options.prefix ? options.prefix + ': ' : '';
    const context = options.context?.length === 0 ?
      '' : (options.context ?? 'Value') + ' ';
    const err = new TypeError(`${prefix}${context}${message}`);
    err.code = options.code || 'ERR_INVALID_ARG_TYPE';
    return err;
  }
  function makeOptions(options, context = options.context, code = options.code) {
    return {
      prefix: options.prefix,
      context,
      code,
      enforceRange: options.enforceRange,
      clamp: options.clamp,
      allowShared: options.allowShared,
      allowResizable: options.allowResizable,
    };
  }
  function toString(V, options = { __proto__: null }) {
    if (typeof V === 'symbol') {
      throw makeException('is a Symbol and cannot be converted to a string.', options);
    }
    return String(V);
  }
  function createEnumConverter(name, values) {
    const E = new Set(values);
    return function (V, options = { __proto__: null }) {
      const S = toString(V, options);
      if (!E.has(S)) {
        throw makeException(
          `'${S}' is not a valid enum value of type ${name}.`,
          makeOptions(options, options.context, 'ERR_INVALID_ARG_VALUE'));
      }
      return S;
    };
  }
  module.exports = { createEnumConverter };
});

define('internal/encoding', (module) => {
  module.exports = { TextDecoder, TextEncoder };
});

// ── lib/internal/webstreams/util.js ────────────────────────────────────────────────────────
define('internal/webstreams/util', (module, exports, require, primordials, internalBinding) => {
'use strict';

const {
  Array,
  ArrayBufferPrototypeGetByteLength,
  ArrayBufferPrototypeGetDetached,
  ArrayBufferPrototypeSlice,
  DataViewPrototypeGetBuffer,
  DataViewPrototypeGetByteLength,
  DataViewPrototypeGetByteOffset,
  FunctionPrototypeCall,
  MathMax,
  NumberIsNaN,
  ObjectFreeze,
  Promise,
  PromisePrototypeThen,
  PromiseReject,
  PromiseResolve,
  Symbol,
  TypedArrayPrototypeGetBuffer,
  TypedArrayPrototypeGetByteLength,
  TypedArrayPrototypeGetByteOffset,
  Uint8Array,
} = primordials;

const {
  codes: {
    ERR_INVALID_ARG_VALUE,
  },
} = require('internal/errors');

const {
  copyArrayBuffer,
} = internalBinding('buffer');

const {
  inspect,
} = require('util');

const {
  constants: {
    kPending,
  },
  getPromiseDetails,
} = internalBinding('util');

const assert = require('internal/assert');

const {
  isDataView,
} = require('internal/util/types');

const {
  validateFunction,
} = require('internal/validators');

const kState = Symbol('kState');
const kType = Symbol('kType');

const getNonWritablePropertyDescriptor = (value) => {
  return {
    __proto__: null,
    configurable: true,
    value,
  };
};

function extractHighWaterMark(value, defaultHWM) {
  if (value === undefined) return defaultHWM;
  const coercedValue = +value;
  if (NumberIsNaN(coercedValue) ||
      coercedValue < 0)
    throw new ERR_INVALID_ARG_VALUE.RangeError('strategy.highWaterMark', value);
  return coercedValue;
}

// The default size algorithm is never exposed to user code, so a single
// shared function avoids one closure allocation per stream.
const defaultSizeAlgorithm = () => 1;

function extractSizeAlgorithm(size) {
  if (size === undefined) return defaultSizeAlgorithm;
  validateFunction(size, 'strategy.size');
  return size;
}

function customInspect(depth, options, name, data) {
  if (depth < 0)
    return this;

  const opts = {
    ...options,
    depth: options.depth == null ? null : options.depth - 1,
  };

  return `${name} ${inspect(data, opts)}`;
}

// These use the original prototype getters so that user tampering with
// the buffer, byteLength, and byteOffset properties on ArrayBuffer and
// ArrayBufferView's is not observed. They run once or more per chunk on
// every byte-stream path, so they must not go through a reflective get
// (the previous view.constructor.prototype lookup was both slower and
// spoofable via a user-defined .constructor).

function ArrayBufferViewGetBuffer(view) {
  return isDataView(view) ?
    DataViewPrototypeGetBuffer(view) :
    TypedArrayPrototypeGetBuffer(view);
}

function ArrayBufferViewGetByteLength(view) {
  return isDataView(view) ?
    DataViewPrototypeGetByteLength(view) :
    TypedArrayPrototypeGetByteLength(view);
}

function ArrayBufferViewGetByteOffset(view) {
  return isDataView(view) ?
    DataViewPrototypeGetByteOffset(view) :
    TypedArrayPrototypeGetByteOffset(view);
}

function cloneAsUint8Array(view) {
  const buffer = ArrayBufferViewGetBuffer(view);
  const byteOffset = ArrayBufferViewGetByteOffset(view);
  const byteLength = ArrayBufferViewGetByteLength(view);
  return new Uint8Array(
    ArrayBufferPrototypeSlice(buffer, byteOffset, byteOffset + byteLength),
  );
}

function canCopyArrayBuffer(toBuffer, toIndex, fromBuffer, fromIndex, count) {
  return toBuffer !== fromBuffer &&
    !ArrayBufferPrototypeGetDetached(toBuffer) &&
    !ArrayBufferPrototypeGetDetached(fromBuffer) &&
    toIndex + count <= ArrayBufferPrototypeGetByteLength(toBuffer) &&
    fromIndex + count <= ArrayBufferPrototypeGetByteLength(fromBuffer);
}

function isBrandCheck(brand) {
  return (value) => {
    return value != null &&
           value[kState] !== undefined &&
           value[kType] === brand;
  };
}

// Backing store for the spec's [[queue]]: a power-of-two ring buffer
// instead of a plain array. Entries are pushed at the tail and consumed
// at the head, so a plain array either moves every element or forces the
// engine to re-linearize on each shift, and the default controllers would
// additionally have to allocate a { value, size } wrapper object per
// chunk just to keep the pair together. Default readable/writable
// controller queues store each entry as (value, size) in two consecutive
// slots via the *Pair methods; the readable byte controller queue stores
// its chunk descriptor records in single slots via push/shift/peek. A
// given instance only ever uses one of the two access patterns, so
// head/tail stay aligned to the entry stride.
class Queue {
  constructor(listLength = 8) {
    this.head = 0;
    this.tail = 0;
    // Number of logical entries currently in the queue: (value, size)
    // pairs for default controller queues, descriptor records for byte
    // controller queues.
    this.length = 0;
    this.capacityMask = listLength - 1;
    this.list = new Array(listLength);
    this.dequeuedSize = 0;
  }

  // Single-slot entries (readable byte controller chunk records).

  push(entry) {
    if (this.length === this.list.length)
      this.grow();
    const tail = this.tail;
    this.list[tail] = entry;
    this.tail = (tail + 1) & this.capacityMask;
    this.length++;
  }

  shift() {
    const head = this.head;
    const list = this.list;
    const entry = list[head];
    list[head] = undefined;
    this.head = (head + 1) & this.capacityMask;
    if (--this.length === 0)
      this.rewind();
    return entry;
  }

  peek() {
    return this.list[this.head];
  }

  // Two-slot (value, size) entries (default controller queues). The
  // stride is always 2 and capacities are even, so `tail + 1`/`head + 1`
  // never need to wrap.

  pushPair(value, size) {
    if (this.length * 2 === this.list.length)
      this.grow();
    const tail = this.tail;
    const list = this.list;
    list[tail] = value;
    list[tail + 1] = size;
    this.tail = (tail + 2) & this.capacityMask;
    this.length++;
  }

  // Returns the dequeued value; the size of the same entry is left in
  // `this.dequeuedSize` so that callers can update [[queueTotalSize]]
  // without a per-entry wrapper object having to exist.
  shiftPair() {
    const head = this.head;
    const list = this.list;
    const value = list[head];
    this.dequeuedSize = list[head + 1];
    list[head] = undefined;
    list[head + 1] = undefined;
    this.head = (head + 2) & this.capacityMask;
    if (--this.length === 0)
      this.rewind();
    return value;
  }

  peekPairValue() {
    return this.list[this.head];
  }

  // The ring is completely full (the tail has caught up with the head, so
  // the next push would overwrite the oldest entry): double the capacity,
  // re-linearizing from the head so index arithmetic stays trivial.
  // Growing before the push rather than after it lets the initial 8-slot
  // ring hold four (value, size) pairs without reallocating.
  grow() {
    const list = this.list;
    const capacity = list.length;
    const head = this.head;
    if (head !== 0) {
      const relinearized = new Array(capacity * 2);
      let n = 0;
      for (let i = head; i < capacity; i++)
        relinearized[n++] = list[i];
      for (let i = 0; i < head; i++)
        relinearized[n++] = list[i];
      this.list = relinearized;
      this.head = 0;
    } else {
      list.length = capacity * 2;
    }
    this.tail = capacity;
    this.capacityMask = (capacity * 2) - 1;
  }

  // The queue just became empty: restart at slot 0 so shallow queues net
  // sequential slot access, and drop the enlarged backing store after a
  // large burst has fully drained.
  rewind() {
    this.head = 0;
    this.tail = 0;
    if (this.list.length > 1024) {
      this.list.length = 8;
      this.capacityMask = 0b111;
    }
  }
}

// Controllers start out with (and are reset to) this shared immutable
// empty queue, so constructing a stream never allocates queue storage;
// a real Queue is materialized by the enqueue paths on first use. All
// dequeue/peek paths are guarded by `.length` (or the equivalent
// [[queueTotalSize]]) checks, so they can never observe the sentinel in
// a mutating way; it never stores entries, so it gets a zero-length
// backing list.
const kEmptyQueue = ObjectFreeze(new Queue(0));

function materializeQueue(state) {
  const queue = state.queue;
  if (queue === kEmptyQueue)
    return state.queue = new Queue();
  return queue;
}

// The queue helpers below run once per chunk on the hot paths of every
// default readable/writable stream, so they load the controller state a
// single time and don't assert the existence of the queue fields (both
// are unconditionally initialized during controller setup and only ever
// replaced wholesale).
function dequeueValue(controller) {
  const state = controller[kState];
  const queue = state.queue;
  assert(queue.length);
  const value = queue.shiftPair();
  state.queueTotalSize = MathMax(0, state.queueTotalSize - queue.dequeuedSize);
  return value;
}

function resetQueue(controller) {
  const state = controller[kState];
  state.queue = kEmptyQueue;
  state.queueTotalSize = 0;
}

function peekQueueValue(controller) {
  const state = controller[kState];
  assert(state.queue.length);
  return state.queue.peekPairValue();
}

function enqueueValueWithSize(controller, value, size) {
  const state = controller[kState];
  const coercedSize = +size;
  if (NumberIsNaN(coercedSize) ||
      coercedSize < 0 ||
      coercedSize === Infinity) {
    throw new ERR_INVALID_ARG_VALUE.RangeError('size', size);
  }
  materializeQueue(state).pushPair(value, coercedSize);
  state.queueTotalSize += coercedSize;
}

// Arity-specialized variants of the promise-callback wrapper. The generic
// rest-parameter + ReflectApply form allocated an arguments array on every
// invocation; these run on per-chunk hot paths (pull/write/transform), so
// each known call-site arity gets its own wrapper. The exact number of
// arguments passed through to the user callback is observable and must be
// preserved.
function createPromiseCallbackNoParams(name, fn, thisArg) {
  validateFunction(fn, name);
  return async () => FunctionPrototypeCall(fn, thisArg);
}

// Raw variants that skip the async wrapper's implicit result promise.
// Consumers of a raw callback invoke it inside try/catch and route the
// result through thenAlgorithmResult() below.
function createRawCallback1Param(name, fn, thisArg) {
  validateFunction(fn, name);
  return (arg) => FunctionPrototypeCall(fn, thisArg, arg);
}

function createRawCallback2Params(name, fn, thisArg) {
  validateFunction(fn, name);
  return (arg1, arg2) => FunctionPrototypeCall(fn, thisArg, arg1, arg2);
}

// A single shared, forever-resolved promise used to enqueue a reaction at
// the next microtask checkpoint without allocating a fresh promise.
const kResolvedPromise = PromiseResolve();

// Returned by an internal algorithm to signal that it parked the
// operation and takes responsibility for delivering the fulfilled (or
// rejected) continuation itself later, instead of settling a promise
// (see the transform stream source pull algorithm).
const kParkedAlgorithmResult = Symbol('kParkedAlgorithmResult');

// Wires the (possibly non-thenable) result of an underlying algorithm
// callback to its fulfilled/rejected continuations. A non-thenable result
// means fulfillment is guaranteed and no then() lookup is observable, so
// the fulfillment step is enqueued directly at the exact microtask
// position the coerced promise's reaction would have had, skipping the
// per-chunk promise allocation. For thenable results PromiseResolve()
// matches the spec's "a promise resolved with" conversion (identity for
// native promises).
function thenAlgorithmResult(result, onFulfilled, onRejected) {
  if (result === kParkedAlgorithmResult)
    return;
  if (result === null ||
      (typeof result !== 'object' && typeof result !== 'function')) {
    PromisePrototypeThen(kResolvedPromise, onFulfilled);
  } else {
    PromisePrototypeThen(PromiseResolve(result), onFulfilled, onRejected);
  }
}

function createPromiseCallback1Param(name, fn, thisArg) {
  validateFunction(fn, name);
  return async (arg) => FunctionPrototypeCall(fn, thisArg, arg);
}

function createPromiseCallback2Params(name, fn, thisArg) {
  validateFunction(fn, name);
  return async (arg1, arg2) => FunctionPrototypeCall(fn, thisArg, arg1, arg2);
}

function isPromisePending(promise) {
  if (promise === undefined) return false;
  const details = getPromiseDetails(promise);
  return details?.[0] === kPending;
}

// Shared shapes for lazily-materialized { promise, resolve, reject }
// records whose settlement is already known.
function resolvedRecord() {
  return {
    promise: PromiseResolve(),
    resolve: undefined,
    reject: undefined,
  };
}

function rejectedHandledRecord(error) {
  const record = {
    promise: PromiseReject(error),
    resolve: undefined,
    reject: undefined,
  };
  setPromiseHandled(record.promise);
  return record;
}

// A single shared, forever-pending promise carried by the duck-typed
// promise records that pipeTo and tee install on their internal reader
// and writer (see ClosedPromiseHook in readablestream.js): the probes on
// the erroring/release paths see a pending promise, and setPromiseHandled
// skips it so that no reaction ever accumulates on it.
const kPendingPromise = new Promise(() => {});

function setPromiseHandled(promise) {
  if (promise === kPendingPromise)
    return;
  // Alternatively, we could use the native API
  // MarkAsHandled, but this avoids the extra boundary cross
  // and is hopefully faster at the cost of an extra Promise
  // allocation.
  PromisePrototypeThen(promise, undefined, () => {});
}

async function nonOpFlush() {}

// Shared non-op for the start/pull/write algorithm callbacks, which all
// follow the raw-callback contract (see createRawCallback*): the
// non-thenable return takes the allocation-free fast path in
// thenAlgorithmResult().
function nonOpCallback() {}

async function nonOpCancel() {}

let transfer;
function lazyTransfer() {
  if (transfer === undefined)
    transfer = require('internal/webstreams/transfer');
  return transfer;
}

module.exports = {
  ArrayBufferViewGetBuffer,
  ArrayBufferViewGetByteLength,
  ArrayBufferViewGetByteOffset,
  Queue,
  canCopyArrayBuffer,
  cloneAsUint8Array,
  copyArrayBuffer,
  createPromiseCallbackNoParams,
  createPromiseCallback1Param,
  createPromiseCallback2Params,
  createRawCallback1Param,
  createRawCallback2Params,
  customInspect,
  defaultSizeAlgorithm,
  dequeueValue,
  enqueueValueWithSize,
  extractHighWaterMark,
  extractSizeAlgorithm,
  getNonWritablePropertyDescriptor,
  isBrandCheck,
  isPromisePending,
  kEmptyQueue,
  kParkedAlgorithmResult,
  kPendingPromise,
  kResolvedPromise,
  kState,
  kType,
  lazyTransfer,
  materializeQueue,
  nonOpCallback,
  nonOpCancel,
  nonOpFlush,

  peekQueueValue,
  rejectedHandledRecord,
  resetQueue,
  resolvedRecord,
  setPromiseHandled,
  thenAlgorithmResult,
};
});

// ── lib/internal/webstreams/queuingstrategies.js ───────────────────────────────────────────
define('internal/webstreams/queuingstrategies', (module, exports, require, primordials, internalBinding) => {
'use strict';

const {
  ObjectDefineProperties,
  ObjectDefineProperty,
  SymbolToStringTag,
} = primordials;

const {
  codes: {
    ERR_MISSING_OPTION,
  },
} = require('internal/errors');

const {
  customInspectSymbol: kInspect,
  kEnumerableProperty,
} = require('internal/util');

const {
  customInspect,
  getNonWritablePropertyDescriptor,
} = require('internal/webstreams/util');

const {
  validateObject,
} = require('internal/validators');

/**
 * @callback QueuingStrategySize
 * @param {any} chunk
 * @returns {number}
 */

/**
 * @typedef {{
 *   highWaterMark : number,
 *   size? : QueuingStrategySize,
 * }} QueuingStrategy
 */

const nameDescriptor = { __proto__: null, value: 'size' };
const byteSizeFunction = ObjectDefineProperty(
  (chunk) => chunk.byteLength,
  'name',
  nameDescriptor,
);
const countSizeFunction = ObjectDefineProperty(() => 1, 'name', nameDescriptor);

/**
 * @type {QueuingStrategy}
 */
class ByteLengthQueuingStrategy {
  #state;
  #byteSizeFunction = byteSizeFunction;

  /**
   * @param {{
   *   highWaterMark : number
   * }} init
   */
  constructor(init) {
    validateObject(init, 'init');
    if (init.highWaterMark === undefined)
      throw new ERR_MISSING_OPTION('init.highWaterMark');

    // The highWaterMark value is not checked until the strategy
    // is actually used, per the spec.
    this.#state = {
      highWaterMark: +init.highWaterMark,
    };
  }

  /**
   * @readonly
   * @type {number}
   */
  get highWaterMark() {
    return this.#state.highWaterMark;
  }

  /**
   * @type {QueuingStrategySize}
   */
  get size() {
    return this.#byteSizeFunction;
  }

  [kInspect](depth, options) {
    return customInspect(depth, options, 'ByteLengthQueuingStrategy', {
      highWaterMark: this.highWaterMark,
    });
  }
}

ObjectDefineProperties(ByteLengthQueuingStrategy.prototype, {
  highWaterMark: kEnumerableProperty,
  size: kEnumerableProperty,
  [SymbolToStringTag]: getNonWritablePropertyDescriptor(ByteLengthQueuingStrategy.name),
});

/**
 * @type {QueuingStrategy}
 */
class CountQueuingStrategy {
  #state;
  #countSizeFunction = countSizeFunction;

  /**
   * @param {{
   *   highWaterMark : number
   * }} init
   */
  constructor(init) {
    validateObject(init, 'init');
    if (init.highWaterMark === undefined)
      throw new ERR_MISSING_OPTION('init.highWaterMark');

    // The highWaterMark value is not checked until the strategy
    // is actually used, per the spec.
    this.#state = {
      highWaterMark: +init.highWaterMark,
    };
  }

  /**
   * @readonly
   * @type {number}
   */
  get highWaterMark() {
    return this.#state.highWaterMark;
  }

  /**
   * @type {QueuingStrategySize}
   */
  get size() {
    return this.#countSizeFunction;
  }

  [kInspect](depth, options) {
    return customInspect(depth, options, 'CountQueuingStrategy', {
      highWaterMark: this.highWaterMark,
    });
  }
}

ObjectDefineProperties(CountQueuingStrategy.prototype, {
  highWaterMark: kEnumerableProperty,
  size: kEnumerableProperty,
  [SymbolToStringTag]: getNonWritablePropertyDescriptor(CountQueuingStrategy.name),
});

module.exports = {
  ByteLengthQueuingStrategy,
  CountQueuingStrategy,
};
});

// ── lib/internal/webstreams/writablestream.js ──────────────────────────────────────────────
define('internal/webstreams/writablestream', (module, exports, require, primordials, internalBinding) => {
'use strict';

const {
  FunctionPrototypeBind,
  FunctionPrototypeCall,
  ObjectDefineProperties,
  ObjectSetPrototypeOf,
  Promise,
  PromisePrototypeThen,
  PromiseReject,
  PromiseResolve,
  PromiseWithResolvers,
  Symbol,
  SymbolToStringTag,
} = primordials;

const {
  codes: {
    ERR_ILLEGAL_CONSTRUCTOR,
    ERR_INVALID_ARG_TYPE,
    ERR_INVALID_ARG_VALUE,
    ERR_INVALID_STATE,
    ERR_INVALID_THIS,
  },
} = require('internal/errors');

const {
  DOMException,
} = internalBinding('messaging');

const {
  customInspectSymbol: kInspect,
  kEmptyObject,
  kEnumerableProperty,
  SideEffectFreeRegExpPrototypeSymbolReplace,
} = require('internal/util');

const {
  validateObject,
  kValidateObjectAllowObjects,
  kValidateObjectAllowObjectsAndNull,
} = require('internal/validators');

const {
  MessageChannel,
} = require('internal/worker/io');

const {
  kDeserialize,
  kTransfer,
  kTransferList,
  markTransferMode,
} = require('internal/worker/js_transferable');

const {
  Queue,
  createPromiseCallbackNoParams,
  createPromiseCallback1Param,
  createRawCallback2Params,
  customInspect,
  defaultSizeAlgorithm,
  dequeueValue,
  enqueueValueWithSize,
  extractHighWaterMark,
  extractSizeAlgorithm,
  getNonWritablePropertyDescriptor,
  isBrandCheck,
  isPromisePending,
  kEmptyQueue,
  kState,
  kType,
  lazyTransfer,
  nonOpCallback,
  nonOpCancel,
  peekQueueValue,
  rejectedHandledRecord,
  resetQueue,
  resolvedRecord,
  setPromiseHandled,
  thenAlgorithmResult,
} = require('internal/webstreams/util');

const {
  kIsClosedPromise,
  kIsErrored,
  kIsWritable,
  kControllerErrorFunction,
} = require('internal/streams/utils');

const {
  AbortController,
} = require('internal/abort_controller');

const {
  queueMicrotask,
} = require('internal/process/task_queues');

const assert = require('internal/assert');

const kAbort = Symbol('kAbort');
const kCloseSentinel = Symbol('kCloseSentinel');
const kError = Symbol('kError');
const kSkipThrow = Symbol('kSkipThrow');

// Shared sentinels for the "no pending request" state records. These
// records are only ever replaced wholesale and never mutated in place,
// so single shared instances are safe and avoid an allocation on every
// state reset (one per write on the hot path). The prototype is nulled
// after creation: a `__proto__: null` literal is created in dictionary
// mode, and these fields are loaded several times per write.
const kNilRequest = ObjectSetPrototypeOf({
  promise: undefined,
  resolve: undefined,
  reject: undefined,
}, null);
const kNilPendingAbortRequest = ObjectSetPrototypeOf({
  abort: kNilRequest,
  reason: undefined,
  wasAlreadyErroring: false,
}, null);

let releasedError;

function lazyWritableReleasedError() {
  if (releasedError) {
    return releasedError;
  }
  const userModuleRegExp = /^ {4}at (?:[^/\\(]+ \()(?!node:(.+):\d+:\d+\)$).*/gm;

  releasedError = new ERR_INVALID_STATE.TypeError('Writer has been released');
  // Avoid V8 leak and remove userland stackstrace
  releasedError.stack = SideEffectFreeRegExpPrototypeSymbolReplace(userModuleRegExp, releasedError.stack, '');
  return releasedError;
}

/**
 * @typedef {import('../abort_controller').AbortSignal} AbortSignal
 * @typedef {import('./queuingstrategies').QueuingStrategy
 * } QueuingStrategy
 * @typedef {import('./queuingstrategies').QueuingStrategySize
 * } QueuingStrategySize
 */

/**
 * @callback UnderlyingSinkStartCallback
 * @param {WritableStreamDefaultController} controller
 */

/**
 * @callback UnderlyingSinkWriteCallback
 * @param {any} chunk
 * @param {WritableStreamDefaultController} controller
 * @returns {Promise<void>}
 */

/**
 * @callback UnderlyingSinkCloseCallback
 * @returns {Promise<void>}
 */

/**
 * @callback UnderlyingSinkAbortCallback
 * @param {any} reason
 * @returns {Promise<void>}
 */

/**
 * @typedef {{
 *   start? : UnderlyingSinkStartCallback,
 *   write? : UnderlyingSinkWriteCallback,
 *   close? : UnderlyingSinkCloseCallback,
 *   abort? : UnderlyingSinkAbortCallback,
 *   type? : any,
 * }} UnderlyingSink
 */

class WritableStream {
  [kType] = 'WritableStream';

  /**
   * @param {UnderlyingSink} [sink]
   * @param {QueuingStrategy} [strategy]
   */
  constructor(sink = kEmptyObject, strategy = kEmptyObject) {
    markTransferMode(this, false, true);
    // Internal construction (transform streams, adapters, transfer):
    // the caller sets up the controller, so every WritableStream shares
    // one hidden class and no per-instance prototype swap is needed.
    if (sink === kSkipThrow) {
      this[kState] = createWritableStreamState();
      return;
    }
    validateObject(sink, 'sink', kValidateObjectAllowObjects);
    validateObject(strategy, 'strategy', kValidateObjectAllowObjectsAndNull);
    const type = sink?.type;
    if (type !== undefined)
      throw new ERR_INVALID_ARG_VALUE.RangeError('type', type);

    this[kState] = createWritableStreamState();

    const size = extractSizeAlgorithm(strategy?.size);
    const highWaterMark = extractHighWaterMark(strategy?.highWaterMark, 1);

    setupWritableStreamDefaultControllerFromSink(
      this,
      sink,
      highWaterMark,
      size);
  }

  get [kIsErrored]() {
    return this[kState].state === 'errored';
  }

  get [kIsWritable]() {
    return this[kState].state === 'writable';
  }

  [kControllerErrorFunction](error) {
    // Used by the internal stream interop (addAbortSignal).
    this[kState].controller.error(error);
  }

  // Used by the internal stream interop (end-of-stream). Materialized
  // lazily since its settlement is derivable from the stream state.
  get [kIsClosedPromise]() {
    let cache = this[kState].closedPromise;
    if (cache === undefined) {
      switch (this[kState].state) {
        case 'writable':
        case 'erroring':
          cache = PromiseWithResolvers();
          break;
        case 'closed':
          cache = resolvedRecord();
          break;
        default:
          cache = rejectedHandledRecord(this[kState].storedError);
      }
      this[kState].closedPromise = cache;
    }
    return cache;
  }

  /**
   * @readonly
   * @type {boolean}
   */
  get locked() {
    if (!isWritableStream(this))
      throw new ERR_INVALID_THIS('WritableStream');
    return isWritableStreamLocked(this);
  }

  /**
   * @param {any} [reason]
   * @returns {Promise<void>}
   */
  abort(reason = undefined) {
    if (!isWritableStream(this))
      return PromiseReject(new ERR_INVALID_THIS('WritableStream'));
    if (isWritableStreamLocked(this)) {
      return PromiseReject(
        new ERR_INVALID_STATE.TypeError('WritableStream is locked'));
    }
    return writableStreamAbort(this, reason);
  }

  /**
   * @returns {Promise<void>}
   */
  close() {
    if (!isWritableStream(this))
      return PromiseReject(new ERR_INVALID_THIS('WritableStream'));
    if (isWritableStreamLocked(this)) {
      return PromiseReject(
        new ERR_INVALID_STATE.TypeError('WritableStream is locked'));
    }
    if (writableStreamCloseQueuedOrInFlight(this)) {
      return PromiseReject(
        new ERR_INVALID_STATE.TypeError('Failure closing WritableStream'));
    }
    return writableStreamClose(this);
  }

  /**
   * @returns {WritableStreamDefaultWriter}
   */
  getWriter() {
    if (!isWritableStream(this))
      throw new ERR_INVALID_THIS('WritableStream');
    // eslint-disable-next-line no-use-before-define
    return new WritableStreamDefaultWriter(this);
  }

  [kInspect](depth, options) {
    return customInspect(depth, options, this[kType], {
      locked: this.locked,
      state: this[kState].state,
    });
  }

  [kTransfer]() {
    if (!isWritableStream(this))
      throw new ERR_INVALID_THIS('WritableStream');
    if (this.locked) {
      this[kState].transfer.port1?.close();
      this[kState].transfer.port1 = undefined;
      this[kState].transfer.port2 = undefined;
      throw new DOMException(
        'Cannot transfer a locked WritableStream',
        'DataCloneError');
    }

    const {
      readable,
      promise,
    } = lazyTransfer().newCrossRealmReadableStream(
      this,
      this[kState].transfer.port1);

    this[kState].transfer.readable = readable;
    this[kState].transfer.promise = promise;

    return {
      data: { port: this[kState].transfer.port2 },
      deserializeInfo:
        'internal/webstreams/writablestream:TransferredWritableStream',
    };
  }

  [kTransferList]() {
    const { port1, port2 } = new MessageChannel();
    this[kState].transfer.port1 = port1;
    this[kState].transfer.port2 = port2;
    return [ port2 ];
  }

  [kDeserialize]({ port }) {
    const transfer = lazyTransfer();
    setupWritableStreamDefaultControllerFromSink(
      this,
      // The MessagePort is set to be referenced when reading.
      // After two MessagePorts are closed, there is a problem with
      // lingering promise not being properly resolved.
      // https://github.com/nodejs/node/issues/51486
      new transfer.CrossRealmTransformWritableSink(port, true),
      1,
      defaultSizeAlgorithm);
  }
}

ObjectDefineProperties(WritableStream.prototype, {
  locked: kEnumerableProperty,
  abort: kEnumerableProperty,
  close: kEnumerableProperty,
  getWriter: kEnumerableProperty,
  [SymbolToStringTag]: getNonWritablePropertyDescriptor(WritableStream.name),
});

function TransferredWritableStream() {
  return new WritableStream(kSkipThrow);
}

TransferredWritableStream.prototype[kDeserialize] = () => {};

class WritableStreamDefaultWriter {
  [kType] = 'WritableStreamDefaultWriter';

  /**
   * @param {WritableStream} stream
   */
  constructor(stream) {
    if (!isWritableStream(stream))
      throw new ERR_INVALID_ARG_TYPE('stream', 'WritableStream', stream);
    this[kState] = {
      stream: undefined,
      // Lazily materialized by writerClosedPromise()/writerReadyPromise().
      close: undefined,
      ready: undefined,
    };
    setupWritableStreamDefaultWriter(this, stream);
  }

  /**
   * @readonly
   * @type {Promise<void>}
   */
  get closed() {
    if (!isWritableStreamDefaultWriter(this))
      return PromiseReject(new ERR_INVALID_THIS('WritableStreamDefaultWriter'));
    return writerClosedPromise(this).promise;
  }

  /**
   * @readonly
   * @type {number}
   */
  get desiredSize() {
    if (!isWritableStreamDefaultWriter(this))
      throw new ERR_INVALID_THIS('WritableStreamDefaultWriter');
    if (this[kState].stream === undefined) {
      throw new ERR_INVALID_STATE.TypeError(
        'Writer is not bound to a WritableStream');
    }
    return writableStreamDefaultWriterGetDesiredSize(this);
  }

  /**
   * @readonly
   * @type {Promise<void>}
   */
  get ready() {
    if (!isWritableStreamDefaultWriter(this))
      return PromiseReject(new ERR_INVALID_THIS('WritableStreamDefaultWriter'));
    return writerReadyPromise(this).promise;
  }

  /**
   * @param {any} [reason]
   * @returns {Promise<void>}
   */
  abort(reason = undefined) {
    if (!isWritableStreamDefaultWriter(this))
      return PromiseReject(new ERR_INVALID_THIS('WritableStreamDefaultWriter'));
    if (this[kState].stream === undefined) {
      return PromiseReject(
        new ERR_INVALID_STATE.TypeError(
          'Writer is not bound to a WritableStream'));
    }
    return writableStreamDefaultWriterAbort(this, reason);
  }

  /**
   * @returns {Promise<void>}
   */
  close() {
    if (!isWritableStreamDefaultWriter(this))
      return PromiseReject(new ERR_INVALID_THIS('WritableStreamDefaultWriter'));
    const {
      stream,
    } = this[kState];
    if (stream === undefined) {
      return PromiseReject(
        new ERR_INVALID_STATE.TypeError(
          'Writer is not bound to a WritableStream'));
    }
    if (writableStreamCloseQueuedOrInFlight(stream)) {
      return PromiseReject(
        new ERR_INVALID_STATE.TypeError('Failure to close WritableStream'));
    }
    return writableStreamDefaultWriterClose(this);
  }

  releaseLock() {
    if (!isWritableStreamDefaultWriter(this))
      throw new ERR_INVALID_THIS('WritableStreamDefaultWriter');
    const {
      stream,
    } = this[kState];
    if (stream === undefined)
      return;
    assert(stream[kState].writer !== undefined);
    writableStreamDefaultWriterRelease(this);
  }

  /**
   * @param {any} [chunk]
   * @returns {Promise<void>}
   */
  write(chunk = undefined) {
    if (!isWritableStreamDefaultWriter(this))
      return PromiseReject(new ERR_INVALID_THIS('WritableStreamDefaultWriter'));
    if (this[kState].stream === undefined) {
      return PromiseReject(
        new ERR_INVALID_STATE.TypeError(
          'Writer is not bound to a WritableStream'));
    }
    return writableStreamDefaultWriterWrite(this, chunk);
  }

  [kInspect](depth, options) {
    return customInspect(depth, options, this[kType], {
      stream: this[kState].stream,
      close: writerClosedPromise(this).promise,
      ready: writerReadyPromise(this).promise,
      desiredSize: this.desiredSize,
    });
  }
}

ObjectDefineProperties(WritableStreamDefaultWriter.prototype, {
  closed: kEnumerableProperty,
  ready: kEnumerableProperty,
  desiredSize: kEnumerableProperty,
  abort: kEnumerableProperty,
  close: kEnumerableProperty,
  releaseLock: kEnumerableProperty,
  write: kEnumerableProperty,
  [SymbolToStringTag]: getNonWritablePropertyDescriptor(WritableStreamDefaultWriter.name),
});

class WritableStreamDefaultController {
  [kType] = 'WritableStreamDefaultController';
  [kState];

  constructor(skipThrowSymbol = undefined) {
    if (skipThrowSymbol !== kSkipThrow) {
      throw new ERR_ILLEGAL_CONSTRUCTOR();
    }
  }

  [kAbort](reason) {
    const result = this[kState].abortAlgorithm(reason);
    writableStreamDefaultControllerClearAlgorithms(this);
    return result;
  }

  [kError]() {
    resetQueue(this);
  }

  /**
   * @type {AbortSignal}
   */
  get signal() {
    if (!isWritableStreamDefaultController(this))
      throw new ERR_INVALID_THIS('WritableStreamDefaultController');
    return this[kState].abortController.signal;
  }

  /**
   * @param {any} [error]
   */
  error(error = undefined) {
    if (!isWritableStreamDefaultController(this))
      throw new ERR_INVALID_THIS('WritableStreamDefaultController');
    if (this[kState].stream[kState].state !== 'writable')
      return;
    writableStreamDefaultControllerError(this, error);
  }

  [kInspect](depth, options) {
    return customInspect(depth, options, this[kType], {
      stream: this[kState].stream,
    });
  }
}

ObjectDefineProperties(WritableStreamDefaultController.prototype, {
  signal: kEnumerableProperty,
  error: kEnumerableProperty,
  [SymbolToStringTag]: getNonWritablePropertyDescriptor(WritableStreamDefaultController.name),
});

function createWritableStream(start, write, close, abort, highWaterMark = 1, size = defaultSizeAlgorithm) {
  const stream = new WritableStream(kSkipThrow);
  setupWritableStreamDefaultController(
    stream,
    new WritableStreamDefaultController(kSkipThrow),
    start,
    write,
    close,
    abort,
    highWaterMark,
    size,
  );
  return stream;
}

const isWritableStream =
  isBrandCheck('WritableStream');
const isWritableStreamDefaultWriter =
  isBrandCheck('WritableStreamDefaultWriter');
const isWritableStreamDefaultController =
  isBrandCheck('WritableStreamDefaultController');

// The state record classes null out their prototype chain to keep
// Object.prototype out of property lookups; every field ever assigned
// is declared so the shape never transitions.
class WritableStreamTransferState {
  readable = undefined;
  port1 = undefined;
  port2 = undefined;
  promise = undefined;
}
ObjectSetPrototypeOf(WritableStreamTransferState.prototype, null);

class WritableStreamState {
  closedPromise = undefined;
  closeRequest = kNilRequest;
  // Mirrors "closeRequest or inFlightCloseRequest is pending"; kept as a
  // flag because the predicate runs several times per chunk on the write
  // hot path.
  closeQueuedOrInFlight = false;
  inFlightWriteRequest = kNilRequest;
  inFlightCloseRequest = kNilRequest;
  pendingAbortRequest = kNilPendingAbortRequest;
  backpressure = false;
  controller = undefined;
  state = 'writable';
  storedError = undefined;
  // Ring-buffer request queue, materialized lazily on the first pending
  // write (see writableStreamAddWriteRequest) so construction allocates
  // no request storage.
  writeRequests = kEmptyQueue;
  writer = undefined;
  transfer = new WritableStreamTransferState();
}
ObjectSetPrototypeOf(WritableStreamState.prototype, null);

function createWritableStreamState() {
  return new WritableStreamState();
}

function isWritableStreamLocked(stream) {
  return stream[kState].writer !== undefined;
}

function setupWritableStreamDefaultWriter(writer, stream) {
  if (isWritableStreamLocked(stream))
    throw new ERR_INVALID_STATE.TypeError('WritableStream is locked');
  writer[kState].stream = stream;
  stream[kState].writer = writer;
  // The writer's [[readyPromise]] and [[closedPromise]] are materialized
  // lazily by writerReadyPromise()/writerClosedPromise(); their settlement
  // is fully derivable from the writer/stream state, so nothing is
  // allocated here.
}

// Materializes the writer's [[closedPromise]] record on first
// observation. While unobserved, its settlement is derivable: a detached
// writer was released (rejected with the shared released error);
// otherwise it follows the stream state. The settle sites only touch the
// record when it has been materialized.
function writerClosedPromise(writer) {
  let close = writer[kState].close;
  if (close === undefined) {
    const stream = writer[kState].stream;
    if (stream === undefined) {
      close = rejectedHandledRecord(lazyWritableReleasedError());
    } else {
      switch (stream[kState].state) {
        case 'writable':
        case 'erroring':
          close = PromiseWithResolvers();
          break;
        case 'closed':
          close = resolvedRecord();
          break;
        default:
          close = rejectedHandledRecord(stream[kState].storedError);
      }
    }
    writer[kState].close = close;
  }
  return close;
}

// Same as writerClosedPromise(), for the writer's [[readyPromise]]. The
// pending epoch is derivable too: a writable stream with backpressure and
// no queued close has a pending ready promise.
function writerReadyPromise(writer) {
  let ready = writer[kState].ready;
  if (ready === undefined) {
    const stream = writer[kState].stream;
    if (stream === undefined) {
      ready = rejectedHandledRecord(lazyWritableReleasedError());
    } else {
      switch (stream[kState].state) {
        case 'writable':
          ready = !writableStreamCloseQueuedOrInFlight(stream) &&
                  stream[kState].backpressure ?
            PromiseWithResolvers() :
            resolvedRecord();
          break;
        case 'closed':
          ready = resolvedRecord();
          break;
        default:
          ready = rejectedHandledRecord(stream[kState].storedError);
      }
    }
    writer[kState].ready = ready;
  }
  return ready;
}

function writableStreamAbort(stream, reason) {
  const { controller } = stream[kState];

  let state = stream[kState].state;
  if (state === 'closed' || state === 'errored')
    return PromiseResolve();

  controller[kState].abortController.abort(reason);

  state = stream[kState].state;
  if (state === 'closed' || state === 'errored')
    return PromiseResolve();

  if (stream[kState].pendingAbortRequest.abort.promise !== undefined)
    return stream[kState].pendingAbortRequest.abort.promise;

  assert(state === 'writable' || state === 'erroring');

  let wasAlreadyErroring = false;
  if (state === 'erroring') {
    wasAlreadyErroring = true;
    reason = undefined;
  }

  const abort = PromiseWithResolvers();

  stream[kState].pendingAbortRequest = {
    abort,
    reason,
    wasAlreadyErroring,
  };

  if (!wasAlreadyErroring)
    writableStreamStartErroring(stream, reason);

  return abort.promise;
}

function writableStreamClose(stream) {
  const {
    state,
    writer,
    backpressure,
    controller,
  } = stream[kState];
  if (state === 'closed' || state === 'errored') {
    return PromiseReject(
      new ERR_INVALID_STATE.TypeError('WritableStream is closed'));
  }
  assert(state === 'writable' || state === 'erroring');
  assert(!writableStreamCloseQueuedOrInFlight(stream));
  stream[kState].closeRequest = PromiseWithResolvers();
  stream[kState].closeQueuedOrInFlight = true;
  const { promise } = stream[kState].closeRequest;
  if (writer !== undefined && backpressure && state === 'writable')
    writer[kState].ready?.resolve();
  writableStreamDefaultControllerClose(controller);
  return promise;
}

// Callers invoke this only in the 'writable' state with no close queued or
// in flight (the two conditions the spec's assertions check), so the
// backpressure value is derived here from the desired size instead of being
// computed and passed in by each caller.
function writableStreamUpdateBackpressure(controller, streamState) {
  const controllerState = controller[kState];
  const backpressure =
    controllerState.highWaterMark - controllerState.queueTotalSize <= 0;
  const writer = streamState.writer;
  const changed = streamState.backpressure !== backpressure;
  // The state field is published before the ready record is resolved so
  // that a ready resolve hook (pipeTo's pump continuation) observes the
  // new value.
  streamState.backpressure = backpressure;
  if (writer !== undefined && changed) {
    if (backpressure) {
      // The spec replaces [[readyPromise]] with a fresh pending promise;
      // dropping the cache lets the next observation derive it.
      writer[kState].ready = undefined;
    } else {
      writer[kState].ready?.resolve();
    }
  }
}

function writableStreamStartErroring(stream, reason) {
  assert(stream[kState].storedError === undefined);
  assert(stream[kState].state === 'writable');
  const {
    controller,
    writer,
  } = stream[kState];
  assert(controller !== undefined);
  stream[kState].state = 'erroring';
  stream[kState].storedError = reason;
  if (writer !== undefined) {
    writableStreamDefaultWriterEnsureReadyPromiseRejected(writer, reason);
  }
  if (!writableStreamHasOperationMarkedInFlight(stream) &&
      controller[kState].started) {
    writableStreamFinishErroring(stream);
  }
}

function writableStreamRejectCloseAndClosedPromiseIfNeeded(stream) {
  assert(stream[kState].state === 'errored');
  if (stream[kState].closeRequest.promise !== undefined) {
    assert(stream[kState].inFlightCloseRequest.promise === undefined);
    stream[kState].closeRequest.reject(stream[kState].storedError);
    stream[kState].closeRequest = kNilRequest;
    stream[kState].closeQueuedOrInFlight = false;
  }

  const closedPromiseCache = stream[kState].closedPromise;
  if (closedPromiseCache !== undefined && isPromisePending(closedPromiseCache.promise)) {
    setPromiseHandled(closedPromiseCache.promise);
    closedPromiseCache.reject(stream[kState].storedError);
  }

  const {
    writer,
  } = stream[kState];
  if (writer !== undefined) {
    const closeCache = writer[kState].close;
    if (closeCache !== undefined && isPromisePending(closeCache.promise)) {
      setPromiseHandled(closeCache.promise);
      closeCache.reject(stream[kState].storedError);
    }
  }
}

function writableStreamMarkFirstWriteRequestInFlight(stream) {
  assert(stream[kState].inFlightWriteRequest.promise === undefined);
  assert(stream[kState].writeRequests.length);
  const writeRequest = stream[kState].writeRequests.shift();
  stream[kState].inFlightWriteRequest = writeRequest;
}

function writableStreamMarkCloseRequestInFlight(stream) {
  assert(stream[kState].inFlightWriteRequest.promise === undefined);
  assert(stream[kState].closeRequest.promise !== undefined);
  stream[kState].inFlightCloseRequest = stream[kState].closeRequest;
  stream[kState].closeRequest = kNilRequest;
}

function writableStreamHasOperationMarkedInFlight(stream) {
  const {
    inFlightWriteRequest,
    inFlightCloseRequest,
  } = stream[kState];
  if (inFlightWriteRequest.promise === undefined &&
      inFlightCloseRequest.promise === undefined) {
    return false;
  }
  return true;
}

function writableStreamFinishInFlightWriteWithError(stream, error) {
  assert(stream[kState].inFlightWriteRequest.promise !== undefined);
  stream[kState].inFlightWriteRequest.reject(error);
  stream[kState].inFlightWriteRequest = kNilRequest;
  assert(stream[kState].state === 'writable' ||
         stream[kState].state === 'erroring');
  writableStreamDealWithRejection(stream, error);
}

function writableStreamFinishInFlightWrite(stream) {
  assert(stream[kState].inFlightWriteRequest.promise !== undefined);
  stream[kState].inFlightWriteRequest.resolve();
  stream[kState].inFlightWriteRequest = kNilRequest;
}

function writableStreamFinishInFlightCloseWithError(stream, error) {
  assert(stream[kState].inFlightCloseRequest.promise !== undefined);
  stream[kState].inFlightCloseRequest.reject(error);
  stream[kState].inFlightCloseRequest = kNilRequest;
  stream[kState].closeQueuedOrInFlight = false;
  assert(stream[kState].state === 'writable' ||
         stream[kState].state === 'erroring');
  if (stream[kState].pendingAbortRequest.abort.promise !== undefined) {
    stream[kState].pendingAbortRequest.abort.reject(error);
    stream[kState].pendingAbortRequest = kNilPendingAbortRequest;
  }
  writableStreamDealWithRejection(stream, error);
}

function writableStreamFinishInFlightClose(stream) {
  assert(stream[kState].inFlightCloseRequest.promise !== undefined);
  stream[kState].inFlightCloseRequest.resolve();
  stream[kState].inFlightCloseRequest = kNilRequest;
  stream[kState].closeQueuedOrInFlight = false;
  if (stream[kState].state === 'erroring') {
    stream[kState].storedError = undefined;
    if (stream[kState].pendingAbortRequest.abort.promise !== undefined) {
      stream[kState].pendingAbortRequest.abort.resolve();
      stream[kState].pendingAbortRequest = kNilPendingAbortRequest;
    }
  }
  stream[kState].state = 'closed';
  if (stream[kState].writer !== undefined)
    stream[kState].writer[kState].close?.resolve();
  stream[kState].closedPromise?.resolve();
  assert(stream[kState].pendingAbortRequest.abort.promise === undefined);
  assert(stream[kState].storedError === undefined);
}

function writableStreamFinishErroring(stream) {
  assert(stream[kState].state === 'erroring');
  assert(!writableStreamHasOperationMarkedInFlight(stream));
  stream[kState].state = 'errored';
  stream[kState].controller[kError]();
  const storedError = stream[kState].storedError;
  const writeRequests = stream[kState].writeRequests;
  while (writeRequests.length)
    writeRequests.shift().reject(storedError);

  if (stream[kState].pendingAbortRequest.abort.promise === undefined) {
    writableStreamRejectCloseAndClosedPromiseIfNeeded(stream);
    return;
  }

  const abortRequest = stream[kState].pendingAbortRequest;
  stream[kState].pendingAbortRequest = kNilPendingAbortRequest;
  if (abortRequest.wasAlreadyErroring) {
    abortRequest.abort.reject(storedError);
    writableStreamRejectCloseAndClosedPromiseIfNeeded(stream);
    return;
  }
  // The closed promises are only rejected once the sink's abort algorithm
  // settles, so materialize them here: deriving them from the 'errored'
  // state within that window would hand out an already rejected promise.
  stream[kState].closedPromise ??= PromiseWithResolvers();
  const writer = stream[kState].writer;
  if (writer !== undefined)
    writer[kState].close ??= PromiseWithResolvers();
  PromisePrototypeThen(
    stream[kState].controller[kAbort](abortRequest.reason),
    () => {
      abortRequest.abort.resolve();
      writableStreamRejectCloseAndClosedPromiseIfNeeded(stream);
    },
    (error) => {
      abortRequest.abort.reject(error);
      writableStreamRejectCloseAndClosedPromiseIfNeeded(stream);
    });
}

function writableStreamDealWithRejection(stream, error) {
  const {
    state,
  } = stream[kState];
  if (state === 'writable') {
    writableStreamStartErroring(stream, error);
    return;
  }

  assert(state === 'erroring');
  writableStreamFinishErroring(stream);
}

function writableStreamCloseQueuedOrInFlight(stream) {
  return stream[kState].closeQueuedOrInFlight;
}

function writableStreamAddWriteRequest(stream) {
  assert(isWritableStreamLocked(stream));
  assert(stream[kState].state === 'writable');
  // PromiseWithResolvers() already returns a { promise, resolve, reject }
  // record, so push it as-is instead of rebuilding an identical object.
  const writeRequest = PromiseWithResolvers();
  const streamState = stream[kState];
  let writeRequests = streamState.writeRequests;
  if (writeRequests === kEmptyQueue)
    writeRequests = streamState.writeRequests = new Queue();
  writeRequests.push(writeRequest);
  return writeRequest.promise;
}

function writableStreamDefaultWriterWrite(writer, chunk) {
  const writerState = writer[kState];
  const stream = writerState.stream;
  assert(stream !== undefined);
  const streamState = stream[kState];
  const {
    controller,
  } = streamState;
  const chunkSize = writableStreamDefaultControllerGetChunkSize(
    controller,
    chunk);
  if (stream !== writerState.stream) {
    return PromiseReject(
      new ERR_INVALID_STATE.TypeError('Mismatched WritableStreams'));
  }
  const {
    state,
  } = streamState;

  if (state === 'errored')
    return PromiseReject(streamState.storedError);

  if (streamState.closeQueuedOrInFlight || state === 'closed') {
    return PromiseReject(
      new ERR_INVALID_STATE.TypeError('WritableStream is closed'));
  }

  if (state === 'erroring')
    return PromiseReject(streamState.storedError);

  assert(state === 'writable');

  const promise = writableStreamAddWriteRequest(stream);
  writableStreamDefaultControllerWrite(controller, chunk, chunkSize);
  return promise;
}

// Variant of writableStreamDefaultWriterWrite for pipeTo: the caller
// provides a shared request object instead of a per-write promise record.
// `pending` is incremented before the controller write, which can settle
// requests synchronously when the stream starts erroring; precondition
// failures are latched on `failed`/`failure`.
function writableStreamDefaultWriterWriteWithRequest(writer, chunk, request) {
  const writerState = writer[kState];
  const stream = writerState.stream;
  assert(stream !== undefined);
  const streamState = stream[kState];
  const {
    controller,
  } = streamState;
  const chunkSize = writableStreamDefaultControllerGetChunkSize(
    controller,
    chunk);
  if (stream !== writerState.stream) {
    request.failed = true;
    request.failure =
      new ERR_INVALID_STATE.TypeError('Mismatched WritableStreams');
    return;
  }
  const {
    state,
  } = streamState;

  if (state === 'errored') {
    request.failed = true;
    request.failure = streamState.storedError;
    return;
  }

  if (streamState.closeQueuedOrInFlight || state === 'closed') {
    request.failed = true;
    request.failure =
      new ERR_INVALID_STATE.TypeError('WritableStream is closed');
    return;
  }

  if (state === 'erroring') {
    request.failed = true;
    request.failure = streamState.storedError;
    return;
  }

  assert(state === 'writable');

  let writeRequests = streamState.writeRequests;
  if (writeRequests === kEmptyQueue)
    writeRequests = streamState.writeRequests = new Queue();
  writeRequests.push(request);
  request.pending++;
  writableStreamDefaultControllerWrite(controller, chunk, chunkSize);
}

function writableStreamDefaultWriterRelease(writer) {
  const {
    stream,
  } = writer[kState];
  assert(stream !== undefined);
  assert(stream[kState].writer === writer);
  const releasedStateError = lazyWritableReleasedError();
  writableStreamDefaultWriterEnsureReadyPromiseRejected(writer, releasedStateError);
  writableStreamDefaultWriterEnsureClosedPromiseRejected(writer, releasedStateError);
  stream[kState].writer = undefined;
  writer[kState].stream = undefined;
}

function writableStreamDefaultWriterGetDesiredSize(writer) {
  const {
    stream,
  } = writer[kState];
  switch (stream[kState].state) {
    case 'errored':
      // Fall through
    case 'erroring':
      return null;
    case 'closed':
      return 0;
  }
  return writableStreamDefaultControllerGetDesiredSize(
    stream[kState].controller);
}

function writableStreamDefaultWriterEnsureReadyPromiseRejected(writer, error) {
  const ready = writer[kState].ready;
  if (ready !== undefined && isPromisePending(ready.promise)) {
    ready.reject(error);
    setPromiseHandled(ready.promise);
  } else {
    // The spec replaces [[readyPromise]] with a promise rejected with the
    // same error the post-release/post-erroring state derives; dropping
    // the cache lets the next observation materialize it.
    writer[kState].ready = undefined;
  }
}

function writableStreamDefaultWriterEnsureClosedPromiseRejected(writer, error) {
  const close = writer[kState].close;
  if (close !== undefined && isPromisePending(close.promise)) {
    close.reject(error);
    setPromiseHandled(close.promise);
  } else {
    // See writableStreamDefaultWriterEnsureReadyPromiseRejected.
    writer[kState].close = undefined;
  }
}

function writableStreamDefaultWriterCloseWithErrorPropagation(writer) {
  const {
    stream,
  } = writer[kState];
  assert(stream !== undefined);
  const {
    state,
  } = stream[kState];
  if (writableStreamCloseQueuedOrInFlight(stream) || state === 'closed')
    return PromiseResolve();

  if (state === 'errored')
    return PromiseReject(stream[kState].storedError);

  assert(state === 'writable' || state === 'erroring');

  return writableStreamDefaultWriterClose(writer);
}

function writableStreamDefaultWriterClose(writer) {
  const {
    stream,
  } = writer[kState];
  assert(stream !== undefined);
  return writableStreamClose(stream);
}

function writableStreamDefaultWriterAbort(writer, reason) {
  const {
    stream,
  } = writer[kState];
  assert(stream !== undefined);
  return writableStreamAbort(stream, reason);
}

function writableStreamDefaultControllerWrite(controller, chunk, chunkSize) {
  try {
    enqueueValueWithSize(controller, chunk, chunkSize);
  } catch (error) {
    writableStreamDefaultControllerErrorIfNeeded(controller, error);
    return;
  }
  const {
    stream,
  } = controller[kState];
  const streamState = stream[kState];
  if (!streamState.closeQueuedOrInFlight &&
      streamState.state === 'writable') {
    writableStreamUpdateBackpressure(controller, streamState);
  }
  writableStreamDefaultControllerAdvanceQueueIfNeeded(controller);
}

function writableStreamDefaultControllerProcessWrite(controller, chunk) {
  const {
    stream,
    writeAlgorithm,
  } = controller[kState];
  writableStreamMarkFirstWriteRequestInFlight(stream);

  if (controller[kState].writeFulfilled === undefined) {
    // The write reaction closures only capture the stream and controller,
    // so they are created once on the first write and reused for every
    // subsequent write instead of allocating two fresh closures per chunk.
    controller[kState].writeFulfilled = () => {
      writableStreamFinishInFlightWrite(stream);
      const streamState = stream[kState];
      const {
        state,
      } = streamState;
      assert(state === 'writable' || state === 'erroring');
      dequeueValue(controller);
      if (!streamState.closeQueuedOrInFlight &&
          state === 'writable') {
        writableStreamUpdateBackpressure(controller, streamState);
      }
      writableStreamDefaultControllerAdvanceQueueIfNeeded(controller);
    };
    controller[kState].writeRejected = (error) => {
      if (stream[kState].state === 'writable')
        writableStreamDefaultControllerClearAlgorithms(controller);
      writableStreamFinishInFlightWriteWithError(stream, error);
    };
  }

  // The write algorithm may be a raw callback (a wrapped user sink.write
  // returns its result uncoerced; a synchronous throw surfaces here) or an
  // internal algorithm that always returns a promise; thenAlgorithmResult
  // handles both.
  let result;
  try {
    result = writeAlgorithm(chunk, controller);
  } catch (error) {
    result = PromiseReject(error);
  }
  thenAlgorithmResult(
    result,
    controller[kState].writeFulfilled,
    controller[kState].writeRejected);
}

function writableStreamDefaultControllerProcessClose(controller) {
  const {
    closeAlgorithm,
    queue,
    stream,
  } = controller[kState];
  writableStreamMarkCloseRequestInFlight(stream);
  dequeueValue(controller);
  assert(!queue.length);
  const sinkClosePromise = closeAlgorithm();
  writableStreamDefaultControllerClearAlgorithms(controller);
  PromisePrototypeThen(
    sinkClosePromise,
    () => writableStreamFinishInFlightClose(stream),
    (error) => writableStreamFinishInFlightCloseWithError(stream, error));
}

function writableStreamDefaultControllerGetDesiredSize(controller) {
  const {
    highWaterMark,
    queueTotalSize,
  } = controller[kState];
  return highWaterMark - queueTotalSize;
}

function writableStreamDefaultControllerGetChunkSize(controller, chunk) {
  const {
    stream,
    sizeAlgorithm,
  } = controller[kState];
  if (sizeAlgorithm === undefined) {
    assert(stream[kState].state === 'closed' ||
           stream[kState].state === 'errored' ||
           stream[kState].state === 'erroring');
    return 1;
  }

  // The internal default size algorithm is never observable by user
  // code, always returns 1, and cannot throw: skip the call.
  if (sizeAlgorithm === defaultSizeAlgorithm)
    return 1;

  try {
    return FunctionPrototypeCall(
      sizeAlgorithm,
      undefined,
      chunk);
  } catch (error) {
    writableStreamDefaultControllerErrorIfNeeded(controller, error);
    return 1;
  }
}

function writableStreamDefaultControllerErrorIfNeeded(controller, error) {
  const {
    stream,
  } = controller[kState];
  if (stream[kState].state === 'writable')
    writableStreamDefaultControllerError(controller, error);
}

function writableStreamDefaultControllerError(controller, error) {
  const {
    stream,
  } = controller[kState];
  assert(stream[kState].state === 'writable');
  writableStreamDefaultControllerClearAlgorithms(controller);
  writableStreamStartErroring(stream, error);
}

function writableStreamDefaultControllerClose(controller) {
  enqueueValueWithSize(controller, kCloseSentinel, 0);
  writableStreamDefaultControllerAdvanceQueueIfNeeded(controller);
}

function writableStreamDefaultControllerClearAlgorithms(controller) {
  controller[kState].writeAlgorithm = undefined;
  controller[kState].closeAlgorithm = undefined;
  controller[kState].abortAlgorithm = undefined;
  controller[kState].sizeAlgorithm = undefined;
}

function writableStreamDefaultControllerAdvanceQueueIfNeeded(controller) {
  const {
    queue,
    started,
    stream,
  } = controller[kState];
  if (!started || stream[kState].inFlightWriteRequest.promise !== undefined)
    return;

  if (stream[kState].state === 'erroring') {
    writableStreamFinishErroring(stream);
    return;
  }

  if (!queue.length)
    return;

  const value = peekQueueValue(controller);
  if (value === kCloseSentinel)
    writableStreamDefaultControllerProcessClose(controller);
  else
    writableStreamDefaultControllerProcessWrite(controller, value);
}

function setupWritableStreamDefaultControllerFromSink(
  stream,
  sink,
  highWaterMark,
  sizeAlgorithm) {
  const controller = new WritableStreamDefaultController(kSkipThrow);
  const start = sink?.start;
  const write = sink?.write;
  const close = sink?.close;
  const abort = sink?.abort;
  const startAlgorithm = start ?
    FunctionPrototypeBind(start, sink, controller) :
    nonOpCallback;
  const writeAlgorithm = write ?
    createRawCallback2Params('sink.write', write, sink) :
    nonOpCallback;
  const closeAlgorithm = close ?
    createPromiseCallbackNoParams('sink.close', close, sink) :
    nonOpCancel;
  const abortAlgorithm = abort ?
    createPromiseCallback1Param('sink.abort', abort, sink) :
    nonOpCancel;
  setupWritableStreamDefaultController(
    stream,
    controller,
    startAlgorithm,
    writeAlgorithm,
    closeAlgorithm,
    abortAlgorithm,
    highWaterMark,
    sizeAlgorithm);
}

function setupWritableStreamDefaultController(
  stream,
  controller,
  startAlgorithm,
  writeAlgorithm,
  closeAlgorithm,
  abortAlgorithm,
  highWaterMark,
  sizeAlgorithm) {
  assert(isWritableStream(stream));
  assert(stream[kState].controller === undefined);
  controller[kState] = {
    abortAlgorithm,
    closeAlgorithm,
    highWaterMark,
    queue: kEmptyQueue,
    queueTotalSize: 0,
    abortController: new AbortController(),
    sizeAlgorithm,
    started: false,
    stream,
    writeAlgorithm,
    writeFulfilled: undefined,
    writeRejected: undefined,
  };
  stream[kState].controller = controller;

  writableStreamUpdateBackpressure(controller, stream[kState]);

  const startResult = startAlgorithm();

  const started = () => {
    assert(stream[kState].state === 'writable' ||
           stream[kState].state === 'erroring');
    controller[kState].started = true;
    writableStreamDefaultControllerAdvanceQueueIfNeeded(controller);
  };

  if (startResult === null ||
      (typeof startResult !== 'object' && typeof startResult !== 'function')) {
    // Non-thenable start result: fulfillment is guaranteed and no .then
    // lookup on the result is observable, so the post-start step runs at
    // the exact microtask position the promise reaction would have had.
    queueMicrotask(started);
    return;
  }

  // The wrapper promise matches the reference implementation's
  // promiseResolvedWith(), whose extra microtask hops WPT relies on.
  PromisePrototypeThen(
    new Promise((r) => r(startResult)),
    started,
    (error) => {
      assert(stream[kState].state === 'writable' ||
             stream[kState].state === 'erroring');
      controller[kState].started = true;
      writableStreamDealWithRejection(stream, error);
    });
}

module.exports = {
  WritableStream,
  WritableStreamDefaultWriter,
  WritableStreamDefaultController,
  TransferredWritableStream,

  // Exported Brand Checks
  isWritableStream,
  isWritableStreamDefaultController,
  isWritableStreamDefaultWriter,

  isWritableStreamLocked,
  setupWritableStreamDefaultWriter,
  writableStreamAbort,
  writableStreamClose,
  writableStreamUpdateBackpressure,
  writableStreamStartErroring,
  writableStreamRejectCloseAndClosedPromiseIfNeeded,
  writableStreamMarkFirstWriteRequestInFlight,
  writableStreamMarkCloseRequestInFlight,
  writableStreamHasOperationMarkedInFlight,
  writableStreamFinishInFlightWriteWithError,
  writableStreamFinishInFlightWrite,
  writableStreamFinishInFlightCloseWithError,
  writableStreamFinishInFlightClose,
  writableStreamFinishErroring,
  writableStreamDealWithRejection,
  writableStreamCloseQueuedOrInFlight,
  writableStreamAddWriteRequest,
  writableStreamDefaultWriterWrite,
  writableStreamDefaultWriterWriteWithRequest,
  writableStreamDefaultWriterRelease,
  writableStreamDefaultWriterGetDesiredSize,
  writableStreamDefaultWriterEnsureReadyPromiseRejected,
  writableStreamDefaultWriterEnsureClosedPromiseRejected,
  writerClosedPromise,
  writerReadyPromise,
  writableStreamDefaultWriterCloseWithErrorPropagation,
  writableStreamDefaultWriterClose,
  writableStreamDefaultWriterAbort,
  writableStreamDefaultControllerWrite,
  writableStreamDefaultControllerProcessWrite,
  writableStreamDefaultControllerProcessClose,
  writableStreamDefaultControllerGetDesiredSize,
  writableStreamDefaultControllerGetChunkSize,
  writableStreamDefaultControllerErrorIfNeeded,
  writableStreamDefaultControllerError,
  writableStreamDefaultControllerClose,
  writableStreamDefaultControllerClearAlgorithms,
  writableStreamDefaultControllerAdvanceQueueIfNeeded,
  setupWritableStreamDefaultControllerFromSink,
  setupWritableStreamDefaultController,
  createWritableStream,
};
});

// ── lib/internal/webstreams/readablestream.js ──────────────────────────────────────────────
define('internal/webstreams/readablestream', (module, exports, require, primordials, internalBinding) => {
'use strict';

const {
  ArrayBuffer,
  ArrayBufferPrototypeGetByteLength,
  ArrayBufferPrototypeGetDetached,
  ArrayBufferPrototypeSlice,
  ArrayBufferPrototypeTransfer,
  ArrayPrototypePush,
  AsyncIteratorPrototype,
  DataView,
  FunctionPrototypeBind,
  FunctionPrototypeCall,
  MathMin,
  NumberIsInteger,
  ObjectDefineProperties,
  ObjectSetPrototypeOf,
  Promise,
  PromisePrototypeThen,
  PromiseReject,
  PromiseResolve,
  PromiseWithResolvers,
  SafePromiseAll,
  Symbol,
  SymbolAsyncIterator,
  SymbolDispose,
  SymbolIterator,
  SymbolToStringTag,
  TypedArrayPrototypeGetLength,
  Uint8Array,
} = primordials;

const {
  AbortError,
  codes: {
    ERR_ARG_NOT_ITERABLE,
    ERR_ILLEGAL_CONSTRUCTOR,
    ERR_INVALID_ARG_TYPE,
    ERR_INVALID_ARG_VALUE,
    ERR_INVALID_STATE,
    ERR_INVALID_THIS,
    ERR_OUT_OF_RANGE,
  },
} = require('internal/errors');

const {
  DOMException,
} = internalBinding('messaging');

const {
  isArrayBufferView,
  isDataView,
  isSharedArrayBuffer,
} = require('internal/util/types');

const {
  customInspectSymbol: kInspect,
  kEmptyObject,
  kEnumerableProperty,
  SideEffectFreeRegExpPrototypeSymbolReplace,
} = require('internal/util');

const {
  validateAbortSignal,
  validateBuffer,
  validateNumber,
  validateObject,
  kValidateObjectAllowObjects,
  kValidateObjectAllowObjectsAndNull,
} = require('internal/validators');

const {
  MessageChannel,
} = require('internal/worker/io');

const {
  kDeserialize,
  kTransfer,
  kTransferList,
  markTransferMode,
  structuredClone,
} = require('internal/worker/js_transferable');

const {
  queueMicrotask,
} = require('internal/process/task_queues');

const {
  kIsDisturbed,
  kIsErrored,
  kIsReadable,
  kIsClosedPromise,
  kControllerErrorFunction,
} = require('internal/streams/utils');

const {
  ArrayBufferViewGetBuffer,
  ArrayBufferViewGetByteLength,
  ArrayBufferViewGetByteOffset,
  Queue,
  canCopyArrayBuffer,
  cloneAsUint8Array,
  copyArrayBuffer,
  createPromiseCallback1Param,
  createRawCallback1Param,
  customInspect,
  defaultSizeAlgorithm,
  dequeueValue,
  enqueueValueWithSize,
  extractHighWaterMark,
  extractSizeAlgorithm,
  getNonWritablePropertyDescriptor,
  isBrandCheck,
  kEmptyQueue,
  kParkedAlgorithmResult,
  kPendingPromise,
  kResolvedPromise,
  kState,
  kType,
  lazyTransfer,
  materializeQueue,
  nonOpCallback,
  nonOpCancel,
  rejectedHandledRecord,
  resetQueue,
  resolvedRecord,
  setPromiseHandled,
  thenAlgorithmResult,
} = require('internal/webstreams/util');

const {
  WritableStreamDefaultWriter,

  isWritableStream,
  isWritableStreamLocked,
  isWritableStreamDefaultController,
  isWritableStreamDefaultWriter,

  writableStreamAbort,
  writableStreamCloseQueuedOrInFlight,
  writableStreamDefaultWriterCloseWithErrorPropagation,
  writableStreamDefaultWriterRelease,
  writableStreamDefaultWriterWriteWithRequest,
} = require('internal/webstreams/writablestream');

const { Buffer } = require('buffer');

const assert = require('internal/assert');

const kCancel = Symbol('kCancel');
const kClose = Symbol('kClose');
const kChunk = Symbol('kChunk');
const kError = Symbol('kError');
const kPull = Symbol('kPull');
const kRelease = Symbol('kRelease');
const kSkipThrow = Symbol('kSkipThrow');

let releasedError;
let releasingError;
let addAbortListener;

const userModuleRegExp = /^ {4}at (?:[^/\\(]+ \()(?!node:(.+):\d+:\d+\)$).*/gm;

function lazyReadableReleasedError() {
  if (releasedError) {
    return releasedError;
  }

  releasedError = new ERR_INVALID_STATE.TypeError('Reader released');
  // Avoid V8 leak and remove userland stackstrace
  releasedError.stack = SideEffectFreeRegExpPrototypeSymbolReplace(userModuleRegExp, releasedError.stack, '');
  return releasedError;
}

function lazyReadableReleasingError() {
  if (releasingError) {
    return releasingError;
  }
  releasingError = new ERR_INVALID_STATE.TypeError('Releasing reader');
  // Avoid V8 leak and remove userland stackstrace
  releasingError.stack = SideEffectFreeRegExpPrototypeSymbolReplace(userModuleRegExp, releasingError.stack, '');
  return releasingError;
}

/**
 * @typedef {import('../abort_controller').AbortSignal} AbortSignal
 * @typedef {import('./queuingstrategies').QueuingStrategy} QueuingStrategy
 * @typedef {import('./queuingstrategies').QueuingStrategySize
 * } QueuingStrategySize
 * @typedef {import('./writablestream').WritableStream} WritableStream
 */

/**
 * @typedef {ReadableStreamDefaultController | ReadableByteStreamController
 * } ReadableStreamController
 */

/**
 * @typedef {ReadableStreamDefaultReader | ReadableStreamBYOBReader
 * } ReadableStreamReader
 */

/**
 * @callback UnderlyingSourceStartCallback
 * @param {ReadableStreamController} controller
 * @returns { any | Promise<void> }
 */

/**
 * @callback UnderlyingSourcePullCallback
 * @param {ReadableStreamController} controller
 * @returns { Promise<void> }
 */

/**
 * @callback UnderlyingSourceCancelCallback
 * @param {any} reason
 * @returns { Promise<void> }
 */

/**
 * @typedef {{
 *   readable: ReadableStream,
 *   writable: WritableStream,
 * }} ReadableWritablePair
 */

/**
 * @typedef {{
 *   preventClose? : boolean,
 *   preventAbort? : boolean,
 *   preventCancel? : boolean,
 *   signal? : AbortSignal,
 * }} StreamPipeOptions
 */

/**
 * @typedef {{
 *   start? : UnderlyingSourceStartCallback,
 *   pull? : UnderlyingSourcePullCallback,
 *   cancel? : UnderlyingSourceCancelCallback,
 *   type? : "bytes",
 *   autoAllocateChunkSize? : number
 * }} UnderlyingSource
 */

class ReadableStream {
  [kType] = 'ReadableStream';

  /**
   * @param {UnderlyingSource} [source]
   * @param {QueuingStrategy} [strategy]
   */
  constructor(source = kEmptyObject, strategy = kEmptyObject) {
    markTransferMode(this, false, true);
    // Internal construction (tee, transform streams, adapters, transfer):
    // the caller sets up the controller, so every ReadableStream shares
    // one hidden class and no per-instance prototype swap is needed.
    if (source === kSkipThrow) {
      this[kState] = createReadableStreamState();
      return;
    }
    validateObject(source, 'source', kValidateObjectAllowObjects);
    validateObject(strategy, 'strategy', kValidateObjectAllowObjectsAndNull);
    this[kState] = createReadableStreamState();

    // The spec requires handling of the strategy first
    // here. Specifically, if getting the size and
    // highWaterMark from the strategy fail, that has
    // to trigger a throw before getting the details
    // from the source. So be sure to keep these in
    // this order.
    const size = strategy?.size;
    const highWaterMark = strategy?.highWaterMark;
    const type = source.type;

    if (`${type}` === 'bytes') {
      if (size !== undefined)
        throw new ERR_INVALID_ARG_VALUE.RangeError('strategy.size', size);
      setupReadableByteStreamControllerFromSource(
        this,
        source,
        extractHighWaterMark(highWaterMark, 0));
    } else {
      if (type !== undefined)
        throw new ERR_INVALID_ARG_VALUE('source.type', type);
      setupReadableStreamDefaultControllerFromSource(
        this,
        source,
        extractHighWaterMark(highWaterMark, 1),
        extractSizeAlgorithm(size));
    }
  }

  get [kIsDisturbed]() {
    return this[kState].disturbed;
  }

  get [kIsErrored]() {
    return this[kState].state === 'errored';
  }

  get [kIsReadable]() {
    return this[kState].state === 'readable';
  }

  [kControllerErrorFunction](error) {
    // Used by the internal stream interop (addAbortSignal). Historically
    // only default controllers were wired here; byte stream controllers
    // keep the previous no-op behavior.
    const controller = this[kState].controller;
    if (isReadableStreamDefaultController(controller))
      controller.error(error);
  }

  // Used by the internal stream interop (end-of-stream). Materialized
  // lazily since its settlement is derivable from the stream state; the
  // settle sites in readableStreamClose/Error only touch the cache.
  get [kIsClosedPromise]() {
    let cache = this[kState].closedPromise;
    if (cache === undefined) {
      switch (this[kState].state) {
        case 'readable':
          cache = PromiseWithResolvers();
          break;
        case 'closed':
          cache = resolvedRecord();
          break;
        default:
          cache = rejectedHandledRecord(this[kState].storedError);
      }
      this[kState].closedPromise = cache;
    }
    return cache;
  }

  /**
   * @readonly
   * @type {boolean}
   */
  get locked() {
    if (!isReadableStream(this))
      throw new ERR_INVALID_THIS('ReadableStream');
    return isReadableStreamLocked(this);
  }

  static from(iterable) {
    return readableStreamFromIterable(iterable);
  }

  /**
   * @param {any} [reason]
   * @returns { Promise<void> }
   */
  cancel(reason = undefined) {
    if (!isReadableStream(this))
      return PromiseReject(new ERR_INVALID_THIS('ReadableStream'));
    if (isReadableStreamLocked(this)) {
      return PromiseReject(
        new ERR_INVALID_STATE.TypeError('ReadableStream is locked'));
    }
    return readableStreamCancel(this, reason);
  }

  /**
   * @param {{
   *   mode? : "byob"
   * }} [options]
   * @returns {ReadableStreamReader}
   */
  getReader(options = kEmptyObject) {
    if (!isReadableStream(this))
      throw new ERR_INVALID_THIS('ReadableStream');
    validateObject(options, 'options', kValidateObjectAllowObjectsAndNull);
    const mode = options?.mode;

    if (mode === undefined)
      // eslint-disable-next-line no-use-before-define
      return new ReadableStreamDefaultReader(this);

    if (`${mode}` !== 'byob')
      throw new ERR_INVALID_ARG_VALUE('options.mode', mode);
    // eslint-disable-next-line no-use-before-define
    return new ReadableStreamBYOBReader(this);
  }

  /**
   * @param {ReadableWritablePair} transform
   * @param {StreamPipeOptions} [options]
   * @returns {ReadableStream}
   */
  pipeThrough(transform, options = kEmptyObject) {
    if (!isReadableStream(this))
      throw new ERR_INVALID_THIS('ReadableStream');
    const readable = transform?.readable;
    if (!isReadableStream(readable)) {
      throw new ERR_INVALID_ARG_TYPE(
        'transform.readable',
        'ReadableStream',
        readable);
    }
    const writable = transform?.writable;
    if (!isWritableStream(writable)) {
      throw new ERR_INVALID_ARG_TYPE(
        'transform.writable',
        'WritableStream',
        writable);
    }

    // The web platform tests require that these be handled one at a
    // time and in a specific order. options can be null or undefined.
    validateObject(options, 'options', kValidateObjectAllowObjectsAndNull);
    const preventAbort = options?.preventAbort;
    const preventCancel = options?.preventCancel;
    const preventClose = options?.preventClose;
    const signal = options?.signal;

    if (signal !== undefined) {
      validateAbortSignal(signal, 'options.signal');
    }

    if (isReadableStreamLocked(this))
      throw new ERR_INVALID_STATE.TypeError('The ReadableStream is locked');
    if (isWritableStreamLocked(writable))
      throw new ERR_INVALID_STATE.TypeError('The WritableStream is locked');

    const promise = readableStreamPipeTo(
      this,
      writable,
      !!preventClose,
      !!preventAbort,
      !!preventCancel,
      signal);
    setPromiseHandled(promise);

    return readable;
  }

  /**
   * @param {WritableStream} destination
   * @param {StreamPipeOptions} [options]
   * @returns {Promise<void>}
   */
  pipeTo(destination, options = kEmptyObject) {
    try {
      if (!isReadableStream(this))
        throw new ERR_INVALID_THIS('ReadableStream');
      if (!isWritableStream(destination)) {
        throw new ERR_INVALID_ARG_TYPE(
          'transform.writable',
          'WritableStream',
          destination);
      }

      validateObject(options, 'options', kValidateObjectAllowObjectsAndNull);
      const preventAbort = options?.preventAbort;
      const preventCancel = options?.preventCancel;
      const preventClose = options?.preventClose;
      const signal = options?.signal;

      if (signal !== undefined) {
        validateAbortSignal(signal, 'options.signal');
      }

      if (isReadableStreamLocked(this))
        throw new ERR_INVALID_STATE.TypeError('The ReadableStream is locked');
      if (isWritableStreamLocked(destination))
        throw new ERR_INVALID_STATE.TypeError('The WritableStream is locked');

      return readableStreamPipeTo(
        this,
        destination,
        !!preventClose,
        !!preventAbort,
        !!preventCancel,
        signal);
    } catch (error) {
      return PromiseReject(error);
    }
  }

  /**
   * @returns {ReadableStream[]}
   */
  tee() {
    if (!isReadableStream(this))
      throw new ERR_INVALID_THIS('ReadableStream');
    return readableStreamTee(this, false);
  }

  /**
   * @param {{
   *   preventCancel? : boolean,
   * }} [options]
   * @returns {AsyncIterable}
   */
  values(options = kEmptyObject) {
    if (!isReadableStream(this))
      throw new ERR_INVALID_THIS('ReadableStream');
    validateObject(options, 'options', kValidateObjectAllowObjectsAndNull);
    const preventCancel = !!(options?.preventCancel);

    // eslint-disable-next-line no-use-before-define
    const reader = new ReadableStreamDefaultReader(this);
    // eslint-disable-next-line no-use-before-define
    const state = new ReadableStreamAsyncIteratorReadRequest(reader, preventCancel);
    // eslint-disable-next-line no-use-before-define
    return new ReadableStreamAsyncIterator(state);
  }

  [kInspect](depth, options) {
    return customInspect(depth, options, this[kType], {
      locked: this.locked,
      state: this[kState].state,
      supportsBYOB:
        // eslint-disable-next-line no-use-before-define
        this[kState].controller instanceof ReadableByteStreamController,
    });
  }

  [kTransfer]() {
    if (!isReadableStream(this))
      throw new ERR_INVALID_THIS('ReadableStream');
    if (this.locked) {
      this[kState].transfer.port1?.close();
      this[kState].transfer.port1 = undefined;
      this[kState].transfer.port2 = undefined;
      throw new DOMException(
        'Cannot transfer a locked ReadableStream',
        'DataCloneError');
    }

    const {
      writable,
      promise,
    } = lazyTransfer().newCrossRealmWritableSink(
      this,
      this[kState].transfer.port1);

    this[kState].transfer.writable = writable;
    this[kState].transfer.promise = promise;

    return {
      data: { port: this[kState].transfer.port2 },
      deserializeInfo:
        'internal/webstreams/readablestream:TransferredReadableStream',
    };
  }

  [kTransferList]() {
    const { port1, port2 } = new MessageChannel();
    this[kState].transfer.port1 = port1;
    this[kState].transfer.port2 = port2;
    return [ port2 ];
  }

  [kDeserialize]({ port }) {
    const transfer = lazyTransfer();
    setupReadableStreamDefaultControllerFromSource(
      this,
      // The MessagePort is set to be referenced when reading.
      // After two MessagePorts are closed, there is a problem with
      // lingering promise not being properly resolved.
      // https://github.com/nodejs/node/issues/51486
      new transfer.CrossRealmTransformReadableSource(port, true),
      0, defaultSizeAlgorithm);
  }
}

ObjectDefineProperties(ReadableStream.prototype, {
  [SymbolAsyncIterator]: {
    __proto__: null,
    configurable: true,
    enumerable: false,
    writable: true,
    value: ReadableStream.prototype.values,
  },
  locked: kEnumerableProperty,
  cancel: kEnumerableProperty,
  getReader: kEnumerableProperty,
  pipeThrough: kEnumerableProperty,
  pipeTo: kEnumerableProperty,
  tee: kEnumerableProperty,
  values: kEnumerableProperty,
  [SymbolToStringTag]: getNonWritablePropertyDescriptor(ReadableStream.name),
});
ObjectDefineProperties(ReadableStream, {
  from: kEnumerableProperty,
});

function TransferredReadableStream() {
  return new ReadableStream(kSkipThrow);
}

TransferredReadableStream.prototype[kDeserialize] = () => {};

class ReadableStreamBYOBRequest {
  [kType] = 'ReadableStreamBYOBRequest';

  constructor(skipThrowSymbol = undefined) {
    if (skipThrowSymbol !== kSkipThrow) {
      throw new ERR_ILLEGAL_CONSTRUCTOR();
    }
  }

  /**
   * @readonly
   * @type {Uint8Array}
   */
  get view() {
    if (!isReadableStreamBYOBRequest(this))
      throw new ERR_INVALID_THIS('ReadableStreamBYOBRequest');
    return this[kState].view;
  }

  /**
   * @param {number} bytesWritten
   */
  respond(bytesWritten) {
    if (!isReadableStreamBYOBRequest(this))
      throw new ERR_INVALID_THIS('ReadableStreamBYOBRequest');
    const {
      view,
      controller,
    } = this[kState];
    if (controller === undefined) {
      throw new ERR_INVALID_STATE.TypeError(
        'This BYOB request has been invalidated');
    }

    const viewByteLength = ArrayBufferViewGetByteLength(view);
    const viewBuffer = ArrayBufferViewGetBuffer(view);
    const viewBufferByteLength = ArrayBufferPrototypeGetByteLength(viewBuffer);

    if (ArrayBufferPrototypeGetDetached(viewBuffer)) {
      throw new ERR_INVALID_STATE.TypeError('Viewed ArrayBuffer is detached');
    }

    assert(viewByteLength > 0);
    assert(viewBufferByteLength > 0);

    readableByteStreamControllerRespond(controller, bytesWritten);
  }

  /**
   * @param {ArrayBufferView} view
   */
  respondWithNewView(view) {
    if (!isReadableStreamBYOBRequest(this))
      throw new ERR_INVALID_THIS('ReadableStreamBYOBRequest');
    const {
      controller,
    } = this[kState];

    if (controller === undefined) {
      throw new ERR_INVALID_STATE.TypeError(
        'This BYOB request has been invalidated');
    }

    validateBuffer(view, 'view');

    if (ArrayBufferPrototypeGetDetached(view.buffer)) {
      throw new ERR_INVALID_STATE.TypeError('Viewed ArrayBuffer is detached');
    }

    readableByteStreamControllerRespondWithNewView(controller, view);
  }

  [kInspect](depth, options) {
    return customInspect(depth, options, this[kType], {
      view: this.view,
      controller: this[kState].controller,
    });
  }
}

ObjectDefineProperties(ReadableStreamBYOBRequest.prototype, {
  view: kEnumerableProperty,
  respond: kEnumerableProperty,
  respondWithNewView: kEnumerableProperty,
  [SymbolToStringTag]: getNonWritablePropertyDescriptor(ReadableStreamBYOBRequest.name),
});

function createReadableStreamBYOBRequest(controller, view) {
  const stream = new ReadableStreamBYOBRequest(kSkipThrow);

  stream[kState] = {
    controller,
    view,
  };

  return stream;
}

// Per-iterator state. It doubles as the iterator's read request: at most
// one read is ever in flight (next() chains through `current`), and the
// request is consumed before the next read starts, so only its promise
// record changes per read.
class ReadableStreamAsyncIteratorReadRequest {
  constructor(reader, preventCancel) {
    this.reader = reader;
    this.preventCancel = preventCancel;
    this.done = false;
    this.started = false;
    this.current = undefined;
    this.promise = undefined;
    this.chainedNextSteps = undefined;
  }

  [kChunk](chunk) {
    this.current = undefined;
    this.promise.resolve({ done: false, value: chunk });
  }

  [kClose]() {
    this.current = undefined;
    this.done = true;
    readableStreamReaderGenericRelease(this.reader);
    this.promise.resolve({ done: true, value: undefined });
  }

  [kError](error) {
    this.current = undefined;
    this.done = true;
    readableStreamReaderGenericRelease(this.reader);
    this.promise.reject(error);
  }
}

// next() is not an async function: it explicitly creates and returns a
// promise in the common case, so an async function would only add two
// promise allocations.
function readableStreamAsyncIteratorNextSteps(state) {
  if (state.done)
    return PromiseResolve({ done: true, value: undefined });

  const reader = state.reader;
  if (reader[kState].stream === undefined) {
    return PromiseReject(
      new ERR_INVALID_STATE.TypeError(
        'The reader is not bound to a ReadableStream'));
  }
  const promise = PromiseWithResolvers();

  state.promise = promise;
  readableStreamDefaultReaderRead(reader, state);
  return promise.promise;
}

// Not an async function either: the cancel path settles one microtask
// after the cancel promise, exactly as `await` would.
function readableStreamAsyncIteratorReturnSteps(state, value) {
  const iterResult = { done: true, value };
  if (state.done)
    return PromiseResolve(iterResult);
  state.done = true;

  try {
    const reader = state.reader;
    if (reader[kState].stream === undefined) {
      throw new ERR_INVALID_STATE.TypeError(
        'The reader is not bound to a ReadableStream');
    }
    assert(!reader[kState].readRequests.length);
    if (!state.preventCancel) {
      const result = readableStreamReaderGenericCancel(reader, value);
      readableStreamReaderGenericRelease(reader);
      return PromisePrototypeThen(result, () => iterResult);
    }

    readableStreamReaderGenericRelease(reader);
    return PromiseResolve(iterResult);
  } catch (error) {
    return PromiseReject(error);
  }
}

// The methods live on a shared prototype, as for any WebIDL async iterator,
// rather than being created per iterator. Neither method may be an async
// function: `await` goes through Promise.prototype.then, which the streams
// WPTs patch.
class ReadableStreamAsyncIterator {
  #state;

  constructor(state) {
    this.#state = state;
  }

  next() {
    if (typeof this !== 'object' || this === null || !(#state in this)) {
      return PromiseReject(
        new ERR_INVALID_THIS('ReadableStreamAsyncIterator'));
    }
    const state = this.#state;
    // If this is the first read, delay by one microtask
    // to ensure that the controller has had an opportunity
    // to properly start and perform the initial pull.
    // TODO(@jasnell): The spec doesn't call this out so
    // need to investigate if it's a bug in our impl or
    // the spec.
    if (!state.started) {
      state.current = kResolvedPromise;
      state.started = true;
    }
    if (state.current !== undefined) {
      let steps = state.chainedNextSteps;
      if (steps === undefined) {
        steps = state.chainedNextSteps =
          () => readableStreamAsyncIteratorNextSteps(state);
      }
      state.current = PromisePrototypeThen(state.current, steps, steps);
      return state.current;
    }
    // No read is in flight. Mirror the buffered fast path of
    // ReadableStreamDefaultReader.read(): when data is already queued
    // in the controller, resolve immediately without allocating a
    // read request. The result settles synchronously, so leaving
    // state.current undefined matches the state the slow path reaches
    // once its read request callbacks have settled.
    const stream = state.reader[kState].stream;
    if (!state.done && stream !== undefined &&
        stream[kState].state === 'readable') {
      const controller = stream[kState].controller;
      if (isReadableStreamDefaultController(controller)) {
        if (controller[kState].queue.length > 0) {
          stream[kState].disturbed = true;
          const chunk = dequeueValue(controller);

          if (controller[kState].closeRequested &&
              !controller[kState].queue.length) {
            readableStreamDefaultControllerClearAlgorithms(controller);
            readableStreamClose(stream);
          } else if (!controller[kState].closeRequested &&
                     controller[kState].started &&
                     controller[kState].highWaterMark -
                       controller[kState].queueTotalSize > 0) {
            // Reduced ShouldCallPull, as in the read() fast path.
            readableStreamDefaultControllerPull(controller);
          }

          return PromiseResolve({ done: false, value: chunk });
        }
      } else if (controller[kState].queueTotalSize > 0) {
        // Byte controller with buffered data: same shape as above via
        // the queue-filled arm of the byte controller's pull steps.
        stream[kState].disturbed = true;
        return PromiseResolve({
          done: false,

          value: readableByteStreamControllerDequeueChunk(controller),
        });
      }
    }
    state.current = readableStreamAsyncIteratorNextSteps(state);
    return state.current;
  }

  return(value) {
    if (typeof this !== 'object' || this === null || !(#state in this)) {
      return PromiseReject(
        new ERR_INVALID_THIS('ReadableStreamAsyncIterator'));
    }
    const state = this.#state;
    state.started = true;
    state.current = state.current !== undefined ?
      PromisePrototypeThen(
        state.current,
        () => readableStreamAsyncIteratorReturnSteps(state, value),
        () => readableStreamAsyncIteratorReturnSteps(state, value)) :
      readableStreamAsyncIteratorReturnSteps(state, value);
    return state.current;
  }
}

delete ReadableStreamAsyncIterator.prototype.constructor;
ObjectSetPrototypeOf(ReadableStreamAsyncIterator.prototype,
                     AsyncIteratorPrototype);
ObjectDefineProperties(ReadableStreamAsyncIterator.prototype, {
  next: kEnumerableProperty,
  return: kEnumerableProperty,
  [SymbolToStringTag]:
    getNonWritablePropertyDescriptor('ReadableStream AsyncIterator'),
});

class DefaultReadRequest {
  constructor() {
    this[kState] = PromiseWithResolvers();
  }

  [kChunk](value) {
    this[kState].resolve({ done: false, value });
  }

  [kClose]() {
    this[kState].resolve({ done: true, value: undefined });
  }

  [kError](error) {
    this[kState].reject(error);
  }

  get promise() { return this[kState].promise; }
}

// Read (into) request record for the internal consumers (pipeTo, tee).
// The step functions are per-consumer closures, but the record is a class
// instance rather than an object literal: a literal with computed symbol
// keys is rebuilt through the runtime on every evaluation, which costs
// microseconds per pipe or tee (and per read in the byte tee's BYOB path).
class StepsReadRequest {
  constructor(chunkSteps, closeSteps, errorSteps) {
    this.chunkSteps = chunkSteps;
    this.closeSteps = closeSteps;
    this.errorSteps = errorSteps;
  }

  [kChunk](chunk) {
    this.chunkSteps(chunk);
  }

  [kClose](chunk) {
    this.closeSteps(chunk);
  }

  [kError](error) {
    this.errorSteps(error);
  }
}

class ReadIntoRequest {
  constructor() {
    this[kState] = PromiseWithResolvers();
  }

  [kChunk](value) {
    this[kState].resolve({ done: false, value });
  }

  [kClose](value) {
    this[kState].resolve({ done: true, value });
  }

  [kError](error) {
    this[kState].reject(error);
  }

  get promise() { return this[kState].promise; }
}

class ReadableStreamDefaultReader {
  [kType] = 'ReadableStreamDefaultReader';

  /**
   * @param {ReadableStream} stream
   */
  constructor(stream) {
    if (!isReadableStream(stream))
      throw new ERR_INVALID_ARG_TYPE('stream', 'ReadableStream', stream);
    this[kState] = {
      // All fields are unconditionally assigned during setup.
      readRequests: undefined,
      stream: undefined,
      close: undefined,
    };
    setupReadableStreamDefaultReader(this, stream);
  }

  /**
   * @returns {Promise<{
   *   value : any,
   *   done : boolean
   * }>}
   */
  read() {
    if (!isReadableStreamDefaultReader(this))
      return PromiseReject(new ERR_INVALID_THIS('ReadableStreamDefaultReader'));
    if (this[kState].stream === undefined) {
      return PromiseReject(
        new ERR_INVALID_STATE.TypeError(
          'The reader is not attached to a stream'));
    }

    const stream = this[kState].stream;
    const controller = stream[kState].controller;

    // Fast path: if data is already buffered in the controller's queue,
    // return a resolved promise immediately without creating a read request.
    // This is spec-compliant because read() returns a Promise, and
    // Promise.resolve() callbacks still run in the microtask queue.
    if (stream[kState].state === 'readable') {
      if (isReadableStreamDefaultController(controller)) {
        if (controller[kState].queue.length > 0) {
          stream[kState].disturbed = true;
          const chunk = dequeueValue(controller);

          if (controller[kState].closeRequested && !controller[kState].queue.length) {
            readableStreamDefaultControllerClearAlgorithms(controller);
            readableStreamClose(stream);
          } else if (!controller[kState].closeRequested &&
                     controller[kState].started &&
                     controller[kState].highWaterMark -
                       controller[kState].queueTotalSize > 0) {
            // ShouldCallPull reduced to the conditions not already
            // established on this path: the state is readable and the
            // queue was non-empty, so no read requests can be parked.
            readableStreamDefaultControllerPull(controller);
          }

          return PromiseResolve({ done: false, value: chunk });
        }
      } else if (controller[kState].queueTotalSize > 0) {
        // Byte controller with buffered data: mirror the queue-filled arm
        // of its pull steps (which never consults pendingPullIntos) minus
        // the read request.
        stream[kState].disturbed = true;
        return PromiseResolve({
          done: false,

          value: readableByteStreamControllerDequeueChunk(controller),
        });
      }
    }

    // Slow path: create request and go through normal flow
    const readRequest = new DefaultReadRequest();
    readableStreamDefaultReaderRead(this, readRequest);
    return readRequest.promise;
  }

  releaseLock() {
    if (!isReadableStreamDefaultReader(this))
      throw new ERR_INVALID_THIS('ReadableStreamDefaultReader');
    if (this[kState].stream === undefined)
      return;
    readableStreamDefaultReaderRelease(this);
  }

  /**
   * @readonly
   * @type {Promise<void>}
   */
  get closed() {
    if (!isReadableStreamDefaultReader(this))
      return PromiseReject(new ERR_INVALID_THIS('ReadableStreamDefaultReader'));
    return readerClosedPromise(this).promise;
  }

  /**
   * @param {any} [reason]
   * @returns {Promise<void>}
   */
  cancel(reason = undefined) {
    if (!isReadableStreamDefaultReader(this))
      return PromiseReject(new ERR_INVALID_THIS('ReadableStreamDefaultReader'));
    if (this[kState].stream === undefined) {
      return PromiseReject(new ERR_INVALID_STATE.TypeError(
        'The reader is not attached to a stream'));
    }
    return readableStreamReaderGenericCancel(this, reason);
  }

  [kInspect](depth, options) {
    return customInspect(depth, options, this[kType], {
      stream: this[kState].stream,
      readRequests: this[kState].readRequests.length,
      close: readerClosedPromise(this).promise,
    });
  }
}

ObjectDefineProperties(ReadableStreamDefaultReader.prototype, {
  closed: kEnumerableProperty,
  read: kEnumerableProperty,
  releaseLock: kEnumerableProperty,
  cancel: kEnumerableProperty,
  [SymbolToStringTag]: getNonWritablePropertyDescriptor(ReadableStreamDefaultReader.name),
});

class ReadableStreamBYOBReader {
  [kType] = 'ReadableStreamBYOBReader';

  /**
   * @param {ReadableStream} stream
   */
  constructor(stream) {
    if (!isReadableStream(stream))
      throw new ERR_INVALID_ARG_TYPE('stream', 'ReadableStream', stream);
    this[kState] = {
      // All fields are unconditionally assigned during setup.
      stream: undefined,
      readIntoRequests: undefined,
      close: undefined,
    };
    setupReadableStreamBYOBReader(this, stream);
  }

  /**
   * @param {ArrayBufferView} view
   * @param {{
   *   min? : number
   * }} [options]
   * @returns {Promise<{
   *   value : ArrayBufferView,
   *   done : boolean,
   * }>}
   */
  read(view, options = kEmptyObject) {
    // Returns the read request's promise directly, as the spec does,
    // instead of adopting it through an async wrapper (an extra promise
    // and two microtask hops per read); argument errors become rejections.
    try {
      return readableStreamBYOBReaderReadView(this, view, options);
    } catch (error) {
      return PromiseReject(error);
    }
  }

  releaseLock() {
    if (!isReadableStreamBYOBReader(this))
      throw new ERR_INVALID_THIS('ReadableStreamBYOBReader');
    if (this[kState].stream === undefined)
      return;
    readableStreamBYOBReaderRelease(this);
  }

  /**
   * @readonly
   * @type {Promise<void>}
   */
  get closed() {
    if (!isReadableStreamBYOBReader(this))
      return PromiseReject(new ERR_INVALID_THIS('ReadableStreamBYOBReader'));
    return readerClosedPromise(this).promise;
  }

  /**
   * @param {any} [reason]
   * @returns {Promise<void>}
   */
  cancel(reason = undefined) {
    if (!isReadableStreamBYOBReader(this))
      return PromiseReject(new ERR_INVALID_THIS('ReadableStreamBYOBReader'));
    if (this[kState].stream === undefined) {
      return PromiseReject(new ERR_INVALID_STATE.TypeError(
        'The reader is not attached to a stream'));
    }
    return readableStreamReaderGenericCancel(this, reason);
  }

  [kInspect](depth, options) {
    return customInspect(depth, options, this[kType], {
      stream: this[kState].stream,
      readIntoRequests: this[kState].readIntoRequests.length,
      close: readerClosedPromise(this).promise,
    });
  }
}

ObjectDefineProperties(ReadableStreamBYOBReader.prototype, {
  closed: kEnumerableProperty,
  read: kEnumerableProperty,
  releaseLock: kEnumerableProperty,
  cancel: kEnumerableProperty,
  [SymbolToStringTag]: getNonWritablePropertyDescriptor(ReadableStreamBYOBReader.name),
});

class ReadableStreamDefaultController {
  [kType] = 'ReadableStreamDefaultController';
  [kState];

  constructor(skipThrowSymbol = undefined) {
    if (skipThrowSymbol !== kSkipThrow) {
      throw new ERR_ILLEGAL_CONSTRUCTOR();
    }
  }

  /**
   * @readonly
   * @type {number}
   */
  get desiredSize() {
    return readableStreamDefaultControllerGetDesiredSize(this);
  }

  close() {
    if (!readableStreamDefaultControllerCanCloseOrEnqueue(this))
      throw new ERR_INVALID_STATE.TypeError('Controller is already closed');
    readableStreamDefaultControllerClose(this);
  }

  /**
   * @param {any} [chunk]
   */
  enqueue(chunk = undefined) {
    if (!readableStreamDefaultControllerCanCloseOrEnqueue(this))
      throw new ERR_INVALID_STATE.TypeError('Controller is already closed');
    readableStreamDefaultControllerEnqueue(this, chunk);
  }

  /**
   * @param {any} [error]
   */
  error(error = undefined) {
    readableStreamDefaultControllerError(this, error);
  }

  [kCancel](reason) {
    return readableStreamDefaultControllerCancelSteps(this, reason);
  }

  [kPull](readRequest) {
    readableStreamDefaultControllerPullSteps(this, readRequest);
  }

  [kRelease]() {}

  [kInspect](depth, options) {
    return customInspect(depth, options, this[kType], { });
  }
}

ObjectDefineProperties(ReadableStreamDefaultController.prototype, {
  desiredSize: kEnumerableProperty,
  close: kEnumerableProperty,
  enqueue: kEnumerableProperty,
  error: kEnumerableProperty,
  [SymbolToStringTag]: getNonWritablePropertyDescriptor(ReadableStreamDefaultController.name),
});

class ReadableByteStreamController {
  [kType] = 'ReadableByteStreamController';
  [kState];

  constructor(skipThrowSymbol = undefined) {
    if (skipThrowSymbol !== kSkipThrow) {
      throw new ERR_ILLEGAL_CONSTRUCTOR();
    }
  }

  /**
   * @readonly
   * @type {ReadableStreamBYOBRequest}
   */
  get byobRequest() {
    if (!isReadableByteStreamController(this))
      throw new ERR_INVALID_THIS('ReadableByteStreamController');
    if (this[kState].byobRequest === null &&
        this[kState].pendingPullIntos.length) {
      const {
        buffer,
        byteOffset,
        bytesFilled,
        byteLength,
      } = this[kState].pendingPullIntos.peek();
      const view =
        new Uint8Array(
          buffer,
          byteOffset + bytesFilled,
          byteLength - bytesFilled);
      this[kState].byobRequest = createReadableStreamBYOBRequest(this, view);
    }
    return this[kState].byobRequest;
  }

  /**
   * @readonly
   * @type {number}
   */
  get desiredSize() {
    if (!isReadableByteStreamController(this))
      throw new ERR_INVALID_THIS('ReadableByteStreamController');
    return readableByteStreamControllerGetDesiredSize(this);
  }

  close() {
    if (!isReadableByteStreamController(this))
      throw new ERR_INVALID_THIS('ReadableByteStreamController');
    if (this[kState].closeRequested)
      throw new ERR_INVALID_STATE.TypeError('Controller is already closed');
    if (this[kState].stream[kState].state !== 'readable')
      throw new ERR_INVALID_STATE.TypeError('ReadableStream is already closed');
    readableByteStreamControllerClose(this);
  }

  /**
   * @param {ArrayBufferView} chunk
   */
  enqueue(chunk) {
    if (!isReadableByteStreamController(this))
      throw new ERR_INVALID_THIS('ReadableByteStreamController');
    validateBuffer(chunk);
    const chunkByteLength = ArrayBufferViewGetByteLength(chunk);
    const chunkBuffer = ArrayBufferViewGetBuffer(chunk);

    if (isSharedArrayBuffer(chunkBuffer)) {
      throw new ERR_INVALID_ARG_VALUE(
        'chunk',
        chunk,
        'must not be backed by a SharedArrayBuffer',
      );
    }

    const chunkBufferByteLength = ArrayBufferPrototypeGetByteLength(chunkBuffer);
    if (chunkByteLength === 0 || chunkBufferByteLength === 0) {
      throw new ERR_INVALID_STATE.TypeError(
        'chunk ArrayBuffer is zero-length or detached');
    }
    if (this[kState].closeRequested)
      throw new ERR_INVALID_STATE.TypeError('Controller is already closed');
    if (this[kState].stream[kState].state !== 'readable')
      throw new ERR_INVALID_STATE.TypeError('ReadableStream is already closed');
    readableByteStreamControllerEnqueue(this, chunk);
  }

  /**
   * @param {any} [error]
   */
  error(error = undefined) {
    if (!isReadableByteStreamController(this))
      throw new ERR_INVALID_THIS('ReadableByteStreamController');
    readableByteStreamControllerError(this, error);
  }

  [kCancel](reason) {
    return readableByteStreamControllerCancelSteps(this, reason);
  }

  [kPull](readRequest) {
    readableByteStreamControllerPullSteps(this, readRequest);
  }

  [kRelease]() {
    const {
      pendingPullIntos,
    } = this[kState];
    if (pendingPullIntos.length > 0) {
      const firstPendingPullInto = pendingPullIntos.peek();
      firstPendingPullInto.type = 'none';
      const queue = new Queue();
      queue.push(firstPendingPullInto);
      this[kState].pendingPullIntos = queue;
    }
  }

  [kInspect](depth, options) {
    return customInspect(depth, options, this[kType], { });
  }
}

ObjectDefineProperties(ReadableByteStreamController.prototype, {
  byobRequest: kEnumerableProperty,
  desiredSize: kEnumerableProperty,
  close: kEnumerableProperty,
  enqueue: kEnumerableProperty,
  error: kEnumerableProperty,
  [SymbolToStringTag]: getNonWritablePropertyDescriptor(ReadableByteStreamController.name),
});

function createReadableStream(start, pull, cancel, highWaterMark = 1, size = defaultSizeAlgorithm) {
  const stream = new ReadableStream(kSkipThrow);
  setupReadableStreamDefaultController(
    stream,
    new ReadableStreamDefaultController(kSkipThrow),
    start,
    pull,
    cancel,
    highWaterMark,
    size);
  return stream;
}

function createReadableByteStream(start, pull, cancel) {
  const stream = new ReadableStream(kSkipThrow);
  setupReadableByteStreamController(
    stream,
    new ReadableByteStreamController(kSkipThrow),
    start,
    pull,
    cancel,
    0,
    undefined);
  return stream;
}

const isReadableStream =
  isBrandCheck('ReadableStream');
const isReadableByteStreamController =
  isBrandCheck('ReadableByteStreamController');
const isReadableStreamDefaultController =
  isBrandCheck('ReadableStreamDefaultController');
const isReadableStreamBYOBRequest =
  isBrandCheck('ReadableStreamBYOBRequest');
const isReadableStreamDefaultReader =
  isBrandCheck('ReadableStreamDefaultReader');
const isReadableStreamBYOBReader =
  isBrandCheck('ReadableStreamBYOBReader');

// ---- ReadableStream Implementation

// The state record classes null out their prototype chain to keep
// Object.prototype out of property lookups; every field ever assigned
// is declared so the shape never transitions.
class ReadableStreamTransferState {
  writable = undefined;
  port1 = undefined;
  port2 = undefined;
  promise = undefined;
}
ObjectSetPrototypeOf(ReadableStreamTransferState.prototype, null);

class ReadableStreamState {
  closedPromise = undefined;
  disturbed = false;
  reader = undefined;
  state = 'readable';
  storedError = undefined;
  controller = undefined;
  transfer = new ReadableStreamTransferState();
}
ObjectSetPrototypeOf(ReadableStreamState.prototype, null);

function createReadableStreamState() {
  return new ReadableStreamState();
}

function readableStreamFromIterable(iterable) {
  let stream;
  const iteratorGetter = iterable[SymbolAsyncIterator] ?? iterable[SymbolIterator];
  if (iteratorGetter == null || typeof iteratorGetter !== 'function') {
    throw new ERR_ARG_NOT_ITERABLE(iterable);
  }
  const iterator = FunctionPrototypeCall(iteratorGetter, iterable);
  if (iterator === null || (typeof iterator !== 'object' && typeof iterator !== 'function')) {
    throw new ERR_INVALID_STATE.TypeError('The iterator method must return an object');
  }
  // Per GetIteratorDirect, the next method is looked up once.
  const nextMethod = iterator.next;
  const startAlgorithm = nonOpCallback;

  // Callback-style pull: the reaction steps are reused across chunks and
  // completion is delivered to the controller's cached pull reactions
  // (the kParkedAlgorithmResult contract). One pull runs at a time, so a
  // single slot carries a non-thenable next() result between steps.
  let pendingIterResult;

  function rejectPull(error) {
    readableStreamDefaultControllerError(stream[kState].controller, error);
  }

  function processIterResult(iterResult) {
    const controller = stream[kState].controller;
    if (typeof iterResult !== 'object' || iterResult === null) {
      rejectPull(new ERR_INVALID_STATE.TypeError(
        'The promise returned by the iterator.next() method must fulfill with an object'));
      return;
    }
    try {
      if (iterResult.done) {
        readableStreamDefaultControllerClose(controller);
      } else {
        const value = iterResult.value;
        if (value !== null &&
            (typeof value === 'object' || typeof value === 'function')) {
          // Adopted like `await iterResult.value`, keeping the observable
          // .then lookup on plain objects.
          PromisePrototypeThen(PromiseResolve(value), enqueueValue, rejectPull);
          return;
        }
        readableStreamDefaultControllerEnqueue(controller, value);
      }
    } catch (error) {
      rejectPull(error);
      return;
    }
    // pullFulfilled exists: the controller creates it before the pull.
    controller[kState].pullFulfilled();
  }

  function enqueueValue(value) {
    const controller = stream[kState].controller;
    try {
      readableStreamDefaultControllerEnqueue(controller, value);
    } catch (error) {
      rejectPull(error);
      return;
    }
    controller[kState].pullFulfilled();
  }

  function processPendingIterResult() {
    const iterResult = pendingIterResult;
    pendingIterResult = undefined;
    processIterResult(iterResult);
  }

  function pullAlgorithm() {
    let nextResult;
    try {
      nextResult = FunctionPrototypeCall(nextMethod, iterator);
    } catch (error) {
      return PromiseReject(error);
    }
    if (nextResult !== null &&
        (typeof nextResult === 'object' || typeof nextResult === 'function')) {
      // Mirrors `await iterator.next()`: processIterResult runs at the
      // microtask position the await resumed.
      PromisePrototypeThen(
        PromiseResolve(nextResult), processIterResult, rejectPull);
      return kParkedAlgorithmResult;
    }
    // A non-thenable next() result fails validation a microtask later.
    pendingIterResult = nextResult;
    PromisePrototypeThen(kResolvedPromise, processPendingIterResult);
    return kParkedAlgorithmResult;
  }

  async function cancelAlgorithm(reason) {
    const returnMethod = iterator.return;
    if (returnMethod === undefined) {
      return;
    }
    const iterResult = await FunctionPrototypeCall(returnMethod, iterator, reason);
    if (typeof iterResult !== 'object' || iterResult === null) {
      throw new ERR_INVALID_STATE.TypeError(
        'The promise returned by the iterator.return() method must fulfill with an object');
    }
  }

  stream = createReadableStream(
    startAlgorithm,
    pullAlgorithm,
    cancelAlgorithm,
    0,
  );

  return stream;
}

// Duck-types the lazily materialized [[closedPromise]] record of a reader
// or writer that only pipeTo or tee holds a reference to. Settling the
// record enqueues the watcher at the microtask position a reaction on the
// promise would have had, without materializing the promise; the shared
// pending promise satisfies the probes on the erroring/release paths.
class ClosedPromiseHook {
  constructor(onResolved, onRejected) {
    this.promise = kPendingPromise;
    this.onResolved = onResolved;
    this.onRejected = onRejected;
  }

  resolve() {
    if (this.onResolved !== undefined)
      PromisePrototypeThen(kResolvedPromise, this.onResolved);
  }

  reject(error) {
    const onRejected = this.onRejected;
    PromisePrototypeThen(kResolvedPromise, () => onRejected(error));
  }
}

// Installs an error watcher as the [[closedPromise]] record of a reader
// that only its caller holds. A reader of an already errored stream would
// have observed a rejected promise, so its watcher is enqueued right away.
function watchReaderErrored(reader, onRejected) {
  const stream = reader[kState].stream;
  if (stream[kState].state === 'errored') {
    const error = stream[kState].storedError;
    PromisePrototypeThen(kResolvedPromise, () => onRejected(error));
    return;
  }
  reader[kState].close = new ClosedPromiseHook(undefined, onRejected);
}

function readableStreamPipeTo(
  source,
  dest,
  preventClose,
  preventAbort,
  preventCancel,
  signal) {

  let reader;
  let writer;
  let disposable;
  // Both of these can throw synchronously. We want to capture
  // the error and return a rejected promise instead.
  try {
    reader = new ReadableStreamDefaultReader(source);
    writer = new WritableStreamDefaultWriter(dest);
  } catch (error) {
    return PromiseReject(error);
  }

  source[kState].disturbed = true;

  let shuttingDown = false;

  if (signal !== undefined) {
    try {
      validateAbortSignal(signal, 'options.signal');
    } catch (error) {
      return PromiseReject(error);
    }
  }

  const promise = PromiseWithResolvers();

  // One shared write request tracks every chunk written to the
  // destination, instead of a { promise, resolve, reject } record per
  // write. `stall` is armed by waitForPendingWrites() during shutdown;
  // `failed`/`failure` latch a write that could not proceed.
  const writeTracker = {
    // Non-undefined: queue entries are discriminated from kNilRequest
    // by `promise === undefined`.
    promise: null,
    pending: 0,
    failed: false,
    failure: undefined,
    stall: null,
    resolve() {
      if (--this.pending === 0 && this.stall !== null) {
        const stall = this.stall;
        this.stall = null;
        if (this.failed)
          stall.reject(this.failure);
        else
          stall.resolve();
      }
    },
    reject(error) {
      this.pending--;
      this.failed = true;
      this.failure = error;
      if (this.stall !== null) {
        const stall = this.stall;
        this.stall = null;
        stall.reject(error);
      }
    },
  };

  // The error here can be undefined. The rejected arg
  // tells us that the promise must be rejected even
  // when error is undefine.
  function finalize(rejected, error) {
    // The pipe is the only observer of the reader's and writer's promise
    // records (including the ready hook installed by parkOnReady), and
    // it is done with them: dropping them lets release skip the
    // pending-promise probes and the rejections nothing would handle.
    writer[kState].ready = undefined;
    writer[kState].close = undefined;
    reader[kState].close = undefined;
    writableStreamDefaultWriterRelease(writer);
    readableStreamReaderGenericRelease(reader);
    if (signal !== undefined)
      disposable?.[SymbolDispose]();
    if (rejected)
      promise.reject(error);
    else
      promise.resolve();
  }

  function waitForPendingWrites() {
    if (writeTracker.pending === 0) {
      return writeTracker.failed ?
        PromiseReject(writeTracker.failure) :
        PromiseResolve();
    }
    writeTracker.stall = PromiseWithResolvers();
    return writeTracker.stall.promise;
  }

  function shutdownWithAnAction(action, rejected, originalError) {
    if (shuttingDown) return;
    shuttingDown = true;
    if (dest[kState].state === 'writable' &&
        !writableStreamCloseQueuedOrInFlight(dest)) {
      PromisePrototypeThen(
        waitForPendingWrites(),
        complete,
        (error) => finalize(true, error));
      return;
    }
    complete();

    function complete() {
      PromisePrototypeThen(
        action(),
        () => finalize(rejected, originalError),
        (error) => finalize(true, error));
    }
  }

  function shutdown(rejected, error) {
    if (shuttingDown) return;
    shuttingDown = true;
    if (dest[kState].state === 'writable' &&
        !writableStreamCloseQueuedOrInFlight(dest)) {
      PromisePrototypeThen(
        waitForPendingWrites(),
        () => finalize(rejected, error),
        (error) => finalize(true, error));
      return;
    }
    finalize(rejected, error);
  }

  function abortAlgorithm() {
    let error;
    if (signal.reason instanceof AbortError) {
      // Cannot use the AbortError class here. It must be a DOMException.
      error = new DOMException(signal.reason.message, 'AbortError');
    } else {
      error = signal.reason;
    }

    const actions = [];
    if (!preventAbort) {
      ArrayPrototypePush(
        actions,
        () => {
          if (dest[kState].state === 'writable')
            return writableStreamAbort(dest, error);
          return PromiseResolve();
        });
    }
    if (!preventCancel) {
      ArrayPrototypePush(
        actions,
        () => {
          if (source[kState].state === 'readable')
            return readableStreamCancel(source, error);
          return PromiseResolve();
        });
    }

    shutdownWithAnAction(
      () => SafePromiseAll(actions, (action) => action()),
      true,
      error);
  }

  // The pump loop is callback-driven to avoid per-iteration promise
  // allocations. At most one read is in flight at a time, so one read
  // request and one forwarding function are reused for every chunk;
  // the chunk travels through `pendingChunk`.
  let pendingChunk;
  let readRequest;
  let readyHook;

  // Ready promise rejection is handled by the destination-errored
  // watcher.
  function ignoreReadyRejection() {}

  // Parks the pump on the destination's backpressure by installing a
  // record that duck-types the writer's lazily-materialized
  // [[readyPromise]] record: writableStreamUpdateBackpressure resolves it
  // when backpressure clears (after publishing the new backpressure
  // state), which re-enters the pump directly instead of rotating a
  // fresh promise record plus reaction per flip. The pipe holds the only
  // reference to the writer, so the record is never observable as a real
  // ready promise; the erroring/release paths probe `promise` via
  // isPromisePending() and call `reject`, so it carries the shared
  // pending promise and a no-op reject.
  function parkOnReady() {
    readyHook ??= {
      promise: kPendingPromise,
      resolve: pump,
      reject: ignoreReadyRejection,
    };
    writer[kState].ready = readyHook;
  }

  function forwardChunk() {
    const chunk = pendingChunk;
    pendingChunk = undefined;
    writableStreamDefaultWriterWriteWithRequest(writer, chunk, writeTracker);
    pump();
  }

  function pump() {
    if (shuttingDown) return;

    if (dest[kState].backpressure) {
      parkOnReady();
      return;
    }

    const controller = source[kState].controller;

    // Fast path: batch reads when data is buffered in a default controller.
    // This avoids parking read requests and reduces promise allocation
    // overhead.
    if (source[kState].state === 'readable' &&
        isReadableStreamDefaultController(controller) &&
        controller[kState].queue.length > 0) {

      while (controller[kState].queue.length > 0) {
        if (shuttingDown) return;

        const chunk = dequeueValue(controller);

        if (controller[kState].closeRequested && !controller[kState].queue.length) {
          readableStreamDefaultControllerClearAlgorithms(controller);
          readableStreamClose(source);
        }

        // Write the chunk - we're already in a separate microtask from enqueue
        // because we awaited the writer ready promise above.
        writableStreamDefaultWriterWriteWithRequest(writer, chunk, writeTracker);

        // Check backpressure after each write
        if (dest[kState].backpressure) {
          // Backpressure - stop batch and wait for ready
          break;
        } else if (dest[kState].state !== 'writable' || writableStreamCloseQueuedOrInFlight(dest)) {
          // Closing or erroring - stop batch and wait for shutdown
          break;
        }
      }

      // Trigger pull if needed after batch
      readableStreamDefaultControllerCallPullIfNeeded(controller);

      // Check if stream closed during batch
      if (source[kState].state === 'closed') {
        return;
      }

      // Park on backpressure directly: the ready hook resumes the pump
      // when a completed write clears it.
      if (dest[kState].backpressure) {
        parkOnReady();
        return;
      }

      // Yield to the microtask queue between batches so completed-write
      // reactions and events/signals fire; a shared resolved promise
      // enqueues the continuation at the same position as queueMicrotask
      // without the per-batch scheduling overhead.
      PromisePrototypeThen(kResolvedPromise, pump);
      return;
    }

    // Slow path: park a lazily materialized read request. Close and
    // error are handled by the source watchers.
    readRequest ??= new StepsReadRequest(
      (chunk) => {
        // Per spec, pipeTo must queue a microtask for the write to avoid
        // synchronous write during enqueue(). See WHATWG Streams spec
        // "ReadableStreamPipeTo" step 15's "chunk steps".
        pendingChunk = chunk;
        PromisePrototypeThen(kResolvedPromise, forwardChunk);
      },
      nonOpCallback,
      nonOpCallback);
    readableStreamDefaultReaderRead(reader, readRequest);
  }

  if (signal !== undefined) {
    if (signal.aborted) {
      abortAlgorithm();
      return promise.promise;
    }
    addAbortListener ??= require('internal/events/abort_listener').addAbortListener;
    disposable = addAbortListener(signal, abortAlgorithm);
  }

  pump();

  function onSourceErrored(error) {
    if (!preventAbort) {
      return shutdownWithAnAction(
        () => writableStreamAbort(dest, error),
        true,
        error);
    }
    shutdown(true, error);
  }

  function onSourceClosed() {
    if (!preventClose) {
      return shutdownWithAnAction(
        () => writableStreamDefaultWriterCloseWithErrorPropagation(writer));
    }
    shutdown();
  }

  function onDestErrored(error) {
    if (!preventCancel) {
      return shutdownWithAnAction(
        () => readableStreamCancel(source, error),
        true,
        error);
    }
    shutdown(true, error);
  }

  // The spec installs the source-errored watcher before the dest-errored
  // one and the source-closed watcher last; a source that is already
  // errored is handled before the dest watcher is installed, and an
  // already-closed source after it, as before. The pipe holds the only
  // references to its reader and writer, so instead of reacting to their
  // [[closedPromise]] the watchers are installed as the records
  // themselves (see ClosedPromiseHook).
  if (source[kState].state === 'errored') {
    onSourceErrored(source[kState].storedError);
  } else if (source[kState].state !== 'closed') {
    reader[kState].close =
      new ClosedPromiseHook(onSourceClosed, onSourceErrored);
  }

  if (dest[kState].state === 'errored') {
    onDestErrored(dest[kState].storedError);
  } else {
    writer[kState].close = new ClosedPromiseHook(undefined, onDestErrored);
  }

  if (source[kState].state === 'closed')
    onSourceClosed();

  if (writableStreamCloseQueuedOrInFlight(dest) ||
      dest[kState].state === 'closed') {
    const error = new ERR_INVALID_STATE.TypeError(
      'Destination WritableStream is closed');
    if (!preventCancel) {
      shutdownWithAnAction(
        () => readableStreamCancel(source, error), true, error);
    } else {
      shutdown(true, error);
    }
  }

  return promise.promise;
}

function readableStreamTee(stream, cloneForBranch2) {
  if (isReadableByteStreamController(stream[kState].controller)) {
    return readableByteStreamTee(stream);
  }
  return readableStreamDefaultTee(stream, cloneForBranch2);
}

function readableStreamDefaultTee(stream, cloneForBranch2) {
  const reader = new ReadableStreamDefaultReader(stream);
  let reading = false;
  let canceled1 = false;
  let canceled2 = false;
  let reason1;
  let reason2;
  let branch1;
  let branch2;

  // The spec's cancelPromise is materialized by the first branch cancel;
  // until then its settlement is tracked by `cancelSettled`, so a tee
  // whose branches are never canceled allocates no promise record for it.
  let cancelPromise;
  let cancelSettled = false;

  function settleCancelPromise() {
    if (cancelPromise !== undefined)
      cancelPromise.resolve();
    else
      cancelSettled = true;
  }

  function cancelPromiseRecord() {
    if (cancelPromise === undefined) {
      cancelPromise = PromiseWithResolvers();
      if (cancelSettled)
        cancelPromise.resolve();
    }
    return cancelPromise;
  }

  // At most one read is ever in flight (`reading` guards pullAlgorithm),
  // so one read request object and one forwarding microtask function are
  // reused for every chunk; the chunk travels through `pendingChunk`.
  // The request is materialized lazily on the first pull so that tee()
  // itself stays allocation-light.
  let pendingChunk;
  let readRequest;
  function forwardChunk() {
    reading = false;
    const value1 = pendingChunk;
    let value2 = pendingChunk;
    pendingChunk = undefined;
    if (!canceled2 && cloneForBranch2) {
      value2 = structuredClone(value2);
    }
    if (!canceled1) {
      readableStreamDefaultControllerEnqueue(
        branch1[kState].controller,
        value1);
    }
    if (!canceled2) {
      readableStreamDefaultControllerEnqueue(
        branch2[kState].controller,
        value2);
    }
  }

  // A non-thenable pull result reaches the controller's pull-fulfilled
  // step at the same microtask position as the async wrapper's promise
  // did, without the implicit promise per pull.
  function pullAlgorithm() {
    if (reading) return;
    reading = true;
    readRequest ??= new StepsReadRequest(
      (value) => {
        // The microtask is required by the spec (ReadableStreamTee's
        // "chunk steps" queue one).
        pendingChunk = value;
        PromisePrototypeThen(kResolvedPromise, forwardChunk);
      },
      () => {
        // The `process.nextTick()` is not part of the spec.
        // This approach was needed to avoid a race condition working with esm
        // Further information, see: https://github.com/nodejs/node/issues/39758
        process.nextTick(() => {
          reading = false;
          if (!canceled1)
            readableStreamDefaultControllerClose(branch1[kState].controller);
          if (!canceled2)
            readableStreamDefaultControllerClose(branch2[kState].controller);
          if (!canceled1 || !canceled2)
            settleCancelPromise();
        });
      },
      () => {
        reading = false;
      });
    readableStreamDefaultReaderRead(reader, readRequest);
  }

  function cancel1Algorithm(reason) {
    canceled1 = true;
    reason1 = reason;
    const record = cancelPromiseRecord();
    if (canceled2) {
      const compositeReason = [reason1, reason2];
      record.resolve(readableStreamCancel(stream, compositeReason));
    }
    return record.promise;
  }

  function cancel2Algorithm(reason) {
    canceled2 = true;
    reason2 = reason;
    const record = cancelPromiseRecord();
    if (canceled1) {
      const compositeReason = [reason1, reason2];
      record.resolve(readableStreamCancel(stream, compositeReason));
    }
    return record.promise;
  }

  branch1 =
    createReadableStream(nonOpCallback, pullAlgorithm, cancel1Algorithm);
  branch2 =
    createReadableStream(nonOpCallback, pullAlgorithm, cancel2Algorithm);

  // The tee holds the only reference to the reader, so the error watcher
  // is installed as its [[closedPromise]] record (see ClosedPromiseHook).
  watchReaderErrored(reader, (error) => {
    readableStreamDefaultControllerError(branch1[kState].controller, error);
    readableStreamDefaultControllerError(branch2[kState].controller, error);
    if (!canceled1 || !canceled2)
      settleCancelPromise();
  });

  return [branch1, branch2];
}

function readableByteStreamTee(stream) {
  assert(isReadableStream(stream));
  assert(isReadableByteStreamController(stream[kState].controller));

  let reader = new ReadableStreamDefaultReader(stream);
  let reading = false;
  let readAgainForBranch1 = false;
  let readAgainForBranch2 = false;
  let canceled1 = false;
  let canceled2 = false;
  let reason1;
  let reason2;
  let branch1;
  let branch2;

  // See readableStreamDefaultTee.
  let cancelDeferred;
  let cancelSettled = false;

  function settleCancelDeferred() {
    if (cancelDeferred !== undefined)
      cancelDeferred.resolve();
    else
      cancelSettled = true;
  }

  function cancelDeferredRecord() {
    if (cancelDeferred === undefined) {
      cancelDeferred = PromiseWithResolvers();
      if (cancelSettled)
        cancelDeferred.resolve();
    }
    return cancelDeferred;
  }

  // The tee holds the only reference to each reader it creates, so the
  // error watcher is installed as the reader's [[closedPromise]] record
  // (see ClosedPromiseHook); releasing a reader rejects it like the
  // promise, and the guard below ignores a reader that was swapped out.
  function forwardReaderError(thisReader) {
    watchReaderErrored(thisReader, (error) => {
      if (thisReader !== reader) {
        return;
      }
      readableStreamDefaultControllerError(branch1[kState].controller, error);
      readableStreamDefaultControllerError(branch2[kState].controller, error);
      if (!canceled1 || !canceled2) {
        settleCancelDeferred();
      }
    });
  }

  // As in readableStreamDefaultTee, only one read is ever in flight, so
  // the default-reader read request and its forwarding microtask are
  // shared across all chunks.
  let pendingChunk;
  function forwardChunk() {
    readAgainForBranch1 = false;
    readAgainForBranch2 = false;
    const chunk1 = pendingChunk;
    let chunk2 = pendingChunk;
    pendingChunk = undefined;

    if (!canceled1 && !canceled2) {
      try {
        chunk2 = cloneAsUint8Array(chunk1);
      } catch (error) {
        readableByteStreamControllerError(
          branch1[kState].controller,
          error,
        );
        readableByteStreamControllerError(
          branch2[kState].controller,
          error,
        );
        cancelDeferredRecord().resolve(readableStreamCancel(stream, error));
        return;
      }
    }
    if (!canceled1) {
      readableByteStreamControllerEnqueue(
        branch1[kState].controller,
        chunk1,
      );
    }
    if (!canceled2) {
      readableByteStreamControllerEnqueue(
        branch2[kState].controller,
        chunk2,
      );
    }
    reading = false;

    if (readAgainForBranch1) {
      pull1Algorithm();
    } else if (readAgainForBranch2) {
      pull2Algorithm();
    }
  }

  let defaultReadRequest;
  function pullWithDefaultReader() {
    if (isReadableStreamBYOBReader(reader)) {
      readableStreamBYOBReaderRelease(reader);
      reader = new ReadableStreamDefaultReader(stream);
      forwardReaderError(reader);
    }

    defaultReadRequest ??= new StepsReadRequest(
      (chunk) => {
        pendingChunk = chunk;
        PromisePrototypeThen(kResolvedPromise, forwardChunk);
      },
      () => {
        reading = false;

        if (!canceled1) {
          readableByteStreamControllerClose(branch1[kState].controller);
        }
        if (!canceled2) {
          readableByteStreamControllerClose(branch2[kState].controller);
        }
        if (branch1[kState].controller[kState].pendingPullIntos.length > 0) {
          readableByteStreamControllerRespond(branch1[kState].controller, 0);
        }
        if (branch2[kState].controller[kState].pendingPullIntos.length > 0) {
          readableByteStreamControllerRespond(branch2[kState].controller, 0);
        }
        if (!canceled1 || !canceled2) {
          settleCancelDeferred();
        }
      },
      () => {
        reading = false;
      });
    readableStreamDefaultReaderRead(reader, defaultReadRequest);
  }

  function pullWithBYOBReader(view, forBranch2) {
    if (isReadableStreamDefaultReader(reader)) {
      readableStreamDefaultReaderRelease(reader);
      reader = new ReadableStreamBYOBReader(stream);
      forwardReaderError(reader);
    }

    const byobBranch = forBranch2 === true ? branch2 : branch1;
    const otherBranch = forBranch2 === false ? branch2 : branch1;
    const readIntoRequest = new StepsReadRequest(
      (chunk) => {
        queueMicrotask(() => {
          readAgainForBranch1 = false;
          readAgainForBranch2 = false;
          const byobCanceled = forBranch2 === true ? canceled2 : canceled1;
          const otherCanceled = forBranch2 === false ? canceled2 : canceled1;

          if (!otherCanceled) {
            let clonedChunk;

            try {
              clonedChunk = cloneAsUint8Array(chunk);
            } catch (error) {
              readableByteStreamControllerError(
                byobBranch[kState].controller,
                error,
              );
              readableByteStreamControllerError(
                otherBranch[kState].controller,
                error,
              );
              cancelDeferredRecord().resolve(readableStreamCancel(stream, error));
              return;
            }
            if (!byobCanceled) {
              readableByteStreamControllerRespondWithNewView(
                byobBranch[kState].controller,
                chunk,
              );
            }

            readableByteStreamControllerEnqueue(
              otherBranch[kState].controller,
              clonedChunk,
            );
          } else if (!byobCanceled) {
            readableByteStreamControllerRespondWithNewView(
              byobBranch[kState].controller,
              chunk,
            );
          }
          reading = false;

          if (readAgainForBranch1) {
            pull1Algorithm();
          } else if (readAgainForBranch2) {
            pull2Algorithm();
          }
        });
      },
      (chunk) => {
        reading = false;

        const byobCanceled = forBranch2 === true ? canceled2 : canceled1;
        const otherCanceled = forBranch2 === false ? canceled2 : canceled1;

        if (!byobCanceled) {
          readableByteStreamControllerClose(byobBranch[kState].controller);
        }
        if (!otherCanceled) {
          readableByteStreamControllerClose(otherBranch[kState].controller);
        }
        if (chunk !== undefined) {
          if (!byobCanceled) {
            readableByteStreamControllerRespondWithNewView(
              byobBranch[kState].controller,
              chunk,
            );
          }
          if (
            !otherCanceled &&
            otherBranch[kState].controller[kState].pendingPullIntos.length > 0
          ) {
            readableByteStreamControllerRespond(
              otherBranch[kState].controller,
              0,
            );
          }
        }
        if (!byobCanceled || !otherCanceled) {
          settleCancelDeferred();
        }
      },
      () => {
        reading = false;
      });
    readableStreamBYOBReaderRead(reader, view, 1, readIntoRequest);
  }

  function pull1Algorithm() {
    if (reading) {
      readAgainForBranch1 = true;
      return;
    }
    reading = true;

    const byobRequest = branch1[kState].controller.byobRequest;
    if (byobRequest === null) {
      pullWithDefaultReader();
    } else {
      pullWithBYOBReader(byobRequest[kState].view, false);
    }
  }

  function pull2Algorithm() {
    if (reading) {
      readAgainForBranch2 = true;
      return;
    }
    reading = true;

    const byobRequest = branch2[kState].controller.byobRequest;
    if (byobRequest === null) {
      pullWithDefaultReader();
    } else {
      pullWithBYOBReader(byobRequest[kState].view, true);
    }
  }

  function cancel1Algorithm(reason) {
    canceled1 = true;
    reason1 = reason;
    const record = cancelDeferredRecord();
    if (canceled2) {
      record.resolve(readableStreamCancel(stream, [reason1, reason2]));
    }
    return record.promise;
  }

  function cancel2Algorithm(reason) {
    canceled2 = true;
    reason2 = reason;
    const record = cancelDeferredRecord();
    if (canceled1) {
      record.resolve(readableStreamCancel(stream, [reason1, reason2]));
    }
    return record.promise;
  }

  branch1 =
    createReadableByteStream(nonOpCallback, pull1Algorithm, cancel1Algorithm);
  branch2 =
    createReadableByteStream(nonOpCallback, pull2Algorithm, cancel2Algorithm);

  forwardReaderError(reader);

  return [branch1, branch2];
}

function readableByteStreamControllerConvertPullIntoDescriptor(desc) {
  const {
    buffer,
    bytesFilled,
    byteLength,
    byteOffset,
    ctor,
    elementSize,
  } = desc;
  if (bytesFilled > byteLength)
    throw new ERR_INVALID_STATE.RangeError('The buffer size is invalid');
  assert(!(bytesFilled % elementSize));
  const transferredBuffer = ArrayBufferPrototypeTransfer(buffer);

  if (ctor === Buffer) {
    return Buffer.from(transferredBuffer, byteOffset, bytesFilled / elementSize);
  }

  return new ctor(transferredBuffer, byteOffset, bytesFilled / elementSize);
}

function isReadableStreamLocked(stream) {
  return stream[kState].reader !== undefined;
}

function readableStreamCancel(stream, reason) {
  stream[kState].disturbed = true;
  switch (stream[kState].state) {
    case 'closed':
      return PromiseResolve();
    case 'errored':
      return PromiseReject(stream[kState].storedError);
  }
  readableStreamClose(stream);
  const {
    reader,
  } = stream[kState];
  if (reader !== undefined && readableStreamHasBYOBReader(stream)) {
    const readIntoRequests = reader[kState].readIntoRequests;
    while (readIntoRequests.length)
      readIntoRequests.shift()[kClose]();
  }

  return PromisePrototypeThen(
    stream[kState].controller[kCancel](reason),
    () => {});
}

function readableStreamClose(stream) {
  assert(stream[kState].state === 'readable');
  stream[kState].state = 'closed';
  stream[kState].closedPromise?.resolve();
  const {
    reader,
  } = stream[kState];

  if (reader === undefined)
    return;

  reader[kState].close?.resolve();

  if (readableStreamHasDefaultReader(stream)) {
    const readRequests = reader[kState].readRequests;
    while (readRequests.length)
      readRequests.shift()[kClose]();
  }
}

function readableStreamError(stream, error) {
  assert(stream[kState].state === 'readable');
  stream[kState].state = 'errored';
  stream[kState].storedError = error;
  const closedPromiseCache = stream[kState].closedPromise;
  if (closedPromiseCache !== undefined) {
    setPromiseHandled(closedPromiseCache.promise);
    closedPromiseCache.reject(error);
  }

  const {
    reader,
  } = stream[kState];

  if (reader === undefined)
    return;

  const closeCache = reader[kState].close;
  if (closeCache !== undefined) {
    setPromiseHandled(closeCache.promise);
    closeCache.reject(error);
  }

  if (readableStreamHasDefaultReader(stream)) {
    const readRequests = reader[kState].readRequests;
    while (readRequests.length)
      readRequests.shift()[kError](error);
  } else {
    assert(readableStreamHasBYOBReader(stream));
    const readIntoRequests = reader[kState].readIntoRequests;
    while (readIntoRequests.length)
      readIntoRequests.shift()[kError](error);
  }
}

function readableStreamHasDefaultReader(stream) {
  const {
    reader,
  } = stream[kState];

  if (reader === undefined)
    return false;

  return reader[kState] !== undefined &&
         reader[kType] === 'ReadableStreamDefaultReader';
}

function readableStreamGetNumReadRequests(stream) {
  assert(readableStreamHasDefaultReader(stream));
  return stream[kState].reader[kState].readRequests.length;
}

function readableStreamHasBYOBReader(stream) {
  const {
    reader,
  } = stream[kState];

  if (reader === undefined)
    return false;

  return reader[kState] !== undefined &&
         reader[kType] === 'ReadableStreamBYOBReader';
}

function readableStreamGetNumReadIntoRequests(stream) {
  assert(readableStreamHasBYOBReader(stream));
  return stream[kState].reader[kState].readIntoRequests.length;
}

function readableStreamFulfillReadRequest(stream, chunk, done) {
  assert(readableStreamHasDefaultReader(stream));
  const {
    reader,
  } = stream[kState];
  assert(reader[kState].readRequests.length);
  const readRequest = reader[kState].readRequests.shift();

  // TODO(@jasnell): It's not clear under what exact conditions done
  // will be true here. The spec requires this check but none of the
  // WPT's or other tests trigger it. Will need to investigate how to
  // get coverage for this.
  if (done)
    readRequest[kClose]();
  else
    readRequest[kChunk](chunk);
}

function readableStreamFulfillReadIntoRequest(stream, chunk, done) {
  assert(readableStreamHasBYOBReader(stream));
  const {
    reader,
  } = stream[kState];
  assert(reader[kState].readIntoRequests.length);
  const readIntoRequest = reader[kState].readIntoRequests.shift();
  if (done)
    readIntoRequest[kClose](chunk);
  else
    readIntoRequest[kChunk](chunk);
}

function readableStreamAddReadRequest(stream, readRequest) {
  assert(readableStreamHasDefaultReader(stream));
  assert(stream[kState].state === 'readable');
  const readerState = stream[kState].reader[kState];
  let readRequests = readerState.readRequests;
  if (readRequests === kEmptyQueue)
    readRequests = readerState.readRequests = new Queue();
  readRequests.push(readRequest);
}

function readableStreamAddReadIntoRequest(stream, readIntoRequest) {
  assert(readableStreamHasBYOBReader(stream));
  assert(stream[kState].state !== 'errored');
  const readerState = stream[kState].reader[kState];
  let readIntoRequests = readerState.readIntoRequests;
  if (readIntoRequests === kEmptyQueue)
    readIntoRequests = readerState.readIntoRequests = new Queue();
  readIntoRequests.push(readIntoRequest);
}

function readableStreamReaderGenericCancel(reader, reason) {
  const {
    stream,
  } = reader[kState];
  assert(stream !== undefined);
  return readableStreamCancel(stream, reason);
}

function readableStreamReaderGenericInitialize(reader, stream) {
  reader[kState].stream = stream;
  stream[kState].reader = reader;
  // The reader's closed promise (reader[kState].close) is materialized
  // lazily by readerClosedPromise(); its settlement is fully derivable
  // from the reader/stream state, so nothing is allocated here.
}

// Materializes the reader's [[closedPromise]] record on first
// observation. While unobserved, its settlement is derivable: a
// detached reader was released (rejected with the shared released
// error); otherwise it follows the stream state. The settle sites
// (readableStreamClose/Error, readableStreamReaderGenericRelease)
// only touch the record when it has been materialized.
function readerClosedPromise(reader) {
  let close = reader[kState].close;
  if (close === undefined) {
    const stream = reader[kState].stream;
    if (stream === undefined) {
      close = {
        promise: PromiseReject(lazyReadableReleasedError()),
        resolve: undefined,
        reject: undefined,
      };
      setPromiseHandled(close.promise);
    } else {
      switch (stream[kState].state) {
        case 'readable':
          close = PromiseWithResolvers();
          break;
        case 'closed':
          close = {
            promise: PromiseResolve(),
            resolve: undefined,
            reject: undefined,
          };
          break;
        case 'errored':
          close = {
            promise: PromiseReject(stream[kState].storedError),
            resolve: undefined,
            reject: undefined,
          };
          setPromiseHandled(close.promise);
          break;
      }
    }
    reader[kState].close = close;
  }
  return close;
}

function readableStreamDefaultReaderRelease(reader) {
  readableStreamReaderGenericRelease(reader);
  readableStreamDefaultReaderErrorReadRequests(
    reader,
    lazyReadableReleasingError(),
  );
}

function readableStreamDefaultReaderErrorReadRequests(reader, e) {
  const readRequests = reader[kState].readRequests;
  while (readRequests.length)
    readRequests.shift()[kError](e);
}

function readableStreamBYOBReaderRelease(reader) {
  readableStreamReaderGenericRelease(reader);
  readableStreamBYOBReaderErrorReadIntoRequests(
    reader,
    lazyReadableReleasingError(),
  );
}

function readableStreamBYOBReaderErrorReadIntoRequests(reader, e) {
  const readIntoRequests = reader[kState].readIntoRequests;
  while (readIntoRequests.length)
    readIntoRequests.shift()[kError](e);
}

function readableStreamReaderGenericRelease(reader) {
  const {
    stream,
  } = reader[kState];
  assert(stream !== undefined);
  assert(stream[kState].reader === reader);

  const closeCache = reader[kState].close;
  if (stream[kState].state === 'readable') {
    if (closeCache !== undefined) {
      closeCache.reject(lazyReadableReleasedError());
      setPromiseHandled(closeCache.promise);
    }
  } else {
    // The spec replaces [[closedPromise]] here. Dropping the cache makes
    // the next observation derive the released rejection.
    reader[kState].close = undefined;
  }

  stream[kState].controller[kRelease]();

  stream[kState].reader = undefined;
  reader[kState].stream = undefined;
}

// The argument validation half of ReadableStreamBYOBReader.prototype.read().
function readableStreamBYOBReaderReadView(reader, view, options) {
  if (!isReadableStreamBYOBReader(reader))
    throw new ERR_INVALID_THIS('ReadableStreamBYOBReader');
  validateBuffer(view, 'view');
  validateObject(options, 'options', kValidateObjectAllowObjectsAndNull);

  const viewByteLength = ArrayBufferViewGetByteLength(view);
  const viewBuffer = ArrayBufferViewGetBuffer(view);

  if (isSharedArrayBuffer(viewBuffer)) {
    throw new ERR_INVALID_ARG_VALUE(
      'view',
      view,
      'must not be backed by a SharedArrayBuffer',
    );
  }

  const viewBufferByteLength = ArrayBufferPrototypeGetByteLength(viewBuffer);

  if (viewByteLength === 0 || viewBufferByteLength === 0) {
    throw new ERR_INVALID_STATE.TypeError(
      'View or Viewed ArrayBuffer is zero-length or detached');
  }

  // Supposed to assert here that the view's buffer is not
  // detached, but there's no API available to use to check that.

  const min = options?.min ?? 1;
  validateNumber(min, 'options.min');
  if (!NumberIsInteger(min))
    throw new ERR_INVALID_ARG_VALUE('options.min', min, 'must be an integer');
  if (min <= 0)
    throw new ERR_INVALID_ARG_VALUE('options.min', min, 'must be greater than 0');
  if (!isDataView(view)) {
    if (min > TypedArrayPrototypeGetLength(view)) {
      throw new ERR_OUT_OF_RANGE('options.min', '<= view.length', min);
    }
  } else if (min > viewByteLength) {
    throw new ERR_OUT_OF_RANGE('options.min', '<= view.byteLength', min);
  }

  if (reader[kState].stream === undefined) {
    throw new ERR_INVALID_STATE.TypeError('The reader is not attached to a stream');
  }
  const readIntoRequest = new ReadIntoRequest();
  readableStreamBYOBReaderRead(reader, view, min, readIntoRequest);
  return readIntoRequest.promise;
}

function readableStreamBYOBReaderRead(reader, view, min, readIntoRequest) {
  const {
    stream,
  } = reader[kState];
  assert(stream !== undefined);
  stream[kState].disturbed = true;
  if (stream[kState].state === 'errored') {
    readIntoRequest[kError](stream[kState].storedError);
    return;
  }
  readableByteStreamControllerPullInto(
    stream[kState].controller,
    view,
    min,
    readIntoRequest);
}

function readableStreamDefaultReaderRead(reader, readRequest) {
  const {
    stream,
  } = reader[kState];
  assert(stream !== undefined);
  stream[kState].disturbed = true;
  switch (stream[kState].state) {
    case 'closed':
      readRequest[kClose]();
      break;
    case 'errored':
      readRequest[kError](stream[kState].storedError);
      break;
    case 'readable':
      stream[kState].controller[kPull](readRequest);
  }
}

function setupReadableStreamBYOBReader(reader, stream) {
  if (isReadableStreamLocked(stream))
    throw new ERR_INVALID_STATE.TypeError('ReadableStream is locked');
  const {
    controller,
  } = stream[kState];
  if (!isReadableByteStreamController(controller))
    throw new ERR_INVALID_ARG_VALUE('stream', stream, 'must be a byte stream');
  readableStreamReaderGenericInitialize(reader, stream);
  // The read-request queues use the same ring buffer as [[queue]], drained
  // from a moving head rather than with ArrayPrototypeShift. Start from the
  // shared immutable empty queue so acquiring a reader allocates no request
  // storage until a read actually parks.
  reader[kState].readIntoRequests = kEmptyQueue;
}

function setupReadableStreamDefaultReader(reader, stream) {
  if (isReadableStreamLocked(stream))
    throw new ERR_INVALID_STATE.TypeError('ReadableStream is locked');
  readableStreamReaderGenericInitialize(reader, stream);
  reader[kState].readRequests = kEmptyQueue;
}

function readableStreamDefaultControllerClose(controller) {
  if (!readableStreamDefaultControllerCanCloseOrEnqueue(controller))
    return;
  controller[kState].closeRequested = true;
  if (!controller[kState].queue.length) {
    readableStreamDefaultControllerClearAlgorithms(controller);
    readableStreamClose(controller[kState].stream);
  }
}

function readableStreamDefaultControllerEnqueue(controller, chunk) {
  // Equivalent to readableStreamDefaultControllerCanCloseOrEnqueue()
  // followed by isReadableStreamLocked() and
  // readableStreamGetNumReadRequests(), but with the state loaded once:
  // this runs for every enqueued chunk.
  const controllerState = controller[kState];
  const stream = controllerState.stream;
  if (controllerState.closeRequested || stream[kState].state !== 'readable')
    return;

  const reader = stream[kState].reader;
  if (reader !== undefined &&
      reader[kState] !== undefined &&
      reader[kType] === 'ReadableStreamDefaultReader' &&
      reader[kState].readRequests.length) {
    // Fulfilling a read request can run user code synchronously (the
    // pipeTo read request invokes the sink's write algorithm), so the
    // full ShouldCallPull predicate has to be re-evaluated afterwards.
    readableStreamFulfillReadRequest(stream, chunk, false);
  } else if (controllerState.sizeAlgorithm === defaultSizeAlgorithm) {
    // The internal default size algorithm is never observable by user
    // code, always returns 1, and cannot throw: enqueue with the
    // constant size instead of calling it or validating the result.
    // No user code runs between the guards at the top of this function
    // and this point, so ShouldCallPull reduces to the started flag and
    // the desired size (this branch implies no parked read requests,
    // ruling out the reader arm of the predicate).
    materializeQueue(controllerState).pushPair(chunk, 1);
    controllerState.queueTotalSize++;
    if (controllerState.started &&
        controllerState.highWaterMark - controllerState.queueTotalSize > 0) {
      readableStreamDefaultControllerPull(controller);
    }
    return;
  } else {
    // The user-supplied size algorithm may run arbitrary code, so the
    // full ShouldCallPull predicate has to be re-evaluated afterwards.
    try {
      const chunkSize =
        FunctionPrototypeCall(
          controllerState.sizeAlgorithm,
          undefined,
          chunk);
      enqueueValueWithSize(controller, chunk, chunkSize);
    } catch (error) {
      readableStreamDefaultControllerError(controller, error);
      throw error;
    }
  }
  readableStreamDefaultControllerCallPullIfNeeded(controller);
}

function readableStreamDefaultControllerHasBackpressure(controller) {
  return !readableStreamDefaultControllerShouldCallPull(controller);
}

function readableStreamDefaultControllerCanCloseOrEnqueue(controller) {
  const {
    stream,
  } = controller[kState];
  return !controller[kState].closeRequested &&
         stream[kState].state === 'readable';
}

function readableStreamDefaultControllerGetDesiredSize(controller) {
  const {
    stream,
    highWaterMark,
    queueTotalSize,
  } = controller[kState];
  switch (stream[kState].state) {
    case 'errored': return null;
    case 'closed': return 0;
    default:
      return highWaterMark - queueTotalSize;
  }
}

function readableStreamDefaultControllerShouldCallPull(controller) {
  // Single-pass version of the spec's predicate chain (CanCloseOrEnqueue,
  // IsLocked, HasDefaultReader, GetNumReadRequests, GetDesiredSize): this
  // runs at least once per chunk on every default-stream path. The
  // desired-size computation is inlined because the stream state is
  // already known to be 'readable' here.
  const controllerState = controller[kState];
  const stream = controllerState.stream;
  if (controllerState.closeRequested ||
      stream[kState].state !== 'readable' ||
      !controllerState.started)
    return false;

  const reader = stream[kState].reader;
  if (reader !== undefined &&
      reader[kState] !== undefined &&
      reader[kType] === 'ReadableStreamDefaultReader' &&
      reader[kState].readRequests.length) {
    return true;
  }

  return controllerState.highWaterMark - controllerState.queueTotalSize > 0;
}

function readableStreamDefaultControllerCallPullIfNeeded(controller) {
  if (!readableStreamDefaultControllerShouldCallPull(controller))
    return;
  readableStreamDefaultControllerPull(controller);
}

// The ShouldCallPull half of CallPullIfNeeded, split out so that callers
// that have already established the predicate from state in scope (the
// enqueue path above) can skip re-running it.
function readableStreamDefaultControllerPull(controller) {
  // A source without pull() has nothing to run: skip the pulling/pullAgain
  // bookkeeping, the reaction closures and the microtask that would only
  // clear the flag again. Push-style sources hit this on every read.
  if (controller[kState].pullAlgorithm === nonOpCallback)
    return;
  if (controller[kState].pulling) {
    controller[kState].pullAgain = true;
    return;
  }
  assert(!controller[kState].pullAgain);
  controller[kState].pulling = true;
  if (controller[kState].pullFulfilled === undefined) {
    // The pull reaction closures only capture the controller, so they are
    // created once on the first pull and reused for every subsequent pull
    // instead of allocating two fresh closures per chunk.
    controller[kState].pullFulfilled = () => {
      controller[kState].pulling = false;
      if (controller[kState].pullAgain) {
        controller[kState].pullAgain = false;
        readableStreamDefaultControllerCallPullIfNeeded(controller);
      }
    };
    controller[kState].pullRejected =
      (error) => readableStreamDefaultControllerError(controller, error);
  }
  // The pull algorithm may be a raw callback (a wrapped user source.pull
  // returns its result uncoerced; a synchronous throw surfaces here) or an
  // internal algorithm that always returns a promise; thenAlgorithmResult
  // handles both.
  let result;
  try {
    result = controller[kState].pullAlgorithm(controller);
  } catch (error) {
    result = PromiseReject(error);
  }
  thenAlgorithmResult(
    result,
    controller[kState].pullFulfilled,
    controller[kState].pullRejected);
}

function readableStreamDefaultControllerClearAlgorithms(controller) {
  controller[kState].pullAlgorithm = undefined;
  controller[kState].cancelAlgorithm = undefined;
  controller[kState].sizeAlgorithm = undefined;
}

function readableStreamDefaultControllerError(controller, error) {
  const {
    stream,
  } = controller[kState];
  if (stream[kState].state === 'readable') {
    resetQueue(controller);
    readableStreamDefaultControllerClearAlgorithms(controller);
    readableStreamError(stream, error);
  }
}

function readableStreamDefaultControllerCancelSteps(controller, reason) {
  resetQueue(controller);
  const result = controller[kState].cancelAlgorithm(reason);
  readableStreamDefaultControllerClearAlgorithms(controller);
  return result;
}

function readableStreamDefaultControllerPullSteps(controller, readRequest) {
  const {
    stream,
    queue,
  } = controller[kState];
  if (queue.length) {
    const chunk = dequeueValue(controller);
    if (controller[kState].closeRequested && !queue.length) {
      readableStreamDefaultControllerClearAlgorithms(controller);
      readableStreamClose(stream);
    } else if (!controller[kState].closeRequested &&
               controller[kState].started &&
               controller[kState].highWaterMark -
                 controller[kState].queueTotalSize > 0) {
      // Reduced ShouldCallPull: the state is known to be readable and the
      // queue was non-empty, so no read requests can be parked.
      readableStreamDefaultControllerPull(controller);
    }
    readRequest[kChunk](chunk);
    return;
  }
  readableStreamAddReadRequest(stream, readRequest);
  // Reduced ShouldCallPull: the state is known to be readable, an empty
  // queue in that state implies close has not been requested (close with
  // an empty queue closes the stream immediately), and the read request
  // parked above already satisfies the reader-with-pending-reads arm.
  if (controller[kState].started)
    readableStreamDefaultControllerPull(controller);
}

function setupReadableStreamDefaultController(
  stream,
  controller,
  startAlgorithm,
  pullAlgorithm,
  cancelAlgorithm,
  highWaterMark,
  sizeAlgorithm) {
  assert(stream[kState].controller === undefined);
  controller[kState] = {
    cancelAlgorithm,
    closeRequested: false,
    highWaterMark,
    pullAgain: false,
    pullAlgorithm,
    pulling: false,
    pullFulfilled: undefined,
    pullRejected: undefined,
    queue: kEmptyQueue,
    queueTotalSize: 0,
    started: false,
    sizeAlgorithm,
    stream,
  };
  stream[kState].controller = controller;

  const startResult = startAlgorithm();
  // A non-thenable start result guarantees fulfillment, and no .then
  // lookup on it is observable.
  const startFulfilled = startResult === null ||
    (typeof startResult !== 'object' && typeof startResult !== 'function');

  // The started flag only gates calls into the pull algorithm, so for a
  // source without pull() the post-start step has nothing observable
  // left to do: the flag is set right away instead of from a microtask.
  if (startFulfilled && pullAlgorithm === nonOpCallback) {
    controller[kState].started = true;
    return;
  }

  const started = () => {
    controller[kState].started = true;
    assert(!controller[kState].pulling);
    assert(!controller[kState].pullAgain);
    readableStreamDefaultControllerCallPullIfNeeded(controller);
  };

  if (startFulfilled) {
    // The post-start step runs at the exact microtask position the
    // promise reaction would have had.
    queueMicrotask(started);
    return;
  }

  // The wrapper promise matches the reference implementation's
  // promiseResolvedWith(), whose extra microtask hops WPT relies on.
  PromisePrototypeThen(
    new Promise((r) => r(startResult)),
    started,
    (error) => readableStreamDefaultControllerError(controller, error));
}

function setupReadableStreamDefaultControllerFromSource(
  stream,
  source,
  highWaterMark,
  sizeAlgorithm) {
  const controller = new ReadableStreamDefaultController(kSkipThrow);
  const start = source?.start;
  const pull = source?.pull;
  const cancel = source?.cancel;
  const startAlgorithm = start ?
    FunctionPrototypeBind(start, source, controller) :
    nonOpCallback;
  const pullAlgorithm = pull ?
    createRawCallback1Param('source.pull', pull, source) :
    nonOpCallback;
  const cancelAlgorithm = cancel ?
    createPromiseCallback1Param('source.cancel', cancel, source) :
    nonOpCancel;

  setupReadableStreamDefaultController(
    stream,
    controller,
    startAlgorithm,
    pullAlgorithm,
    cancelAlgorithm,
    highWaterMark,
    sizeAlgorithm);
}

function readableByteStreamControllerClose(controller) {
  const {
    closeRequested,
    pendingPullIntos,
    queueTotalSize,
    stream,
  } = controller[kState];

  if (closeRequested || stream[kState].state !== 'readable')
    return;

  if (queueTotalSize) {
    controller[kState].closeRequested = true;
    return;
  }

  if (pendingPullIntos.length) {
    const firstPendingPullInto = pendingPullIntos.peek();
    if (firstPendingPullInto.bytesFilled % firstPendingPullInto.elementSize !== 0) {
      const error = new ERR_INVALID_STATE.TypeError('Partial read');
      readableByteStreamControllerError(controller, error);
      throw error;
    }
  }

  readableByteStreamControllerClearAlgorithms(controller);
  readableStreamClose(stream);
}

function readableByteStreamControllerCommitPullIntoDescriptor(stream, desc) {
  assert(stream[kState].state !== 'errored');
  assert(desc.type !== 'none');

  let done = false;
  if (stream[kState].state === 'closed') {
    assert(desc.bytesFilled % desc.elementSize === 0);
    done = true;
  }

  const filledView =
    readableByteStreamControllerConvertPullIntoDescriptor(desc);

  if (desc.type === 'default') {
    readableStreamFulfillReadRequest(stream, filledView, done);
  } else {
    assert(desc.type === 'byob');
    readableStreamFulfillReadIntoRequest(stream, filledView, done);
  }
}

function readableByteStreamControllerCommitPullIntoDescriptors(stream, descriptors) {
  for (let i = 0; i < descriptors.length; ++i) {
    readableByteStreamControllerCommitPullIntoDescriptor(
      stream,
      descriptors[i],
    );
  }
}

function readableByteStreamControllerInvalidateBYOBRequest(controller) {
  if (controller[kState].byobRequest === null)
    return;
  controller[kState].byobRequest[kState].controller = undefined;
  controller[kState].byobRequest[kState].view = null;
  controller[kState].byobRequest = null;
}

function readableByteStreamControllerClearAlgorithms(controller) {
  controller[kState].pullAlgorithm = undefined;
  controller[kState].cancelAlgorithm = undefined;
}

function readableByteStreamControllerClearPendingPullIntos(controller) {
  readableByteStreamControllerInvalidateBYOBRequest(controller);
  controller[kState].pendingPullIntos = kEmptyQueue;
}

function readableByteStreamControllerGetDesiredSize(controller) {
  const {
    stream,
    highWaterMark,
    queueTotalSize,
  } = controller[kState];
  switch (stream[kState].state) {
    case 'errored': return null;
    case 'closed': return 0;
    default: return highWaterMark - queueTotalSize;
  }
}

function readableByteStreamControllerShouldCallPull(controller) {
  // Single-pass version of the spec's predicate chain (HasDefaultReader,
  // GetNumReadRequests, HasBYOBReader, GetNumReadIntoRequests,
  // GetDesiredSize): this runs at least once per chunk on every byte
  // stream path. The desired-size computation is inlined because the
  // stream state is already known to be 'readable' here.
  const controllerState = controller[kState];
  const stream = controllerState.stream;
  if (stream[kState].state !== 'readable' ||
      controllerState.closeRequested ||
      !controllerState.started) {
    return false;
  }
  const reader = stream[kState].reader;
  if (reader !== undefined && reader[kState] !== undefined) {
    const type = reader[kType];
    if (type === 'ReadableStreamDefaultReader') {
      if (reader[kState].readRequests.length) return true;
    } else if (type === 'ReadableStreamBYOBReader') {
      if (reader[kState].readIntoRequests.length) return true;
    }
  }

  return controllerState.highWaterMark - controllerState.queueTotalSize > 0;
}

function readableByteStreamControllerHandleQueueDrain(controller) {
  const {
    closeRequested,
    queueTotalSize,
    stream,
  } = controller[kState];
  assert(stream[kState].state === 'readable');
  if (!queueTotalSize && closeRequested) {
    readableByteStreamControllerClearAlgorithms(controller);
    readableStreamClose(stream);
    return;
  }
  readableByteStreamControllerCallPullIfNeeded(controller);
}

function readableByteStreamControllerPullInto(
  controller,
  view,
  min,
  readIntoRequest) {
  const {
    closeRequested,
    stream,
    pendingPullIntos,
  } = controller[kState];
  let elementSize = 1;
  let ctor = DataView;
  if (isArrayBufferView(view) && !isDataView(view)) {
    elementSize = view.constructor.BYTES_PER_ELEMENT;
    ctor = view.constructor;
  }

  const minimumFill = min * elementSize;
  assert(minimumFill >= elementSize && minimumFill <= view.byteLength);
  assert(minimumFill % elementSize === 0);

  const buffer = ArrayBufferViewGetBuffer(view);
  const byteOffset = ArrayBufferViewGetByteOffset(view);
  const byteLength = ArrayBufferViewGetByteLength(view);
  const bufferByteLength = ArrayBufferPrototypeGetByteLength(buffer);

  let transferredBuffer;
  try {
    transferredBuffer = ArrayBufferPrototypeTransfer(buffer);
  } catch (error) {
    readIntoRequest[kError](error);
    return;
  }
  const desc = {
    buffer: transferredBuffer,
    bufferByteLength,
    byteOffset,
    byteLength,
    bytesFilled: 0,
    minimumFill,
    elementSize,
    ctor,
    type: 'byob',
  };
  if (pendingPullIntos.length) {
    pendingPullIntos.push(desc);
    readableStreamAddReadIntoRequest(stream, readIntoRequest);
    return;
  }
  if (stream[kState].state === 'closed') {
    const emptyView = new ctor(desc.buffer, byteOffset, 0);
    readIntoRequest[kClose](emptyView);
    return;
  }
  if (controller[kState].queueTotalSize) {
    if (readableByteStreamControllerFillPullIntoDescriptorFromQueue(
      controller,
      desc)) {
      const filledView =
        readableByteStreamControllerConvertPullIntoDescriptor(desc);
      readableByteStreamControllerHandleQueueDrain(controller);
      readIntoRequest[kChunk](filledView);
      return;
    }
    if (closeRequested) {
      const error = new ERR_INVALID_STATE.TypeError('ReadableStream closed');
      readableByteStreamControllerError(controller, error);
      readIntoRequest[kError](error);
      return;
    }
  }
  materializePendingPullIntos(controller[kState]).push(desc);
  readableStreamAddReadIntoRequest(stream, readIntoRequest);
  readableByteStreamControllerCallPullIfNeeded(controller);
}

// Pending pull-into descriptor queues start out as (and are reset to)
// the shared immutable empty queue so that constructing a byte stream
// never allocates descriptor storage; the two push sites that can
// observe an empty queue materialize a real Queue on first use.
function materializePendingPullIntos(state) {
  const pendingPullIntos = state.pendingPullIntos;
  if (pendingPullIntos === kEmptyQueue)
    return state.pendingPullIntos = new Queue();
  return pendingPullIntos;
}

function readableByteStreamControllerRespondInternal(controller, bytesWritten) {
  const {
    stream,
    pendingPullIntos,
  } = controller[kState];
  const desc = pendingPullIntos.peek();
  readableByteStreamControllerInvalidateBYOBRequest(controller);
  if (stream[kState].state === 'closed') {
    if (bytesWritten)
      throw new ERR_INVALID_STATE.TypeError(
        'Controller is closed but view is not zero-length');
    readableByteStreamControllerRespondInClosedState(controller, desc);
  } else {
    assert(stream[kState].state === 'readable');
    if (!bytesWritten)
      throw new ERR_INVALID_STATE.TypeError('View cannot be zero-length');
    readableByteStreamControllerRespondInReadableState(
      controller,
      bytesWritten,
      desc);
  }
  readableByteStreamControllerCallPullIfNeeded(controller);
}

function readableByteStreamControllerRespond(controller, bytesWritten) {
  const {
    pendingPullIntos,
    stream,
  } = controller[kState];
  assert(pendingPullIntos.length);
  const desc = pendingPullIntos.peek();

  if (stream[kState].state === 'closed') {
    if (bytesWritten !== 0)
      throw new ERR_INVALID_ARG_VALUE('bytesWritten', bytesWritten);
  } else {
    assert(stream[kState].state === 'readable');

    if (!bytesWritten)
      throw new ERR_INVALID_ARG_VALUE('bytesWritten', bytesWritten);

    if ((desc.bytesFilled + bytesWritten) > desc.byteLength)
      throw new ERR_INVALID_ARG_VALUE.RangeError('bytesWritten', bytesWritten);
  }

  desc.buffer = ArrayBufferPrototypeTransfer(desc.buffer);

  readableByteStreamControllerRespondInternal(controller, bytesWritten);
}

function readableByteStreamControllerRespondInClosedState(controller, desc) {
  assert(desc.bytesFilled % desc.elementSize === 0);
  if (desc.type === 'none') {
    readableByteStreamControllerShiftPendingPullInto(controller);
  }
  const {
    stream,
  } = controller[kState];
  if (readableStreamHasBYOBReader(stream)) {
    const filledPullIntos = [];
    for (let i = 0; i < readableStreamGetNumReadIntoRequests(stream); ++i) {
      ArrayPrototypePush(filledPullIntos, readableByteStreamControllerShiftPendingPullInto(controller));
    }
    readableByteStreamControllerCommitPullIntoDescriptors(stream, filledPullIntos);
  }
}

function readableByteStreamControllerFillHeadPullIntoDescriptor(
  controller,
  size,
  desc) {
  const {
    pendingPullIntos,
    byobRequest,
  } = controller[kState];
  assert(!pendingPullIntos.length || pendingPullIntos.peek() === desc);
  assert(byobRequest === null);
  desc.bytesFilled += size;
}

function readableByteStreamControllerEnqueue(controller, chunk) {
  const {
    closeRequested,
    pendingPullIntos,
    queue,
    stream,
  } = controller[kState];

  const buffer = ArrayBufferViewGetBuffer(chunk);
  const byteOffset = ArrayBufferViewGetByteOffset(chunk);
  const byteLength = ArrayBufferViewGetByteLength(chunk);

  if (closeRequested || stream[kState].state !== 'readable')
    return;

  const transferredBuffer = ArrayBufferPrototypeTransfer(buffer);

  if (pendingPullIntos.length) {
    const firstPendingPullInto = pendingPullIntos.peek();

    if (ArrayBufferPrototypeGetDetached(firstPendingPullInto.buffer)) {
      throw new ERR_INVALID_STATE.TypeError(
        'Destination ArrayBuffer is detached',
      );
    }

    readableByteStreamControllerInvalidateBYOBRequest(controller);

    firstPendingPullInto.buffer = ArrayBufferPrototypeTransfer(
      firstPendingPullInto.buffer,
    );

    if (firstPendingPullInto.type === 'none') {
      readableByteStreamControllerEnqueueDetachedPullIntoToQueue(
        controller,
        firstPendingPullInto,
      );
    }
  }

  // Single consolidated pass over the reader state. The spec routes this
  // through HasDefaultReader / ProcessReadRequestsUsingQueue /
  // GetNumReadRequests / FulfillReadRequest, which would re-run the same
  // reader brand check and re-load the read request list four times on
  // this per-chunk path.
  const { reader } = stream[kState];
  if (reader !== undefined &&
      reader[kState] !== undefined &&
      reader[kType] === 'ReadableStreamDefaultReader') {
    const { readRequests } = reader[kState];
    if (readRequests.length && controller[kState].queueTotalSize > 0) {
      // Only possible when data was enqueued while the stream was not
      // being read; read requests otherwise never coexist with a
      // non-empty queue.
      readableByteStreamControllerProcessReadRequestsUsingQueue(controller);
    }
    if (!readRequests.length) {
      readableByteStreamControllerEnqueueChunkToQueue(
        controller,
        transferredBuffer,
        byteOffset,
        byteLength);
    } else {
      assert(!queue.length);
      if (pendingPullIntos.length) {
        assert(pendingPullIntos.peek().type === 'default');
        readableByteStreamControllerShiftPendingPullInto(controller);
      }
      const transferredView =
        new Uint8Array(transferredBuffer, byteOffset, byteLength);
      const readRequest = readRequests.shift();
      readRequest[kChunk](transferredView);
    }
  } else if (readableStreamHasBYOBReader(stream)) {
    readableByteStreamControllerEnqueueChunkToQueue(
      controller,
      transferredBuffer,
      byteOffset,
      byteLength);
    const filledPullIntos = readableByteStreamControllerProcessPullIntoDescriptorsUsingQueue(
      controller);
    readableByteStreamControllerCommitPullIntoDescriptors(stream, filledPullIntos);
  } else {
    assert(!isReadableStreamLocked(stream));
    readableByteStreamControllerEnqueueChunkToQueue(
      controller,
      transferredBuffer,
      byteOffset,
      byteLength);
  }
  readableByteStreamControllerCallPullIfNeeded(controller);
}

function readableByteStreamControllerEnqueueClonedChunkToQueue(
  controller,
  buffer,
  byteOffset,
  byteLength,
) {
  let cloneResult;
  try {
    cloneResult = ArrayBufferPrototypeSlice(
      buffer,
      byteOffset,
      byteOffset + byteLength,
    );
  } catch (error) {
    readableByteStreamControllerError(controller, error);
    throw error;
  }
  readableByteStreamControllerEnqueueChunkToQueue(
    controller,
    cloneResult,
    0,
    byteLength,
  );
}

function readableByteStreamControllerEnqueueChunkToQueue(
  controller,
  buffer,
  byteOffset,
  byteLength) {
  const state = controller[kState];
  materializeQueue(state).push({
    buffer,
    byteOffset,
    byteLength,
  });
  state.queueTotalSize += byteLength;
}

function readableByteStreamControllerEnqueueDetachedPullIntoToQueue(
  controller,
  desc,
) {
  const {
    buffer,
    byteOffset,
    bytesFilled,
    type,
  } = desc;
  assert(type === 'none');

  if (bytesFilled > 0) {
    readableByteStreamControllerEnqueueClonedChunkToQueue(
      controller,
      buffer,
      byteOffset,
      bytesFilled,
    );
  }
  readableByteStreamControllerShiftPendingPullInto(controller);
}

function readableByteStreamControllerFillPullIntoDescriptorFromQueue(
  controller,
  desc) {
  const {
    buffer,
    byteLength,
    byteOffset,
    bytesFilled,
    minimumFill,
    elementSize,
  } = desc;
  const maxBytesToCopy = MathMin(
    controller[kState].queueTotalSize,
    byteLength - bytesFilled);
  const maxBytesFilled = bytesFilled + maxBytesToCopy;
  const maxAlignedBytes = maxBytesFilled - (maxBytesFilled % elementSize);
  let totalBytesToCopyRemaining = maxBytesToCopy;
  let ready = false;
  assert(!ArrayBufferPrototypeGetDetached(buffer));
  assert(bytesFilled < minimumFill);
  if (maxAlignedBytes >= minimumFill) {
    totalBytesToCopyRemaining = maxAlignedBytes - bytesFilled;
    ready = true;
  }
  const {
    queue,
  } = controller[kState];

  while (totalBytesToCopyRemaining) {
    const headOfQueue = queue.peek();
    const bytesToCopy = MathMin(
      totalBytesToCopyRemaining,
      headOfQueue.byteLength);
    const destStart = byteOffset + desc.bytesFilled;
    assert(canCopyArrayBuffer(
      buffer,
      destStart,
      headOfQueue.buffer,
      headOfQueue.byteOffset,
      bytesToCopy));
    copyArrayBuffer(
      buffer,
      destStart,
      headOfQueue.buffer,
      headOfQueue.byteOffset,
      bytesToCopy);
    if (headOfQueue.byteLength === bytesToCopy) {
      queue.shift();
    } else {
      headOfQueue.byteOffset += bytesToCopy;
      headOfQueue.byteLength -= bytesToCopy;
    }
    controller[kState].queueTotalSize -= bytesToCopy;
    readableByteStreamControllerFillHeadPullIntoDescriptor(
      controller,
      bytesToCopy,
      desc);
    totalBytesToCopyRemaining -= bytesToCopy;
  }

  if (!ready) {
    assert(!controller[kState].queueTotalSize);
    assert(desc.bytesFilled > 0);
    assert(desc.bytesFilled < minimumFill);
  }
  return ready;
}

function readableByteStreamControllerProcessPullIntoDescriptorsUsingQueue(
  controller) {
  const {
    closeRequested,
    pendingPullIntos,
  } = controller[kState];
  assert(!closeRequested);
  const filledPullIntos = [];
  while (pendingPullIntos.length) {
    if (!controller[kState].queueTotalSize)
      break;
    const desc = pendingPullIntos.peek();
    if (readableByteStreamControllerFillPullIntoDescriptorFromQueue(
      controller,
      desc)) {
      readableByteStreamControllerShiftPendingPullInto(controller);
      ArrayPrototypePush(filledPullIntos, desc);
    }
  }
  return filledPullIntos;
}

function readableByteStreamControllerRespondInReadableState(
  controller,
  bytesWritten,
  desc) {
  const {
    stream,
  } = controller[kState];
  const {
    buffer,
    bytesFilled,
    byteLength,
    type,
  } = desc;

  if (bytesFilled + bytesWritten > byteLength)
    throw new ERR_INVALID_STATE.RangeError('The buffer size is invalid');

  readableByteStreamControllerFillHeadPullIntoDescriptor(
    controller,
    bytesWritten,
    desc);

  if (type === 'none') {
    readableByteStreamControllerEnqueueDetachedPullIntoToQueue(
      controller,
      desc,
    );
    const filledPullIntos = readableByteStreamControllerProcessPullIntoDescriptorsUsingQueue(
      controller,
    );
    readableByteStreamControllerCommitPullIntoDescriptors(stream, filledPullIntos);
    return;
  }

  if (desc.bytesFilled < desc.minimumFill)
    return;

  readableByteStreamControllerShiftPendingPullInto(controller);

  const remainderSize = desc.bytesFilled % desc.elementSize;

  if (remainderSize) {
    const end = desc.byteOffset + desc.bytesFilled;
    const start = end - remainderSize;
    const remainder =
      ArrayBufferPrototypeSlice(
        buffer,
        start,
        end);
    readableByteStreamControllerEnqueueChunkToQueue(
      controller,
      remainder,
      0,
      ArrayBufferPrototypeGetByteLength(remainder));
  }
  desc.bytesFilled -= remainderSize;
  const filledPullIntos = readableByteStreamControllerProcessPullIntoDescriptorsUsingQueue(controller);

  readableByteStreamControllerCommitPullIntoDescriptor(stream, desc);
  readableByteStreamControllerCommitPullIntoDescriptors(stream, filledPullIntos);
}

function readableByteStreamControllerRespondWithNewView(controller, view) {
  const {
    stream,
    pendingPullIntos,
  } = controller[kState];
  assert(pendingPullIntos.length);

  const desc = pendingPullIntos.peek();
  assert(stream[kState].state !== 'errored');

  const viewByteLength = ArrayBufferViewGetByteLength(view);
  const viewByteOffset = ArrayBufferViewGetByteOffset(view);
  const viewBuffer = ArrayBufferViewGetBuffer(view);
  const viewBufferByteLength = ArrayBufferPrototypeGetByteLength(viewBuffer);

  if (stream[kState].state === 'closed') {
    if (viewByteLength !== 0)
      throw new ERR_INVALID_STATE.TypeError('View is not zero-length');
  } else {
    assert(stream[kState].state === 'readable');
    if (viewByteLength === 0)
      throw new ERR_INVALID_STATE.TypeError('View is zero-length');
  }

  const {
    byteOffset,
    byteLength,
    bytesFilled,
    bufferByteLength,
  } = desc;

  if (byteOffset + bytesFilled !== viewByteOffset)
    throw new ERR_INVALID_ARG_VALUE.RangeError('view', view);

  if (bytesFilled + viewByteLength > byteLength)
    throw new ERR_INVALID_ARG_VALUE.RangeError('view', view);

  if (bufferByteLength !== viewBufferByteLength)
    throw new ERR_INVALID_ARG_VALUE.RangeError('view', view);

  desc.buffer = ArrayBufferPrototypeTransfer(viewBuffer);

  readableByteStreamControllerRespondInternal(controller, viewByteLength);
}

function readableByteStreamControllerShiftPendingPullInto(controller) {
  assert(controller[kState].byobRequest === null);
  return controller[kState].pendingPullIntos.shift();
}

function readableByteStreamControllerCallPullIfNeeded(controller) {
  if (!readableByteStreamControllerShouldCallPull(controller))
    return;
  // See readableStreamDefaultControllerPull.
  if (controller[kState].pullAlgorithm === nonOpCallback)
    return;
  if (controller[kState].pulling) {
    controller[kState].pullAgain = true;
    return;
  }
  assert(!controller[kState].pullAgain);
  controller[kState].pulling = true;
  if (controller[kState].pullFulfilled === undefined) {
    // See readableStreamDefaultControllerCallPullIfNeeded: created once,
    // reused for every pull.
    controller[kState].pullFulfilled = () => {
      controller[kState].pulling = false;
      if (controller[kState].pullAgain) {
        controller[kState].pullAgain = false;
        readableByteStreamControllerCallPullIfNeeded(controller);
      }
    };
    controller[kState].pullRejected =
      (error) => readableByteStreamControllerError(controller, error);
  }
  // See readableStreamDefaultControllerPull for the raw-callback contract.
  let result;
  try {
    result = controller[kState].pullAlgorithm(controller);
  } catch (error) {
    result = PromiseReject(error);
  }
  thenAlgorithmResult(
    result,
    controller[kState].pullFulfilled,
    controller[kState].pullRejected);
}

function readableByteStreamControllerError(controller, error) {
  const {
    stream,
  } = controller[kState];
  if (stream[kState].state !== 'readable')
    return;
  readableByteStreamControllerClearPendingPullIntos(controller);
  resetQueue(controller);
  readableByteStreamControllerClearAlgorithms(controller);
  readableStreamError(stream, error);
}

function readableByteStreamControllerCancelSteps(controller, reason) {
  readableByteStreamControllerClearPendingPullIntos(controller);
  resetQueue(controller);
  const result = controller[kState].cancelAlgorithm(reason);
  readableByteStreamControllerClearAlgorithms(controller);
  return result;
}

// Dequeues the first chunk of the byte queue as a Uint8Array view,
// handling queue drain (close-on-empty or pull) before the view is
// created. This is the [[queueTotalSize]] > 0 arm of the byte
// controller's pull steps; it is also called directly from the
// buffered fast paths in ReadableStreamDefaultReader.read() and the
// async iterator, which resolve with the view without allocating a
// read request.
function readableByteStreamControllerDequeueChunk(controller) {
  assert(controller[kState].queueTotalSize > 0);
  const {
    buffer,
    byteOffset,
    byteLength,
  } = controller[kState].queue.shift();

  controller[kState].queueTotalSize -= byteLength;
  readableByteStreamControllerHandleQueueDrain(controller);
  return new Uint8Array(buffer, byteOffset, byteLength);
}

function readableByteStreamControllerFillReadRequestFromQueue(controller, readRequest) {
  readRequest[kChunk](readableByteStreamControllerDequeueChunk(controller));
}

function readableByteStreamControllerProcessReadRequestsUsingQueue(controller) {
  const {
    stream,
    queueTotalSize,
  } = controller[kState];
  const { reader } = stream[kState];
  assert(isReadableStreamDefaultReader(reader));

  while (reader[kState].readRequests.length > 0) {
    if (queueTotalSize === 0) {
      return;
    }
    readableByteStreamControllerFillReadRequestFromQueue(
      controller,
      reader[kState].readRequests.shift(),
    );
  }
}

function readableByteStreamControllerPullSteps(controller, readRequest) {
  const {
    queueTotalSize,
    stream,
  } = controller[kState];
  assert(readableStreamHasDefaultReader(stream));
  if (queueTotalSize) {
    assert(!readableStreamGetNumReadRequests(stream));
    readableByteStreamControllerFillReadRequestFromQueue(
      controller,
      readRequest,
    );
    return;
  }
  const {
    autoAllocateChunkSize,
  } = controller[kState];
  if (autoAllocateChunkSize !== undefined) {
    try {
      const buffer = new ArrayBuffer(autoAllocateChunkSize);
      materializePendingPullIntos(controller[kState]).push(
        {
          buffer,
          bufferByteLength: autoAllocateChunkSize,
          byteOffset: 0,
          byteLength: autoAllocateChunkSize,
          bytesFilled: 0,
          minimumFill: 1,
          elementSize: 1,
          ctor: Uint8Array,
          type: 'default',
        });
    } catch (error) {
      readRequest[kError](error);
      return;
    }
  }

  readableStreamAddReadRequest(stream, readRequest);
  readableByteStreamControllerCallPullIfNeeded(controller);
}

function setupReadableByteStreamController(
  stream,
  controller,
  startAlgorithm,
  pullAlgorithm,
  cancelAlgorithm,
  highWaterMark,
  autoAllocateChunkSize) {
  assert(stream[kState].controller === undefined);
  if (autoAllocateChunkSize !== undefined) {
    assert(NumberIsInteger(autoAllocateChunkSize));
    assert(autoAllocateChunkSize > 0);
  }
  controller[kState] = {
    byobRequest: null,
    closeRequested: false,
    pullAgain: false,
    pulling: false,
    pullFulfilled: undefined,
    pullRejected: undefined,
    started: false,
    stream,
    queue: kEmptyQueue,
    queueTotalSize: 0,
    highWaterMark,
    pullAlgorithm,
    cancelAlgorithm,
    autoAllocateChunkSize,
    pendingPullIntos: kEmptyQueue,
  };
  stream[kState].controller = controller;

  const startResult = startAlgorithm();
  // See setupReadableStreamDefaultController.
  const startFulfilled = startResult === null ||
    (typeof startResult !== 'object' && typeof startResult !== 'function');

  if (startFulfilled && pullAlgorithm === nonOpCallback) {
    controller[kState].started = true;
    return;
  }

  const started = () => {
    controller[kState].started = true;
    assert(!controller[kState].pulling);
    assert(!controller[kState].pullAgain);
    readableByteStreamControllerCallPullIfNeeded(controller);
  };

  if (startFulfilled) {
    queueMicrotask(started);
    return;
  }

  PromisePrototypeThen(
    new Promise((r) => r(startResult)),
    started,
    (error) => readableByteStreamControllerError(controller, error));
}

function setupReadableByteStreamControllerFromSource(
  stream,
  source,
  highWaterMark) {
  const controller = new ReadableByteStreamController(kSkipThrow);
  const start = source?.start;
  const pull = source?.pull;
  const cancel = source?.cancel;
  const autoAllocateChunkSize = source?.autoAllocateChunkSize;
  const startAlgorithm = start ?
    FunctionPrototypeBind(start, source, controller) :
    nonOpCallback;
  const pullAlgorithm = pull ?
    createRawCallback1Param('source.pull', pull, source) :
    nonOpCallback;
  const cancelAlgorithm = cancel ?
    createPromiseCallback1Param('source.cancel', cancel, source) :
    nonOpCancel;

  if (autoAllocateChunkSize === 0) {
    throw new ERR_INVALID_ARG_VALUE(
      'source.autoAllocateChunkSize',
      autoAllocateChunkSize);
  }
  setupReadableByteStreamController(
    stream,
    controller,
    startAlgorithm,
    pullAlgorithm,
    cancelAlgorithm,
    highWaterMark,
    autoAllocateChunkSize);
}

module.exports = {
  ReadableStream,
  ReadableStreamDefaultReader,
  ReadableStreamBYOBReader,
  ReadableStreamBYOBRequest,
  ReadableByteStreamController,
  ReadableStreamDefaultController,
  TransferredReadableStream,

  // Exported Brand Checks
  isReadableStream,
  isReadableByteStreamController,
  isReadableStreamBYOBRequest,
  isReadableStreamDefaultReader,
  isReadableStreamBYOBReader,
  isWritableStreamDefaultWriter,
  isWritableStreamDefaultController,

  readableStreamPipeTo,
  readableStreamTee,
  readableByteStreamControllerConvertPullIntoDescriptor,
  isReadableStreamLocked,
  readableStreamCancel,
  readableStreamClose,
  readableStreamError,
  readableStreamHasDefaultReader,
  readableStreamGetNumReadRequests,
  readableStreamHasBYOBReader,
  readableStreamGetNumReadIntoRequests,
  readableStreamFulfillReadRequest,
  readableStreamFulfillReadIntoRequest,
  readableStreamAddReadRequest,
  readableStreamAddReadIntoRequest,
  readableStreamReaderGenericCancel,
  readableStreamReaderGenericInitialize,
  readableStreamReaderGenericRelease,
  readableStreamBYOBReaderRead,
  readableStreamDefaultReaderRead,
  kChunk,
  kClose,
  kError,
  setupReadableStreamBYOBReader,
  setupReadableStreamDefaultReader,
  readableStreamDefaultControllerClose,
  readableStreamDefaultControllerEnqueue,
  readableStreamDefaultControllerHasBackpressure,
  readableStreamDefaultControllerCanCloseOrEnqueue,
  readableStreamDefaultControllerGetDesiredSize,
  readableStreamDefaultControllerShouldCallPull,
  readableStreamDefaultControllerCallPullIfNeeded,
  readableStreamDefaultControllerClearAlgorithms,
  readableStreamDefaultControllerError,
  readableStreamDefaultControllerCancelSteps,
  readableStreamDefaultControllerPullSteps,
  setupReadableStreamDefaultController,
  setupReadableStreamDefaultControllerFromSource,
  readableByteStreamControllerClose,
  readableByteStreamControllerCommitPullIntoDescriptor,
  readableByteStreamControllerInvalidateBYOBRequest,
  readableByteStreamControllerClearAlgorithms,
  readableByteStreamControllerClearPendingPullIntos,
  readableByteStreamControllerGetDesiredSize,
  readableByteStreamControllerShouldCallPull,
  readableByteStreamControllerHandleQueueDrain,
  readableByteStreamControllerPullInto,
  readableByteStreamControllerRespondInternal,
  readableByteStreamControllerRespond,
  readableByteStreamControllerRespondInClosedState,
  readableByteStreamControllerFillHeadPullIntoDescriptor,
  readableByteStreamControllerEnqueue,
  readableByteStreamControllerEnqueueChunkToQueue,
  readableByteStreamControllerFillPullIntoDescriptorFromQueue,
  readableByteStreamControllerProcessPullIntoDescriptorsUsingQueue,
  readableByteStreamControllerRespondInReadableState,
  readableByteStreamControllerRespondWithNewView,
  readableByteStreamControllerShiftPendingPullInto,
  readableByteStreamControllerCallPullIfNeeded,
  readableByteStreamControllerError,
  readableByteStreamControllerCancelSteps,
  readableByteStreamControllerPullSteps,
  setupReadableByteStreamController,
  setupReadableByteStreamControllerFromSource,
  createReadableStream,
  createReadableByteStream,
};
});

// ── lib/internal/webstreams/transformstream.js ─────────────────────────────────────────────
define('internal/webstreams/transformstream', (module, exports, require, primordials, internalBinding) => {
'use strict';

const {
  FunctionPrototypeCall,
  ObjectDefineProperties,
  ObjectSetPrototypeOf,
  PromisePrototypeThen,
  PromiseReject,
  PromiseResolve,
  PromiseWithResolvers,
  Symbol,
  SymbolToStringTag,
} = primordials;

const {
  codes: {
    ERR_ILLEGAL_CONSTRUCTOR,
    ERR_INVALID_ARG_VALUE,
    ERR_INVALID_STATE,
    ERR_INVALID_THIS,
  },
} = require('internal/errors');

const {
  DOMException,
} = internalBinding('messaging');

const {
  customInspectSymbol: kInspect,
  kEmptyObject,
  kEnumerableProperty,
} = require('internal/util');

const {
  validateObject,
  kValidateObjectAllowObjects,
  kValidateObjectAllowObjectsAndNull,
} = require('internal/validators');

const {
  kDeserialize,
  kTransfer,
  kTransferList,
  markTransferMode,
} = require('internal/worker/js_transferable');

const {
  createPromiseCallback1Param,
  createRawCallback2Params,
  customInspect,
  extractHighWaterMark,
  extractSizeAlgorithm,
  getNonWritablePropertyDescriptor,
  isBrandCheck,
  kParkedAlgorithmResult,
  kResolvedPromise,
  kState,
  kType,
  nonOpCancel,
  nonOpFlush,
} = require('internal/webstreams/util');

const {
  createReadableStream,
  readableStreamDefaultControllerCanCloseOrEnqueue,
  readableStreamDefaultControllerClose,
  readableStreamDefaultControllerEnqueue,
  readableStreamDefaultControllerError,
  readableStreamDefaultControllerGetDesiredSize,
  readableStreamDefaultControllerHasBackpressure,
} = require('internal/webstreams/readablestream');

const {
  createWritableStream,
  writableStreamDefaultControllerErrorIfNeeded,
} = require('internal/webstreams/writablestream');

const assert = require('internal/assert');

const kSkipThrow = Symbol('kSkipThrow');

/**
 * @typedef {import('./queuingstrategies').QueuingStrategy
 * } QueuingStrategy
 * @typedef {import('./queuingstrategies').QueuingStrategySize
 * } QueuingStrategySize
 */

/**
 * @callback TransformerStartCallback
 * @param {TransformStreamDefaultController} controller
 */

/**
 * @callback TransformerFlushCallback
 * @param {TransformStreamDefaultController} controller
 * @returns {Promise<void>}
 */

/**
 * @callback TransformerTransformCallback
 * @param {any} chunk
 * @param {TransformStreamDefaultController} controller
 * @returns {Promise<void>}
 */

/**
 * @typedef {{
 *  start? : TransformerStartCallback,
 *  transform? : TransformerTransformCallback,
 *  flush? : TransformerFlushCallback,
 *  readableType? : any,
 *  writableType? : any,
 * }} Transformer
 */

class TransformStream {
  [kType] = 'TransformStream';

  /**
   * @param {Transformer} [transformer]
   * @param {QueuingStrategy} [writableStrategy]
   * @param {QueuingStrategy} [readableStrategy]
   */
  constructor(
    transformer = kEmptyObject,
    writableStrategy = kEmptyObject,
    readableStrategy = kEmptyObject) {
    markTransferMode(this, false, true);
    validateObject(transformer, 'transformer', kValidateObjectAllowObjects);
    validateObject(writableStrategy, 'writableStrategy', kValidateObjectAllowObjectsAndNull);
    validateObject(readableStrategy, 'readableStrategy', kValidateObjectAllowObjectsAndNull);
    const readableType = transformer?.readableType;
    const writableType = transformer?.writableType;
    const start = transformer?.start;

    if (readableType !== undefined) {
      throw new ERR_INVALID_ARG_VALUE.RangeError(
        'transformer.readableType',
        readableType);
    }
    if (writableType !== undefined) {
      throw new ERR_INVALID_ARG_VALUE.RangeError(
        'transformer.writableType',
        writableType);
    }

    const readableHighWaterMark = readableStrategy?.highWaterMark;
    const readableSize = readableStrategy?.size;

    const writableHighWaterMark = writableStrategy?.highWaterMark;
    const writableSize = writableStrategy?.size;

    const actualReadableHighWaterMark =
      extractHighWaterMark(readableHighWaterMark, 0);
    const actualReadableSize = extractSizeAlgorithm(readableSize);

    const actualWritableHighWaterMark =
      extractHighWaterMark(writableHighWaterMark, 1);
    const actualWritableSize = extractSizeAlgorithm(writableSize);

    // Without a start() the start promise is already resolved by the time
    // the readable and writable sides adopt it, so the shared resolved
    // promise stands in for the record.
    const startPromise = start !== undefined ? PromiseWithResolvers() : undefined;

    initializeTransformStream(
      this,
      startPromise,
      actualWritableHighWaterMark,
      actualWritableSize,
      actualReadableHighWaterMark,
      actualReadableSize);

    setupTransformStreamDefaultControllerFromTransformer(this, transformer);

    if (start !== undefined) {
      startPromise.resolve(
        FunctionPrototypeCall(
          start,
          transformer,
          this[kState].controller));
    }
  }

  /**
   * @readonly
   * @type {ReadableStream}
   */
  get readable() {
    if (!isTransformStream(this))
      throw new ERR_INVALID_THIS('TransformStream');
    return this[kState].readable;
  }

  /**
   * @readonly
   * @type {WritableStream}
   */
  get writable() {
    if (!isTransformStream(this))
      throw new ERR_INVALID_THIS('TransformStream');
    return this[kState].writable;
  }

  [kInspect](depth, options) {
    return customInspect(depth, options, this[kType], {
      readable: this.readable,
      writable: this.writable,
      backpressure: this[kState].backpressure,
    });
  }

  [kTransfer]() {
    if (!isTransformStream(this))
      throw new ERR_INVALID_THIS('TransformStream');
    const {
      readable,
      writable,
    } = this[kState];
    if (readable.locked) {
      throw new DOMException(
        'Cannot transfer a locked ReadableStream',
        'DataCloneError');
    }
    if (writable.locked) {
      throw new DOMException(
        'Cannot transfer a locked WritableStream',
        'DataCloneError');
    }
    return {
      data: {
        readable,
        writable,
      },
      deserializeInfo:
        'internal/webstreams/transformstream:TransferredTransformStream',
    };
  }

  [kTransferList]() {
    return [ this[kState].readable, this[kState].writable ];
  }

  [kDeserialize]({ readable, writable }) {
    this[kState].readable = readable;
    this[kState].writable = writable;
  }
}

ObjectDefineProperties(TransformStream.prototype, {
  readable: kEnumerableProperty,
  writable: kEnumerableProperty,
  [SymbolToStringTag]: getNonWritablePropertyDescriptor(TransformStream.name),
});

// The state record classes null out their prototype chain to keep
// Object.prototype out of property lookups; every field ever assigned
// is declared so the shape never transitions.
class TransformStreamState {
  readable = undefined;
  writable = undefined;
  controller = undefined;
  backpressure = undefined;
  // Continuation slots replacing the spec's
  // [[backpressureChangePromise]]; see transformStreamSetBackpressure.
  pullPending = false;
  pendingWriteParked = false;
  pendingWriteChunk = undefined;
  writeContinuation = undefined;
}
ObjectSetPrototypeOf(TransformStreamState.prototype, null);

class TransformStreamControllerState {
  stream = undefined;
  transformAlgorithm = undefined;
  flushAlgorithm = undefined;
  cancelAlgorithm = undefined;
  performTransformRejected = undefined;
  finishPromise = undefined;
}
ObjectSetPrototypeOf(TransformStreamControllerState.prototype, null);

function InternalTransferredTransformStream() {
  ObjectSetPrototypeOf(this, TransformStream.prototype);
  markTransferMode(this, false, true);
  this[kType] = 'TransformStream';
  this[kState] = new TransformStreamState();
}

ObjectSetPrototypeOf(InternalTransferredTransformStream.prototype, TransformStream.prototype);
ObjectSetPrototypeOf(InternalTransferredTransformStream, TransformStream);

function TransferredTransformStream() {
  const stream = new InternalTransferredTransformStream();

  stream.constructor = TransformStream;

  return stream;
}

TransferredTransformStream.prototype[kDeserialize] = () => {};

class TransformStreamDefaultController {
  [kType] = 'TransformStreamDefaultController';
  [kState];

  constructor(skipThrowSymbol = undefined) {
    if (skipThrowSymbol !== kSkipThrow) {
      throw new ERR_ILLEGAL_CONSTRUCTOR();
    }
  }

  /**
   * @readonly
   * @type {number}
   */
  get desiredSize() {
    if (!isTransformStreamDefaultController(this))
      throw new ERR_INVALID_THIS('TransformStreamDefaultController');
    const {
      stream,
    } = this[kState];
    const {
      readable,
    } = stream[kState];
    const {
      controller: readableController,
    } = readable[kState];
    return readableStreamDefaultControllerGetDesiredSize(readableController);
  }

  /**
   * @param {any} [chunk]
   */
  enqueue(chunk = undefined) {
    if (!isTransformStreamDefaultController(this))
      throw new ERR_INVALID_THIS('TransformStreamDefaultController');
    transformStreamDefaultControllerEnqueue(this, chunk);
  }

  /**
   * @param {any} [reason]
   */
  error(reason = undefined) {
    if (!isTransformStreamDefaultController(this))
      throw new ERR_INVALID_THIS('TransformStreamDefaultController');
    transformStreamDefaultControllerError(this, reason);
  }

  terminate() {
    if (!isTransformStreamDefaultController(this))
      throw new ERR_INVALID_THIS('TransformStreamDefaultController');
    transformStreamDefaultControllerTerminate(this);
  }

  [kInspect](depth, options) {
    return customInspect(depth, options, this[kType], {
      stream: this[kState].stream,
    });
  }
}

ObjectDefineProperties(TransformStreamDefaultController.prototype, {
  desiredSize: kEnumerableProperty,
  enqueue: kEnumerableProperty,
  error: kEnumerableProperty,
  terminate: kEnumerableProperty,
  [SymbolToStringTag]: getNonWritablePropertyDescriptor(TransformStreamDefaultController.name),
});

const isTransformStream =
  isBrandCheck('TransformStream');
const isTransformStreamDefaultController =
  isBrandCheck('TransformStreamDefaultController');

// Raw callback (see createRawCallback*): invoked inside the try/catch of
// transformStreamDefaultControllerPerformTransform.
function defaultTransformAlgorithm(chunk, controller) {
  transformStreamDefaultControllerEnqueue(controller, chunk);
}

function resolvedStartAlgorithm() {
  return kResolvedPromise;
}

function initializeTransformStream(
  stream,
  startPromise,
  writableHighWaterMark,
  writableSizeAlgorithm,
  readableHighWaterMark,
  readableSizeAlgorithm) {

  const startAlgorithm = startPromise === undefined ?
    resolvedStartAlgorithm :
    () => startPromise.promise;

  const writable = createWritableStream(
    startAlgorithm,
    (chunk) => transformStreamDefaultSinkWriteAlgorithm(stream, chunk),
    () => transformStreamDefaultSinkCloseAlgorithm(stream),
    (reason) => transformStreamDefaultSinkAbortAlgorithm(stream, reason),
    writableHighWaterMark,
    writableSizeAlgorithm,
  );

  const readable = createReadableStream(
    startAlgorithm,
    () => transformStreamDefaultSourcePullAlgorithm(stream),
    (reason) => transformStreamDefaultSourceCancelAlgorithm(stream, reason),
    readableHighWaterMark,
    readableSizeAlgorithm,
  );

  const state = new TransformStreamState();
  state.readable = readable;
  state.writable = writable;
  stream[kState] = state;

  transformStreamSetBackpressure(stream, true);
}

function transformStreamError(stream, error) {
  const {
    readable,
  } = stream[kState];
  const {
    controller,
  } = readable[kState];
  readableStreamDefaultControllerError(controller, error);
  transformStreamErrorWritableAndUnblockWrite(stream, error);
}

function transformStreamErrorWritableAndUnblockWrite(stream, error) {
  const {
    controller,
    writable,
  } = stream[kState];
  transformStreamDefaultControllerClearAlgorithms(controller);
  writableStreamDefaultControllerErrorIfNeeded(
    writable[kState].controller,
    error);
  transformStreamUnblockWrite(stream);
}

function transformStreamUnblockWrite(stream) {
  if (stream[kState].backpressure)
    transformStreamSetBackpressure(stream, false);
}

// The spec's [[backpressureChangePromise]] is only ever observed by the
// source pull algorithm (settles when backpressure next becomes true) and
// by a sink write arriving while backpressure is set (settles when
// backpressure next becomes false). Both observers are internal, so the
// promise record is replaced by continuation slots: a parked pull is
// completed by delivering the readable controller's pull-fulfilled step,
// and a parked write by the cached write continuation (see
// transformStreamDefaultSinkWriteAlgorithm). Each is enqueued on the
// shared resolved promise at the exact microtask position the old
// record's reaction would have had.
function transformStreamSetBackpressure(stream, backpressure) {
  const state = stream[kState];
  assert(state.backpressure !== backpressure);
  state.backpressure = backpressure;
  if (backpressure) {
    if (state.pullPending) {
      state.pullPending = false;
      // The pull-fulfilled step exists: a pull parked it (see
      // transformStreamDefaultSourcePullAlgorithm), and the readable
      // controller creates it before invoking the pull algorithm.
      PromisePrototypeThen(
        kResolvedPromise,
        state.readable[kState].controller[kState].pullFulfilled);
    }
  } else if (state.pendingWriteParked) {
    PromisePrototypeThen(kResolvedPromise, state.writeContinuation);
  }
}

function setupTransformStreamDefaultController(
  stream,
  controller,
  transformAlgorithm,
  flushAlgorithm,
  cancelAlgorithm) {
  assert(isTransformStream(stream));
  assert(stream[kState].controller === undefined);
  const controllerState = new TransformStreamControllerState();
  controllerState.stream = stream;
  controllerState.transformAlgorithm = transformAlgorithm;
  controllerState.flushAlgorithm = flushAlgorithm;
  controllerState.cancelAlgorithm = cancelAlgorithm;
  controller[kState] = controllerState;
  stream[kState].controller = controller;
}

function setupTransformStreamDefaultControllerFromTransformer(
  stream,
  transformer) {
  const controller = new TransformStreamDefaultController(kSkipThrow);
  const transform = transformer?.transform;
  const flush = transformer?.flush;
  const cancel = transformer?.cancel;
  const transformAlgorithm = transform ?
    createRawCallback2Params('transformer.transform', transform, transformer) :
    defaultTransformAlgorithm;
  const flushAlgorithm = flush ?
    createPromiseCallback1Param('transformer.flush', flush, transformer) :
    nonOpFlush;
  const cancelAlgorithm = cancel ?
    createPromiseCallback1Param('transformer.cancel', cancel, transformer) :
    nonOpCancel;

  setupTransformStreamDefaultController(
    stream,
    controller,
    transformAlgorithm,
    flushAlgorithm,
    cancelAlgorithm);
}

function transformStreamDefaultControllerClearAlgorithms(controller) {
  controller[kState].transformAlgorithm = undefined;
  controller[kState].flushAlgorithm = undefined;
  controller[kState].cancelAlgorithm = undefined;
}

function transformStreamDefaultControllerEnqueue(controller, chunk) {
  const {
    stream,
  } = controller[kState];
  const {
    readable,
  } = stream[kState];
  const {
    controller: readableController,
  } = readable[kState];
  if (!readableStreamDefaultControllerCanCloseOrEnqueue(readableController))
    throw new ERR_INVALID_STATE.TypeError('Unable to enqueue');
  try {
    readableStreamDefaultControllerEnqueue(readableController, chunk);
  } catch (error) {
    transformStreamErrorWritableAndUnblockWrite(stream, error);
    throw readable[kState].storedError;
  }
  const backpressure =
    readableStreamDefaultControllerHasBackpressure(readableController);
  if (backpressure !== stream[kState].backpressure) {
    assert(backpressure);
    transformStreamSetBackpressure(stream, true);
  }
}

function transformStreamDefaultControllerError(controller, error) {
  transformStreamError(controller[kState].stream, error);
}

// Mirrors the reference implementation's
// `promiseCall(transformAlgorithm, ...).then(undefined, rejectionSteps)`:
// the returned promise settles one microtask after the (coerced) result
// does, and a rejection errors the transform stream before propagating.
// The raw transform callback plus the shared resolved promise for
// non-thenable results replace the previous async wrapper's two implicit
// promises per chunk.
function transformStreamDefaultControllerPerformTransform(controller, chunk) {
  const controllerState = controller[kState];
  const transformAlgorithm = controllerState.transformAlgorithm;
  if (transformAlgorithm === undefined) {
    // Algorithms were cleared by a concurrent cancel/abort/close.
    return kResolvedPromise;
  }
  let result;
  try {
    result = transformAlgorithm(chunk, controller);
  } catch (error) {
    result = PromiseReject(error);
  }
  if (result === null ||
      (typeof result !== 'object' && typeof result !== 'function')) {
    result = kResolvedPromise;
  } else {
    result = PromiseResolve(result);
  }
  controllerState.performTransformRejected ??= (error) => {
    transformStreamError(controller[kState].stream, error);
    throw error;
  };
  return PromisePrototypeThen(
    result,
    undefined,
    controllerState.performTransformRejected);
}

function transformStreamDefaultControllerTerminate(controller) {
  const {
    stream,
  } = controller[kState];
  const {
    readable,
  } = stream[kState];
  assert(readable !== undefined);
  const {
    controller: readableController,
  } = readable[kState];
  readableStreamDefaultControllerClose(readableController);
  transformStreamErrorWritableAndUnblockWrite(
    stream,
    new ERR_INVALID_STATE.TypeError('TransformStream has been terminated'));
}

function transformStreamDefaultSinkWriteAlgorithm(stream, chunk) {
  const state = stream[kState];
  const {
    writable,
    controller,
  } = state;
  assert(writable[kState].state === 'writable');
  if (state.backpressure) {
    // Park the chunk; the backpressure -> false flip delivers the cached
    // continuation (see transformStreamSetBackpressure) at the same
    // microtask position as the old [[backpressureChangePromise]]
    // reaction. The continuation completes the parked write by wiring
    // the perform-transform promise directly to the writable
    // controller's write reactions (they exist: the controller creates
    // them before invoking the write algorithm), replacing the promise
    // record the old code allocated and resolved per parked chunk. The
    // writable dispatches a single write at a time, so one pending slot
    // suffices.
    assert(!state.pendingWriteParked);
    state.pendingWriteParked = true;
    state.pendingWriteChunk = chunk;
    state.writeContinuation ??= () => {
      const pendingChunk = state.pendingWriteChunk;
      state.pendingWriteParked = false;
      state.pendingWriteChunk = undefined;
      const writableState = state.writable[kState];
      const writableControllerState = writableState.controller[kState];
      if (writableState.state === 'erroring') {
        const error = writableState.storedError;
        PromisePrototypeThen(
          kResolvedPromise,
          () => writableControllerState.writeRejected(error));
        return;
      }
      assert(writableState.state === 'writable');
      PromisePrototypeThen(
        transformStreamDefaultControllerPerformTransform(
          controller,
          pendingChunk),
        writableControllerState.writeFulfilled,
        writableControllerState.writeRejected);
    };
    return kParkedAlgorithmResult;
  }
  return transformStreamDefaultControllerPerformTransform(controller, chunk);
}

async function transformStreamDefaultSinkAbortAlgorithm(stream, reason) {
  const {
    controller,
    readable,
  } = stream[kState];

  if (controller[kState].finishPromise !== undefined) {
    return controller[kState].finishPromise;
  }

  const { promise, resolve, reject } = PromiseWithResolvers();
  controller[kState].finishPromise = promise;
  const cancelPromise = controller[kState].cancelAlgorithm(reason);
  transformStreamDefaultControllerClearAlgorithms(controller);

  PromisePrototypeThen(
    cancelPromise,
    () => {
      if (readable[kState].state === 'errored')
        reject(readable[kState].storedError);
      else {
        readableStreamDefaultControllerError(readable[kState].controller, reason);
        resolve();
      }
    },
    (error) => {
      readableStreamDefaultControllerError(readable[kState].controller, error);
      reject(error);
    },
  );

  return controller[kState].finishPromise;
}

function transformStreamDefaultSinkCloseAlgorithm(stream) {
  const {
    readable,
    controller,
  } = stream[kState];

  if (controller[kState].finishPromise !== undefined) {
    return controller[kState].finishPromise;
  }
  const { promise, resolve, reject } = PromiseWithResolvers();
  controller[kState].finishPromise = promise;
  const flushPromise = controller[kState].flushAlgorithm(controller);
  transformStreamDefaultControllerClearAlgorithms(controller);
  PromisePrototypeThen(
    flushPromise,
    () => {
      if (readable[kState].state === 'errored')
        reject(readable[kState].storedError);
      else {
        readableStreamDefaultControllerClose(readable[kState].controller);
        resolve();
      }
    },
    (error) => {
      readableStreamDefaultControllerError(readable[kState].controller, error);
      reject(error);
    });
  return controller[kState].finishPromise;
}

function transformStreamDefaultSourcePullAlgorithm(stream) {
  const state = stream[kState];
  assert(state.backpressure);
  transformStreamSetBackpressure(stream, false);
  // Park the pull: the next backpressure -> true flip delivers the
  // pull-fulfilled step (see transformStreamSetBackpressure). The old
  // [[backpressureChangePromise]] this replaces was only ever resolved,
  // so the parked pull needs no rejection delivery.
  state.pullPending = true;
  return kParkedAlgorithmResult;
}

function transformStreamDefaultSourceCancelAlgorithm(stream, reason) {
  const {
    controller,
    writable,
  } = stream[kState];

  if (controller[kState].finishPromise !== undefined) {
    return controller[kState].finishPromise;
  }

  const { promise, resolve, reject } = PromiseWithResolvers();
  controller[kState].finishPromise = promise;
  const cancelPromise = controller[kState].cancelAlgorithm(reason);
  transformStreamDefaultControllerClearAlgorithms(controller);

  PromisePrototypeThen(
    cancelPromise,
    () => {
      if (writable[kState].state === 'errored')
        reject(writable[kState].storedError);
      else {
        writableStreamDefaultControllerErrorIfNeeded(
          writable[kState].controller,
          reason);
        transformStreamUnblockWrite(stream);
        resolve();
      }
    },
    (error) => {
      writableStreamDefaultControllerErrorIfNeeded(
        writable[kState].controller,
        error);
      transformStreamUnblockWrite(stream);
      reject(error);
    },
  );

  return controller[kState].finishPromise;
}

module.exports = {
  TransformStream,
  TransformStreamDefaultController,
  TransferredTransformStream,

  // Exported Brand Checks
  isTransformStream,
  isTransformStreamDefaultController,
};
});

// ── lib/internal/webstreams/encoding.js ────────────────────────────────────────────────────
define('internal/webstreams/encoding', (module, exports, require, primordials, internalBinding) => {
'use strict';

const {
  ObjectDefineProperties,
  String,
  StringPrototypeCharCodeAt,
  StringPrototypeSlice,
  Uint8Array,
} = primordials;

const {
  TextDecoder,
  TextEncoder,
} = require('internal/encoding');

const {
  TransformStream,
} = require('internal/webstreams/transformstream');

const { customInspect } = require('internal/webstreams/util');

const {
  codes: {
    ERR_INVALID_ARG_TYPE,
    ERR_INVALID_THIS,
  },
} = require('internal/errors');

const {
  customInspectSymbol: kInspect,
  kEmptyObject,
  kEnumerableProperty,
} = require('internal/util');

// Shared per-chunk decode options; decode() only reads the flag.
const kDecodeStreamingOptions = { __proto__: null, stream: true };

/**
 * @typedef {import('./readablestream').ReadableStream} ReadableStream
 * @typedef {import('./writablestream').WritableStream} WritableStream
 */

class TextEncoderStream {
  #pendingHighSurrogate = null;
  #handle;
  #transform;

  constructor() {
    this.#handle = new TextEncoder();
    this.#transform = new TransformStream({
      transform: (chunk, controller) => {
        // https://encoding.spec.whatwg.org/#encode-and-enqueue-a-chunk
        // The only cross-chunk state is a trailing high surrogate;
        // encode() replaces interior lone surrogates with U+FFFD exactly
        // like the spec's per-code-unit walk.
        chunk = String(chunk);
        if (chunk.length === 0)
          return;
        if (this.#pendingHighSurrogate !== null) {
          chunk = this.#pendingHighSurrogate + chunk;
          this.#pendingHighSurrogate = null;
        }
        const lastCodeUnit =
          StringPrototypeCharCodeAt(chunk, chunk.length - 1);
        if (0xD800 <= lastCodeUnit && lastCodeUnit <= 0xDBFF) {
          this.#pendingHighSurrogate =
            StringPrototypeSlice(chunk, -1);
          chunk = StringPrototypeSlice(chunk, 0, -1);
          if (chunk.length === 0)
            return;
        }
        controller.enqueue(this.#handle.encode(chunk));
      },
      flush: (controller) => {
        // https://encoding.spec.whatwg.org/#encode-and-flush
        if (this.#pendingHighSurrogate !== null) {
          controller.enqueue(new Uint8Array([0xEF, 0xBF, 0xBD]));
        }
      },
    });
  }

  /**
   * @readonly
   * @type {string}
   */
  get encoding() {
    return this.#handle.encoding;
  }

  /**
   * @readonly
   * @type {ReadableStream}
   */
  get readable() {
    return this.#transform.readable;
  }

  /**
   * @readonly
   * @type {WritableStream}
   */
  get writable() {
    return this.#transform.writable;
  }

  [kInspect](depth, options) {
    if (this == null)
      throw new ERR_INVALID_THIS('TextEncoderStream');
    return customInspect(depth, options, 'TextEncoderStream', {
      encoding: this.#handle.encoding,
      readable: this.#transform.readable,
      writable: this.#transform.writable,
    });
  }
}

class TextDecoderStream {
  #handle;
  #transform;

  /**
   * @param {string} [encoding]
   * @param {{
   *   fatal? : boolean,
   *   ignoreBOM? : boolean,
   * }} [options]
   */
  constructor(encoding = 'utf-8', options = kEmptyObject) {
    this.#handle = new TextDecoder(encoding, options);
    this.#transform = new TransformStream({
      transform: (chunk, controller) => {
        if (chunk === undefined) {
          throw new ERR_INVALID_ARG_TYPE('chunk', 'string', chunk);
        }
        const value = this.#handle.decode(chunk, kDecodeStreamingOptions);
        if (value)
          controller.enqueue(value);
      },
      flush: (controller) => {
        const value = this.#handle.decode();
        if (value)
          controller.enqueue(value);
        controller.terminate();
      },
    });
  }

  /**
   * @readonly
   * @type {string}
   */
  get encoding() {
    return this.#handle.encoding;
  }

  /**
   * @readonly
   * @type {boolean}
   */
  get fatal() {
    return this.#handle.fatal;
  }

  /**
   * @readonly
   * @type {boolean}
   */
  get ignoreBOM() {
    return this.#handle.ignoreBOM;
  }

  /**
   * @readonly
   * @type {ReadableStream}
   */
  get readable() {
    return this.#transform.readable;
  }

  /**
   * @readonly
   * @type {WritableStream}
   */
  get writable() {
    return this.#transform.writable;
  }

  [kInspect](depth, options) {
    if (this == null)
      throw new ERR_INVALID_THIS('TextDecoderStream');
    return customInspect(depth, options, 'TextDecoderStream', {
      encoding: this.#handle.encoding,
      fatal: this.#handle.fatal,
      ignoreBOM: this.#handle.ignoreBOM,
      readable: this.#transform.readable,
      writable: this.#transform.writable,
    });
  }
}

ObjectDefineProperties(TextEncoderStream.prototype, {
  encoding: kEnumerableProperty,
  readable: kEnumerableProperty,
  writable: kEnumerableProperty,
});

ObjectDefineProperties(TextDecoderStream.prototype, {
  encoding: kEnumerableProperty,
  fatal: kEnumerableProperty,
  ignoreBOM: kEnumerableProperty,
  readable: kEnumerableProperty,
  writable: kEnumerableProperty,
});

module.exports = {
  TextEncoderStream,
  TextDecoderStream,
};
});


// `lib/internal/webstreams/compression.js`, with the Duplex adapter replaced
// (see the header): the format converter, the BufferSource chunk check, the
// accessors, `inspect` and `Symbol.toStringTag` are node's.
define('internal/webstreams/compression', (module, exports, require, primordials) => {
  const {
    ObjectDefineProperties,
    SymbolToStringTag,
  } = primordials;
  const { TransformStream } = require('internal/webstreams/transformstream');
  const { customInspect } = require('internal/webstreams/util');
  const { isArrayBufferView, isSharedArrayBuffer } = require('internal/util/types');
  const { customInspectSymbol: kInspect, kEnumerableProperty } = require('internal/util');
  const { codes: { ERR_INVALID_ARG_TYPE } } = require('internal/errors');
  const { createEnumConverter } = require('internal/webidl');

  let zlib;
  function lazyZlib() {
    zlib ??= require('zlib');
    return zlib;
  }

  function validateBufferSourceChunk(chunk) {
    if (isSharedArrayBuffer(isArrayBufferView(chunk) ? chunk.buffer : chunk)) {
      throw new ERR_INVALID_ARG_TYPE(
        'chunk',
        ['ArrayBuffer', 'Buffer', 'TypedArray', 'DataView'],
        chunk,
      );
    }
  }

  const formatConverter = createEnumConverter('CompressionFormat', [
    'deflate',
    'deflate-raw',
    'gzip',
    'brotli',
  ]);

  // A transform that collects the written bytes and runs `codec` over them
  // when the writable closes.
  function codecTransform(codec) {
    const parts = [];
    return new TransformStream({
      transform(chunk) {
        validateBufferSourceChunk(chunk);
        let bytes;
        if (Buffer.isBuffer(chunk) || typeof chunk === 'string') {
          bytes = Buffer.from(chunk);
        } else if (isArrayBufferView(chunk)) {
          bytes = Buffer.from(chunk.buffer, chunk.byteOffset, chunk.byteLength);
        } else {
          bytes = Buffer.from(chunk);
        }
        parts.push(bytes);
      },
      flush(controller) {
        const out = codec(Buffer.concat(parts));
        if (out.byteLength > 0) {
          controller.enqueue(new Uint8Array(out));
        }
      },
    });
  }

  const compressors = {
    'deflate': () => lazyZlib().deflateSync,
    'deflate-raw': () => lazyZlib().deflateRawSync,
    'gzip': () => lazyZlib().gzipSync,
    'brotli': () => lazyZlib().brotliCompressSync,
  };
  const decompressors = {
    'deflate': () => lazyZlib().inflateSync,
    'deflate-raw': () => lazyZlib().inflateRawSync,
    'gzip': () => lazyZlib().gunzipSync,
    'brotli': () => lazyZlib().brotliDecompressSync,
  };

  class CompressionStream {
    #transform;

    constructor(format) {
      format = formatConverter(format, {
        prefix: "Failed to construct 'CompressionStream'",
        context: '1st argument',
      });
      this.#transform = codecTransform(compressors[format]());
    }

    get readable() {
      return this.#transform.readable;
    }

    get writable() {
      return this.#transform.writable;
    }

    [kInspect](depth, options) {
      return customInspect(depth, options, 'CompressionStream', {
        readable: this.#transform.readable,
        writable: this.#transform.writable,
      });
    }
  }

  class DecompressionStream {
    #transform;

    constructor(format) {
      format = formatConverter(format, {
        prefix: "Failed to construct 'DecompressionStream'",
        context: '1st argument',
      });
      this.#transform = codecTransform(decompressors[format]());
    }

    get readable() {
      return this.#transform.readable;
    }

    get writable() {
      return this.#transform.writable;
    }

    [kInspect](depth, options) {
      return customInspect(depth, options, 'DecompressionStream', {
        readable: this.#transform.readable,
        writable: this.#transform.writable,
      });
    }
  }

  ObjectDefineProperties(CompressionStream.prototype, {
    readable: kEnumerableProperty,
    writable: kEnumerableProperty,
    [SymbolToStringTag]: {
      __proto__: null,
      configurable: true,
      value: 'CompressionStream',
    },
  });

  ObjectDefineProperties(DecompressionStream.prototype, {
    readable: kEnumerableProperty,
    writable: kEnumerableProperty,
    [SymbolToStringTag]: {
      __proto__: null,
      configurable: true,
      value: 'DecompressionStream',
    },
  });

  module.exports = {
    CompressionStream,
    DecompressionStream,
  };
});

// `lib/stream/web.js`.
define('stream/web', (module, exports, require) => {
  const {
    TransformStream,
    TransformStreamDefaultController,
  } = require('internal/webstreams/transformstream');

  const {
    WritableStream,
    WritableStreamDefaultController,
    WritableStreamDefaultWriter,
  } = require('internal/webstreams/writablestream');

  const {
    ReadableStream,
    ReadableStreamDefaultReader,
    ReadableStreamBYOBReader,
    ReadableStreamBYOBRequest,
    ReadableByteStreamController,
    ReadableStreamDefaultController,
    isReadableStream,
    readableStreamTee,
  } = require('internal/webstreams/readablestream');

  const {
    codes: {
      ERR_INVALID_ARG_TYPE,
    },
  } = require('internal/errors');

  function validateBoolean(value, name) {
    if (typeof value !== 'boolean') throw new ERR_INVALID_ARG_TYPE(name, 'boolean', value);
  }

  const {
    ByteLengthQueuingStrategy,
    CountQueuingStrategy,
  } = require('internal/webstreams/queuingstrategies');

  const {
    TextEncoderStream,
    TextDecoderStream,
  } = require('internal/webstreams/encoding');

  const {
    CompressionStream,
    DecompressionStream,
  } = require('internal/webstreams/compression');

  function ReadableStreamTee(stream, cloneForBranch2 = false) {
    if (!isReadableStream(stream)) {
      throw new ERR_INVALID_ARG_TYPE('stream', 'ReadableStream', stream);
    }
    validateBoolean(cloneForBranch2, 'cloneForBranch2');
    return readableStreamTee(stream, cloneForBranch2);
  }

  module.exports = {
    ReadableStream,
    ReadableStreamTee,
    ReadableStreamDefaultReader,
    ReadableStreamBYOBReader,
    ReadableStreamBYOBRequest,
    ReadableByteStreamController,
    ReadableStreamDefaultController,
    TransformStream,
    TransformStreamDefaultController,
    WritableStream,
    WritableStreamDefaultWriter,
    WritableStreamDefaultController,
    ByteLengthQueuingStrategy,
    CountQueuingStrategy,
    TextEncoderStream,
    TextDecoderStream,
    CompressionStream,
    DecompressionStream,
  };
});

// The module's exports, then the internals other ports and the host reach
// for: the brand checks `util.isReadableStream`-style callers need, and the
// lock-free cancel `FileHandle.readableWebStream` uses.
const readable = requireInternal('internal/webstreams/readablestream');
const writable = requireInternal('internal/webstreams/writablestream');
return {
  __proto__: null,
  web: requireInternal('stream/web'),
  isReadableStream: readable.isReadableStream,
  isWritableStream: writable.isWritableStream,
  readableStreamCancel: readable.readableStreamCancel,
};
})
