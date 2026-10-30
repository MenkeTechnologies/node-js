// `Blob.prototype.stream()`: node's `createBlobReaderStream`
// (lib/internal/blob.js, v26.12.1), ported as written. The reader is the
// in-memory blob reader: each `pull` hands back the next part's bytes
// (`STATUS_CONTINUE`, 1), then end of stream (`STATUS_EOS`, 0); it never
// blocks, so the wakeup callback is never invoked.
(function (parts) {
  let next = 0;
  const reader = {
    pull(callback) {
      if (next < parts.length) callback(1, parts[next++].buffer);
      else callback(0);
    },
    setWakeup() {},
  };
  return new ReadableStream({
    type: 'bytes',
    start(c) {
      // There really should only be one read at a time so using an
      // array here is purely defensive.
      this.pendingPulls = [];
      // Lazily register a wakeup callback that the C++ side can invoke
      // when new data is available after a STATUS_BLOCK.
      let immediate;
      this.wakeup = () => {
        if (this.pendingPulls.length > 0) {
          immediate ??= setImmediate(() => {
            immediate = undefined;
            this.readNext(c);
          });
        }
      };
    },
    pull(c) {
      const { promise, resolve, reject } = Promise.withResolvers();
      if (this.pendingPulls.length === 0) {
        reader.setWakeup(this.wakeup);
      }
      this.pendingPulls.push({ resolve, reject });
      this.readNext(c);
      return promise;
    },
    clearWakeupIfIdle() {
      if (this.pendingPulls.length === 0) {
        reader.setWakeup(undefined);
      }
    },
    readNext(c) {
      reader.pull((status, buffer) => {
        // If pendingPulls is empty here, the stream had to have
        // been canceled, and we don't really care about the result.
        if (this.pendingPulls.length === 0) {
          reader.setWakeup(undefined);
          return;
        }
        if (status === 0) {
          // EOS
          reader.setWakeup(undefined);
          c.close();
          // This is to signal the end for byob readers
          // see https://streams.spec.whatwg.org/#example-rbs-pull
          c.byobRequest?.respond(0);
          const pending = this.pendingPulls.shift();
          pending.resolve();
          return;
        }
        // ReadableByteStreamController.enqueue errors if we submit a
        // 0-length buffer. We need to check for that here.
        if (buffer !== undefined && buffer.byteLength !== 0) {
          c.enqueue(new Uint8Array(buffer));
        }
        // We keep reading until we either reach EOS, some error, or
        // we hit the flow rate of the stream (c.desiredSize).
        setImmediate(() => {
          if (c.desiredSize < 0) {
            // A manual backpressure check.
            if (this.pendingPulls.length !== 0) {
              const pending = this.pendingPulls.shift();
              pending.resolve();
              this.clearWakeupIfIdle();
            }
            return;
          }
          this.readNext(c);
        });
      });
    },
    cancel(reason) {
      reader.setWakeup(undefined);
      // Reject any currently pending pulls here.
      for (const pending of this.pendingPulls) {
        pending.reject(reason);
      }
      this.pendingPulls = [];
    },
  // We set the highWaterMark to 0 because we do not want the stream to
  // start reading immediately on creation. We want it to wait until read
  // is called.
  }, { highWaterMark: 0 });
});
