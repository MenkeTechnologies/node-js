// `fs/promises`' `FileHandle` and `open`, ported from node's
// `lib/internal/fs/promises.js` (v26.12.0) for a runtime whose descriptor I/O
// is the synchronous `fs.*Sync` family: each binding call there is the
// matching sync call here, so validation, error codes and messages are the
// sync API's, which node shares. Evaluated once by `src/stdlib/fs_promises.rs`;
// the file is a function of `nodeError` (`util.@@nodeError`, node's coded
// errors) returning the object at its end.
//
// Kept from node: `fsCall`'s closed-handle `EBADF` (with the operation as
// `syscall`), the reference count that makes `close()` wait for an operation
// in flight, the `read`/`write` argument forms (a params object, a buffer plus
// an options object) and their null-prototype results, `readFile` from the
// current position until end of file, `writeFile`/`appendFile` writing from
// the current position, and `'close'` emitted once.
//
// `readableWebStream` delivers its reads as a default (not a byte) stream;
// see the method. Not modelled: the transfer symbols.
(function (nodeError) {
'use strict';
const fs = require('fs');
const EventEmitter = require('events');
const { StringDecoder } = require('string_decoder');

const kReadFileBufferLength = 512 * 1024;
const kReadFileUnknownBufferLength = 64 * 1024;
const kWriteFileMaxChunkSize = 512 * 1024;

const kFd = Symbol('kFd');
const kRefs = Symbol('kRefs');
const kClosePromise = Symbol('kClosePromise');
const kCloseResolve = Symbol('kCloseResolve');
const kCloseReject = Symbol('kCloseReject');
const kRef = Symbol('kRef');
const kUnref = Symbol('kUnref');
const kCloseSync = Symbol('kCloseSync');
const kLocked = Symbol('kLocked');
const kAsyncId = Symbol('kAsyncId');
let nextAsyncId = 1;

const isArrayBufferView = ArrayBuffer.isView;

// internal/validators
function validateInteger(value, name, min = Number.MIN_SAFE_INTEGER, max = Number.MAX_SAFE_INTEGER) {
  if (typeof value !== 'number') nodeError('type', name, 'number', value);
  if (!Number.isInteger(value)) nodeError('range', name, 'an integer', value);
  if (value < min || value > max) nodeError('range', name, `>= ${min} && <= ${max}`, value);
}
function validateObject(value, name) {
  if (value === null || Array.isArray(value) || typeof value !== 'object') {
    nodeError('type', name, 'Object', value);
  }
}
// internal/fs/utils
function validatePosition(position, name, length) {
  if (typeof position === 'number') {
    validateInteger(position, name, -1);
  } else if (typeof position === 'bigint') {
    const maxPosition = 2n ** 63n - 1n - BigInt(length);
    if (!(position >= -1n && position <= maxPosition)) {
      nodeError('code', 'ERR_OUT_OF_RANGE',
                `The value of "${name}" is out of range. It must be >= -1 && <= ${maxPosition}. Received ${position}n`);
    }
  } else {
    nodeError('type', name, 'of type bigint or integer', position);
  }
}

// The binding's promise-returning `close`, here the synchronous close settled.
function closeFd(fd) {
  return new Promise((resolve) => { fs.closeSync(fd); resolve(); });
}

class FileHandle extends EventEmitter {
  #brandCheck = undefined;

  static isFileHandle(value) {
    return (value != null && typeof value === 'object' && #brandCheck in value);
  }

  constructor(fd) {
    super();
    this[kFd] = fd === undefined ? -1 : fd;
    this[kRefs] = 1;
    this[kClosePromise] = null;
  }

  // The handle's async resource id: one number per handle, for the life of
  // the handle (node-js models no other async resources to number against).
  getAsyncId() {
    if (this[kAsyncId] === undefined) this[kAsyncId] = nextAsyncId++;
    return this[kAsyncId];
  }

  get fd() {
    return this[kFd];
  }

  appendFile(data, options) {
    return fsCall(writeFile, this, data, options);
  }

  chmod(mode) {
    return fsCall(fchmod, this, mode);
  }

  chown(uid, gid) {
    return fsCall(fchown, this, uid, gid);
  }

  datasync() {
    return fsCall(fdatasync, this);
  }

  sync() {
    return fsCall(fsync, this);
  }

  read(buffer, offset, length, position) {
    return fsCall(read, this, buffer, offset, length, position);
  }

  readv(buffers, position) {
    return fsCall(readv, this, buffers, position);
  }

  readFile(options) {
    return fsCall(readFile, this, options);
  }

  readLines(options = undefined) {
    return require('readline').createInterface({
      input: this.createReadStream(options),
      crlfDelay: Infinity,
    });
  }

  stat(options) {
    return fsCall(fstat, this, options);
  }

  truncate(len = 0) {
    return fsCall(ftruncate, this, len);
  }

  utimes(atime, mtime) {
    return fsCall(futimes, this, atime, mtime);
  }

  write(buffer, offset, length, position) {
    return fsCall(write, this, buffer, offset, length, position);
  }

  writev(buffers, position) {
    return fsCall(writev, this, buffers, position);
  }

  writeFile(data, options) {
    return fsCall(writeFile, this, data, options);
  }

  close = () => {
    if (this[kFd] === -1) {
      return Promise.resolve();
    }

    if (this[kClosePromise]) {
      return this[kClosePromise];
    }

    this[kRefs]--;
    if (this[kRefs] === 0) {
      const fd = this[kFd];
      this[kFd] = -1;
      this[kClosePromise] = closeFd(fd).finally(
        () => { this[kClosePromise] = undefined; },
      );
    } else {
      this[kClosePromise] = new Promise((resolve, reject) => {
        this[kCloseResolve] = resolve;
        this[kCloseReject] = reject;
      }).finally(() => {
        this[kClosePromise] = undefined;
        this[kCloseReject] = undefined;
        this[kCloseResolve] = undefined;
      });
    }

    this.emit('close');
    return this[kClosePromise];
  };

  [kCloseSync]() {
    if (this[kFd] === -1) return;
    const fd = this[kFd];
    this[kFd] = -1;
    fs.closeSync(fd);
    this.emit('close');
  }

  async [Symbol.asyncDispose]() {
    await this.close();
  }

  // node builds a `type: 'bytes'` stream that reads into each BYOB request
  // with an `autoAllocateChunkSize` of 16384. Byte streams are not
  // implemented here, so this is the same reads delivered as a default
  // stream: each pull reads up to 16384 bytes and enqueues them as a
  // Uint8Array, which is what a default reader of node's stream receives.
  readableWebStream(options = {}) {
    if (this[kFd] === -1)
      nodeError('error', 'ERR_INVALID_STATE', 'Invalid state: The FileHandle is closed');
    if (this[kClosePromise])
      nodeError('error', 'ERR_INVALID_STATE', 'Invalid state: The FileHandle is closing');
    if (this[kLocked])
      nodeError('error', 'ERR_INVALID_STATE', 'Invalid state: The FileHandle is locked');
    this[kLocked] = true;

    validateObject(options, 'options');
    const {
      type = 'bytes',
      autoClose = false,
    } = options;

    if (typeof autoClose !== 'boolean') nodeError('type', 'options.autoClose', 'boolean', autoClose);

    if (type !== 'bytes') {
      process.emitWarning(
        'A non-"bytes" options.type has no effect. A byte-oriented steam is ' +
        'always created.',
        'ExperimentalWarning',
      );
    }

    const readFn = this.read.bind(this);
    const ondone = async () => {
      this[kUnref]();
      if (autoClose) await this.close();
    };

    const readable = new ReadableStream({
      async pull(controller) {
        const view = new Uint8Array(16384);
        const { bytesRead } = await readFn(view, 0, view.byteLength);

        if (bytesRead === 0) {
          controller.close();
          await ondone();
        } else {
          controller.enqueue(view.subarray(0, bytesRead));
        }
      },

      async cancel() {
        await ondone();
      },
    });

    this[kRef]();
    this.once('close', () => {
      readable.cancel().catch(() => {});
    });

    return readable;
  }

  // node hands the stream the FileHandle itself, and the stream's autoClose
  // closes the HANDLE. The streams here take a descriptor, so the stream is
  // told not to close it and the handle is closed when the stream is.
  createReadStream(options = undefined) {
    return streamOverHandle(this, fs.createReadStream, 'end', options);
  }

  createWriteStream(options = undefined) {
    return streamOverHandle(this, fs.createWriteStream, 'finish', options);
  }

  [kRef]() {
    this[kRefs]++;
  }

  [kUnref]() {
    this[kRefs]--;
    if (this[kRefs] === 0) {
      const fd = this[kFd];
      this[kFd] = -1;
      closeFd(fd).then(this[kCloseResolve], this[kCloseReject]);
    }
  }
}

function streamOverHandle(handle, create, done, options) {
  const autoClose = options?.autoClose ?? true;
  const stream = create(undefined, { ...options, fd: handle.fd, autoClose: false });
  if (autoClose) stream.once(done, () => handle.close().then(() => stream.emit('close')));
  return stream;
}

async function fsCall(fn, handle, ...args) {
  if (handle.fd === -1) {
    const err = new Error('file closed');
    err.code = 'EBADF';
    err.syscall = fn.name;
    throw err;
  }

  try {
    handle[kRef]();
    return await fn(handle, ...args);
  } finally {
    handle[kUnref]();
  }
}

async function open(path, flags, mode) {
  return new FileHandle(fs.openSync(path, flags, mode));
}

async function read(handle, bufferOrParams, offset, length, position) {
  let buffer = bufferOrParams;
  if (!isArrayBufferView(buffer)) {
    // This is fh.read(params)
    if (bufferOrParams !== undefined) {
      // validateObject(bufferOrParams, 'options', kValidateObjectAllowNullable)
      if (Array.isArray(bufferOrParams) ||
          (bufferOrParams !== null && typeof bufferOrParams !== 'object')) {
        nodeError('type', 'options', 'Object', bufferOrParams);
      }
    }
    ({
      buffer = Buffer.alloc(16384),
      offset = 0,
      length = buffer.byteLength - offset,
      position = null,
    } = bufferOrParams ?? {});

    // validateBuffer(buffer)
    if (!isArrayBufferView(buffer)) {
      nodeError('type', 'buffer', 'an instance of Buffer, TypedArray, or DataView', buffer);
    }
  }

  if (offset !== null && typeof offset === 'object') {
    // This is fh.read(buffer, options)
    ({
      offset = 0,
      length = buffer.byteLength - offset,
      position = null,
    } = offset);
  }

  if (offset == null) {
    offset = 0;
  } else {
    validateInteger(offset, 'offset', 0);
  }

  length ??= buffer.byteLength - offset;
  length |= 0;

  if (position == null) {
    position = -1;
  } else {
    validatePosition(position, 'position', length);
  }

  if (length === 0)
    return { __proto__: null, bytesRead: length, buffer };

  const bytesRead = fs.readSync(handle.fd, buffer, offset, length,
                                position === -1 ? null : position) || 0;

  return { __proto__: null, bytesRead, buffer };
}

async function readv(handle, buffers, position) {
  if (typeof position !== 'number')
    position = null;

  const bytesRead = fs.readvSync(handle.fd, buffers, position) || 0;
  return { __proto__: null, bytesRead, buffers };
}

async function write(handle, buffer, offsetOrOptions, length, position) {
  if (buffer?.byteLength === 0)
    return { __proto__: null, bytesWritten: 0, buffer };

  let offset = offsetOrOptions;
  if (isArrayBufferView(buffer)) {
    if (typeof offset === 'object') {
      ({
        offset = 0,
        length = buffer.byteLength - offset,
        position = null,
      } = offsetOrOptions ?? {});
    }

    if (offset == null) {
      offset = 0;
    }
    if (typeof length !== 'number')
      length = buffer.byteLength - offset;
    if (typeof position !== 'number')
      position = null;
    const bytesWritten =
      fs.writeSync(handle.fd, buffer, offset, length, position) || 0;
    return { __proto__: null, bytesWritten, buffer };
  }

  const bytesWritten = fs.writeSync(handle.fd, buffer, offset, length) || 0;
  return { __proto__: null, bytesWritten, buffer };
}

async function writev(handle, buffers, position) {
  if (typeof position !== 'number')
    position = null;

  if (Array.isArray(buffers) && buffers.length === 0) {
    return { __proto__: null, bytesWritten: 0, buffers };
  }

  const bytesWritten = fs.writevSync(handle.fd, buffers, position) || 0;
  return { __proto__: null, bytesWritten, buffers };
}

async function fstat(handle, options = { __proto__: null, bigint: false }) {
  validateObject(options, 'options');
  const { signal } = options;
  if (signal !== undefined) validateAbortSignal(signal, 'options.signal');
  checkAborted(signal);
  return fs.fstatSync(handle.fd, options);
}

async function ftruncate(handle, len = 0) {
  return fs.ftruncateSync(handle.fd, len);
}

async function fchmod(handle, mode) {
  return fs.fchmodSync(handle.fd, mode);
}

async function fchown(handle, uid, gid) {
  return fs.fchownSync(handle.fd, uid, gid);
}

async function fsync(handle) {
  return fs.fsyncSync(handle.fd);
}

async function fdatasync(handle) {
  return fs.fdatasyncSync(handle.fd);
}

async function futimes(handle, atime, mtime) {
  return fs.futimesSync(handle.fd, atime, mtime);
}

function getOptions(options, defaultOptions = {}) {
  if (options == null || typeof options === 'function') {
    return defaultOptions;
  }
  if (typeof options === 'string') {
    defaultOptions = { ...defaultOptions };
    defaultOptions.encoding = options;
    options = defaultOptions;
  }
  if (options.signal !== undefined) {
    validateAbortSignal(options.signal, 'options.signal');
  }
  return options;
}

// internal/errors `AbortError`.
class AbortError extends Error {
  constructor(message = 'The operation was aborted', options = undefined) {
    super(message, options);
    this.code = 'ABORT_ERR';
    this.name = 'AbortError';
  }
}

function validateAbortSignal(signal, name) {
  if (signal !== undefined &&
      (signal === null || typeof signal !== 'object' || !('aborted' in signal))) {
    nodeError('type', name, 'an instance of AbortSignal', signal);
  }
}

function checkAborted(signal) {
  if (signal?.aborted)
    throw new AbortError(undefined, { cause: signal.reason });
}

async function writeFile(handle, data, options) {
  options = getOptions(options, { encoding: 'utf8', mode: 0o666, flag: 'w' });
  checkAborted(options.signal);
  if (!isArrayBufferView(data)) {
    data = Buffer.from(data, options.encoding || 'utf8');
  }
  // A Buffer here has no shared `.buffer`, so it is written through directly
  // and advanced with `subarray` (a copy here, a window in node: the bytes
  // written are the same).
  if (!Buffer.isBuffer(data)) {
    data = new Uint8Array(data.buffer, data.byteOffset, data.byteLength);
  }
  let remaining = data.byteLength;
  if (remaining === 0) return;
  do {
    const { bytesWritten } =
      await write(handle, data, 0,
                  Math.min(kWriteFileMaxChunkSize, data.byteLength));
    remaining -= bytesWritten;
    data = data.subarray(bytesWritten);
  } while (remaining > 0);
}

async function readFile(filehandle, options) {
  options = getOptions(options, { flag: 'r' });
  checkAborted(options.signal);
  const encoding = options?.encoding;
  const decoder = encoding && new StringDecoder(encoding);

  let size = 0;
  let length = 0;
  const stats = fs.fstatSync(filehandle.fd);
  if (stats.isFile()) {
    size = stats.size;
    length = encoding ? Math.min(size, kReadFileBufferLength) : size;
  }
  if (length === 0) {
    length = kReadFileUnknownBufferLength;
  }

  let totalRead = 0;
  const noSize = size === 0;
  let buffer = Buffer.allocUnsafeSlow(length);
  let result = '';
  let offset = 0;
  let buffers;
  const chunkedRead = length > kReadFileBufferLength;

  while (true) {
    if (chunkedRead) {
      length = Math.min(size - totalRead, kReadFileBufferLength);
    }

    const bytesRead = fs.readSync(filehandle.fd, buffer, offset, length, null) ?? 0;
    totalRead += bytesRead;

    if (bytesRead === 0 ||
        totalRead === size ||
        (bytesRead !== buffer.length && !chunkedRead && !noSize)) {
      const singleRead = bytesRead === totalRead;

      const bytesToCheck = chunkedRead ? totalRead : bytesRead;

      if (bytesToCheck !== buffer.length) {
        buffer = buffer.subarray(0, bytesToCheck);
      }

      if (!encoding) {
        if (noSize && !singleRead) {
          buffers.push(buffer);
          return Buffer.concat(buffers, totalRead);
        }
        return buffer;
      }

      if (singleRead) {
        return buffer.toString(encoding);
      }
      result += decoder.end(buffer);
      return result;
    }
    const readBuffer = bytesRead !== buffer.length ?
      buffer.subarray(0, bytesRead) :
      buffer;
    if (encoding) {
      result += decoder.write(readBuffer);
    } else if (size !== 0) {
      offset = totalRead;
    } else {
      buffers ??= [];
      buffers.push(readBuffer);
      buffer = Buffer.allocUnsafeSlow(kReadFileUnknownBufferLength);
    }
  }
}

// `fsPromises.readFile(path, { signal })` and friends: the options check the
// path forms make before their (synchronous) work, as a function that
// returns the rejection value, or `undefined` to proceed.
function signalRefusal(options) {
  try {
    if (options !== null && typeof options === 'object') {
      validateAbortSignal(options.signal, 'options.signal');
      checkAborted(options.signal);
    }
  } catch (e) {
    return e;
  }
}

return { open, FileHandle, readFile, writeFile, signalRefusal };
})
