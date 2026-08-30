// Node surfaces that were missing entirely. Everything printed here is
// platform-INVARIANT: the numeric constants come from this platform's libc, and
// `EADDRINUSE` is 48 on macOS but 98 on Linux, so only the POSIX-universal
// values are shown by number and everything else by type.

// os.constants had a hardcoded macOS signal table and no errno at all, so
// `SIGUSR1` was 30 on Linux (where it is 10) and `os.constants.errno` threw.
const os = require("os");
console.log("signals ", os.constants.signals.SIGKILL, os.constants.signals.SIGINT,
  os.constants.signals.SIGTERM, os.constants.signals.SIGABRT, os.constants.signals.SIGQUIT);
console.log("errno   ", os.constants.errno.EPERM, os.constants.errno.ENOENT,
  os.constants.errno.EINTR, os.constants.errno.EBADF, os.constants.errno.EACCES);
console.log("os-shape", typeof os.constants.dlopen.RTLD_LAZY, typeof os.constants.priority.PRIORITY_LOW,
  typeof os.constants.errno.EADDRINUSE, typeof os.constants.signals.SIGUSR1);

// fs.constants and crypto.constants were absent, so `fs.constants.O_RDONLY` and
// the RSA padding a library passes straight to `crypto` read as undefined.
const fsc = require("fs").constants;
console.log("fs      ", fsc.O_RDONLY, fsc.F_OK, fsc.R_OK, fsc.W_OK, fsc.X_OK, fsc.COPYFILE_EXCL);
console.log("fs-shape", typeof fsc.O_CREAT, typeof fsc.S_IFREG, typeof fsc.S_IRUSR, typeof fsc.O_APPEND);
const cc = require("crypto").constants;
console.log("crypto  ", cc.RSA_PKCS1_PADDING, cc.RSA_NO_PADDING, cc.RSA_PKCS1_OAEP_PADDING,
  cc.RSA_PKCS1_PSS_PADDING, cc.POINT_CONVERSION_COMPRESSED, cc.TLS1_3_VERSION);

// The legacy flat `require('constants')` is the union of all of them.
const flat = require("constants");
console.log("flat    ", flat.SIGKILL, flat.ENOENT, flat.F_OK, flat.RSA_PKCS1_PADDING,
  typeof flat.RTLD_LAZY, typeof flat.O_RDONLY, typeof require("node:constants").EPERM);
console.log("flat-t  ", typeof flat, Object.keys(flat).length > 100);

// DOMException, the class AbortSignal.reason carries. Its `name` is the SECOND
// constructor argument, not the class name, and `code` follows from that name.
const d = new DOMException("m", "AbortError");
console.log("domexc  ", d.name, d.message, d.code, d instanceof Error, d instanceof DOMException);
console.log("dom-str ", d.constructor.name, String(d), Object.prototype.toString.call(d));
console.log("dom-dflt", new DOMException("m").name, JSON.stringify(new DOMException().message),
  new DOMException("m").code);
console.log("dom-code", new DOMException("x", "IndexSizeError").code, new DOMException("x", "NotFoundError").code,
  new DOMException("x", "DataCloneError").code, new DOMException("x", "Unknown").code);
console.log("dom-stat", DOMException.ABORT_ERR, DOMException.INDEX_SIZE_ERR, DOMException.DATA_CLONE_ERR,
  DOMException.NOT_FOUND_ERR);
// Only `stack` is an own property; name/message/code read through the class.
console.log("dom-own ", Object.getOwnPropertyNames(d).sort().join(","), d.stack.split("\n")[0]);
console.log("dom-insp", require("util").inspect(d).split("\n")[0]);
// An aborted controller's reason is one of these, not a bare Error.
const ac = new AbortController();
ac.abort();
console.log("abort   ", ac.signal.reason.name, ac.signal.reason instanceof DOMException, ac.signal.aborted);

// process capability surfaces. The VALUES describe this runtime rather than
// node's own build, so only their shape is asserted.
const pf = process.features;
console.log("features", typeof pf, typeof pf.typescript, typeof pf.inspector, typeof pf.tls,
  typeof pf.uv, typeof pf.quic, typeof pf.cached_builtins);
console.log("config  ", typeof process.config, typeof process.config.variables,
  typeof process.config.target_defaults);
const flags = process.allowedNodeEnvironmentFlags;
console.log("flags   ", typeof flags.has, typeof flags.size, flags instanceof Set,
  flags.has("--definitely-not-a-real-flag"));

// readline/promises, whose Interface resolves `question` instead of taking a
// callback. The module could not be required at all.
const rlp = require("readline/promises");
console.log("readline", typeof rlp.createInterface, typeof require("readline").promises,
  typeof require("node:readline/promises").createInterface, typeof rlp);
const iface = rlp.createInterface({ input: process.stdin, output: process.stdout });
console.log("iface   ", typeof iface.question, typeof iface.close, typeof iface.write);
iface.close();

// Hashing BINARY input. Every `update` argument was stringified first, so
// `hash.update(new TextEncoder().encode(s))` hashed the text
// `"[object Object]"`, and a Buffer only appeared to work because stringifying
// it happened to yield its utf8 text — wrong the moment the bytes are not
// valid utf8, which is exactly when hashing bytes matters.
const crypto = require("crypto");
const utf8 = new TextEncoder().encode("abc");
console.log("hash-ta ", crypto.createHash("sha256").update(utf8).digest("hex"));
console.log("hash-str", crypto.createHash("sha256").update("abc").digest("hex"));
console.log("hash-buf", crypto.createHash("sha256").update(Buffer.from("abc")).digest("hex"));
// Bytes that are not valid utf8 — the case stringifying silently corrupted.
const rawBytes = Buffer.from([0xff, 0xfe, 0x00, 0x41]);
console.log("hash-bin", crypto.createHash("sha256").update(rawBytes).digest("hex"));
console.log("hmac-bin", crypto.createHmac("sha256", "k").update(rawBytes).digest("hex"));
console.log("hmac-key", crypto.createHmac("sha256", Buffer.from([1, 2, 3])).update("abc").digest("hex"));
// Mixed chunks accumulate in order regardless of each one's form.
console.log("hash-mix", crypto.createHash("sha256").update("a").update(new Uint8Array([98]))
  .update(Buffer.from("c")).digest("hex"));

// The WHATWG crypto surface. `globalThis.crypto` IS `crypto.webcrypto`, which
// is a different object from the node-flavoured module — it carries
// `randomUUID` but no `createHash`.
const wc = require("crypto").webcrypto;
console.log("webc    ", typeof wc, globalThis.crypto === wc, typeof wc.randomUUID,
  typeof wc.getRandomValues, typeof wc.createHash);
console.log("random  ", wc.randomUUID().length, wc.getRandomValues(new Uint8Array(4)).length,
  wc.randomUUID() !== wc.randomUUID());
// Only `digest` of SubtleCrypto is implemented — the key-based half
// (`encrypt`, `generateKey`, `importKey`, …) is not, so it is not pinned here.
console.log("subtle  ", typeof wc.subtle, typeof wc.subtle.digest);
(async () => {
  const dg = await wc.subtle.digest("SHA-256", utf8);
  console.log("digest  ", dg.constructor.name, dg.byteLength, Buffer.from(dg).toString("hex"));
  const sha1 = await wc.subtle.digest({ name: "SHA-1" }, new Uint8Array([1, 2, 3]));
  console.log("digest1 ", sha1.byteLength, Buffer.from(sha1).toString("hex"));
  try {
    await wc.subtle.digest("BOGUS", new Uint8Array(1));
  } catch (e) {
    console.log("unknown ", e.name, e.message, e instanceof DOMException);
  }
})();
