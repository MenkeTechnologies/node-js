// `events.on(emitter, event[, options])`, ported from node's `lib/events.js`
// (v26.12.1): an async iterator of the event's argument arrays, ending on
// `error` (which it rethrows once), on any of `options.close`, on `return()`,
// or on `options.signal`. `highWaterMark`/`lowWaterMark` pause and resume an
// emitter that has `pause`/`resume`. Evaluated once by `src/stdlib/events.rs`;
// the file is a function of `nodeError` (`util.@@nodeError`) returning `on`.
//
// node's `FixedQueue` is an array here, and the abort listener is removed
// directly rather than through `addAbortListener`'s disposable.
(function (nodeError) {
'use strict';

class AbortError extends Error {
  constructor(message = 'The operation was aborted', options = undefined) {
    super(message, options);
    this.code = 'ABORT_ERR';
    this.name = 'AbortError';
  }
}

function validateObject(value, name) {
  if (value === null || Array.isArray(value) || typeof value !== 'object') {
    nodeError('type', name, 'Object', value);
  }
}
function validateAbortSignal(signal, name) {
  if (signal !== undefined &&
      (signal === null || typeof signal !== 'object' || !('aborted' in signal))) {
    nodeError('type', name, 'an instance of AbortSignal', signal);
  }
}
function validateInteger(value, name, min = Number.MIN_SAFE_INTEGER, max = Number.MAX_SAFE_INTEGER) {
  if (typeof value !== 'number') nodeError('type', name, 'number', value);
  if (!Number.isInteger(value)) nodeError('range', name, 'an integer', value);
  if (value < min || value > max) nodeError('range', name, `>= ${min} && <= ${max}`, value);
}

const AsyncIteratorPrototype = Object.getPrototypeOf(
  Object.getPrototypeOf(async function* () {}).prototype);

function createIterResult(value, done) {
  return { done, value };
}

function eventTargetAgnosticRemoveListener(emitter, name, listener, flags) {
  if (typeof emitter.removeListener === 'function') {
    emitter.removeListener(name, listener);
  } else if (typeof emitter.removeEventListener === 'function') {
    emitter.removeEventListener(name, listener, flags);
  } else {
    nodeError('type', 'emitter', 'an instance of EventEmitter', emitter);
  }
}

function eventTargetAgnosticAddListener(emitter, name, listener, flags) {
  if (typeof emitter.on === 'function') {
    if (flags?.once) {
      emitter.once(name, listener);
    } else {
      emitter.on(name, listener);
    }
  } else if (typeof emitter.addEventListener === 'function') {
    emitter.addEventListener(name, listener, flags);
  } else {
    nodeError('type', 'emitter', 'an instance of EventEmitter', emitter);
  }
}

function listenersController() {
  const listeners = [];
  return {
    addEventListener(emitter, event, handler, flags) {
      eventTargetAgnosticAddListener(emitter, event, handler, flags);
      listeners.push([emitter, event, handler, flags]);
    },
    removeAll() {
      while (listeners.length > 0) {
        Reflect.apply(eventTargetAgnosticRemoveListener, undefined, listeners.pop());
      }
    },
  };
}

function on(emitter, event, options = {}) {
  validateObject(options, 'options');
  const signal = options.signal;
  validateAbortSignal(signal, 'options.signal');
  if (signal?.aborted)
    throw new AbortError(undefined, { cause: signal.reason });
  const highWatermark = options.highWaterMark ?? options.highWatermark ?? Number.MAX_SAFE_INTEGER;
  validateInteger(highWatermark, 'options.highWaterMark', 1);
  const lowWatermark = options.lowWaterMark ?? options.lowWatermark ?? 1;
  validateInteger(lowWatermark, 'options.lowWaterMark', 1);

  const unconsumedEvents = [];
  const unconsumedPromises = [];
  let paused = false;
  let error = null;
  let finished = false;
  let size = 0;

  const iterator = Object.setPrototypeOf({
    next() {
      if (size) {
        const value = unconsumedEvents.shift();
        size--;
        if (paused && size < lowWatermark) {
          emitter.resume();
          paused = false;
        }
        return Promise.resolve(createIterResult(value, false));
      }
      if (error) {
        const p = Promise.reject(error);
        error = null;
        return p;
      }
      if (finished) return closeHandler();
      return new Promise(function(resolve, reject) {
        unconsumedPromises.push({ resolve, reject });
      });
    },

    return() {
      return closeHandler();
    },

    throw(err) {
      if (!err || !(err instanceof Error)) {
        nodeError('type', 'EventEmitter.AsyncIterator', 'an instance of Error', err);
      }
      errorHandler(err);
    },
    [Symbol.asyncIterator]() {
      return this;
    },
  }, AsyncIteratorPrototype);

  const { addEventListener, removeAll } = listenersController();
  addEventListener(emitter, event, function(...args) {
    return eventHandler(args);
  });
  if (event !== 'error' && typeof emitter.on === 'function') {
    addEventListener(emitter, 'error', errorHandler);
  }
  const closeEvents = options?.close;
  if (closeEvents?.length) {
    for (let i = 0; i < closeEvents.length; i++) {
      addEventListener(emitter, closeEvents[i], closeHandler);
    }
  }

  if (signal) signal.addEventListener('abort', abortListener, { once: true });

  return iterator;

  function abortListener() {
    errorHandler(new AbortError(undefined, { cause: signal?.reason }));
  }

  function eventHandler(value) {
    if (unconsumedPromises.length === 0) {
      size++;
      if (!paused && size > highWatermark) {
        paused = true;
        emitter.pause();
      }
      unconsumedEvents.push(value);
    } else unconsumedPromises.shift().resolve(createIterResult(value, false));
  }

  function errorHandler(err) {
    if (unconsumedPromises.length === 0) error = err;
    else unconsumedPromises.shift().reject(err);
    closeHandler();
  }

  function closeHandler() {
    if (signal) signal.removeEventListener('abort', abortListener);
    removeAll();
    finished = true;
    paused = false;
    const doneResult = createIterResult(undefined, true);
    while (unconsumedPromises.length !== 0) {
      unconsumedPromises.shift().resolve(doneResult);
    }
    return Promise.resolve(doneResult);
  }
}

return on;
})
