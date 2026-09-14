// The state machine and argument checks around `crypto`'s hash/cipher objects.
// Every method here accepted whatever it was handed: a number was stringified
// and hashed as its text, a finalized digest kept taking input, and `digest`
// and `final` could be run twice for a second valid-looking answer over input
// that was already consumed. None of that fails loudly — it produces a wrong
// key or a wrong digest that looks exactly like a right one.
const c = require("crypto");

const t = (l, f) => {
  try {
    console.log(l, JSON.stringify(f()));
  } catch (e) {
    console.log(l, "THROW", e.constructor.name, JSON.stringify([e.code, e.message]));
  }
};

// ── Hash / Hmac ──────────────────────────────────────────────────────────────
// `update` takes a string or a byte VIEW and REJECTS anything else. Hashing the
// text "5" for `update(5)` is the failure with no symptom.
t("update-num  ", () => c.createHash("md5").update(5).digest("hex"));
t("update-null ", () => c.createHash("md5").update(null).digest("hex"));
t("update-none ", () => c.createHash("md5").update().digest("hex"));
t("update-view ", () => c.createHash("md5").update(new Uint8Array([97])).digest("hex"));
t("update-str  ", () => c.createHash("md5").update("a").digest("hex"));
t("update-hex  ", () => c.createHash("md5").update("61", "hex").digest("hex"));

// `digest` finalizes. A Hash throws on a second call and on later input; an
// Hmac answers a second `digest` with an EMPTY buffer instead of throwing —
// the two are not the same object and do not behave the same way.
t("digest-twice", () => {
  const h = c.createHash("md5");
  h.update("a");
  h.digest("hex");
  return h.digest("hex");
});
t("update-after", () => {
  const h = c.createHash("md5");
  h.digest();
  return h.update("a");
});
t("hmac-twice  ", () => {
  const h = c.createHmac("sha256", "k");
  h.update("abc");
  const first = h.digest("hex");
  const second = h.digest();
  return [first.length, Buffer.isBuffer(second), second.length, h.digest("hex")];
});
t("hmac-after  ", () => {
  const h = c.createHmac("md5", "k");
  h.digest();
  h.update("a");
  return "accepted";
});

// `copy` forks the running state, so a common prefix is hashed once and the two
// copies go on to different tails. Without it the prefix has to be replayed.
t("copy        ", () => {
  const h = c.createHash("md5");
  h.update("a");
  const forked = h.copy();
  forked.update("b");
  return [forked.digest("hex"), h.digest("hex")];
});
t("copy-empty  ", () => c.createHash("md5").copy().digest("hex"));
t("copy-after  ", () => {
  const h = c.createHash("md5");
  h.digest();
  return h.copy();
});
// An Hmac has no `copy` at all.
t("hmac-copy   ", () => typeof c.createHmac("md5", "k").copy);
t("members     ", () => [
  Object.getOwnPropertyNames(Object.getPrototypeOf(c.createHash("md5"))).includes("copy"),
  Object.getOwnPropertyNames(Object.getPrototypeOf(c.createHmac("md5", "k"))).includes("copy"),
]);

// ── Cipher / Decipher ────────────────────────────────────────────────────────
const key = Buffer.alloc(32, 1);
const iv = Buffer.alloc(16, 2);
t("encrypt     ", () => {
  const e = c.createCipheriv("aes-256-cbc", key, iv);
  return e.update("hello", "utf8", "hex") + e.final("hex");
});
t("roundtrip   ", () => {
  const e = c.createCipheriv("aes-256-cbc", key, iv);
  const ct = Buffer.concat([e.update(Buffer.from("hello")), e.final()]);
  const d = c.createDecipheriv("aes-256-cbc", key, iv);
  return Buffer.concat([d.update(ct), d.final()]).toString();
});
// A byte view works here too — only a Buffer used to be recognised.
t("cipher-view ", () => {
  const e = c.createCipheriv("aes-256-cbc", key, iv);
  return Buffer.concat([e.update(new Uint8Array([104, 105])), e.final()]).toString("hex");
});
t("cipher-num  ", () => c.createCipheriv("aes-256-cbc", key, iv).update(5));
t("final-twice ", () => {
  const e = c.createCipheriv("aes-256-cbc", key, iv);
  e.final();
  return e.final("hex");
});
t("cipher-after", () => {
  const e = c.createCipheriv("aes-256-cbc", key, iv);
  e.final();
  return e.update("x", "utf8", "hex");
});
// The constructor rejections carry node's own class and code — a wrong key
// length is a RangeError, a wrong IV a TypeError, and the unknown-cipher
// message names no cipher.
t("bad-keylen  ", () => c.createCipheriv("aes-256-cbc", Buffer.alloc(8), iv));
t("bad-iv      ", () => c.createCipheriv("aes-256-cbc", key, Buffer.alloc(3)));
t("bad-cipher  ", () => c.createCipheriv("not-a-cipher", key, iv));
t("bad-decrypt ", () => {
  const d = c.createDecipheriv("aes-256-cbc", key, iv);
  d.update(Buffer.alloc(16, 9));
  return d.final("hex");
});

// ── Key derivation ───────────────────────────────────────────────────────────
// Deriving a key from the TEXT of a number is the worst of these: the result is
// a plausible key for input the caller never supplied.
t("pbkdf2      ", () => c.pbkdf2Sync("p", "s", 10, 16, "sha256").toString("hex"));
t("pbkdf2-num  ", () => c.pbkdf2Sync(5, "s", 10, 16, "sha256"));
t("pbkdf2-salt ", () => c.pbkdf2Sync("p", 5, 10, 16, "sha256"));
t("scrypt-num  ", () => c.scryptSync(5, "s", 16));
t("hkdf-ikm    ", () => c.hkdfSync("sha256", 5, "s", "i", 16));
t("hkdf-salt   ", () => c.hkdfSync("sha256", "k", 5, "i", 16));
t("hkdf-key-obj", () => Buffer.from(c.hkdfSync("sha256", c.createSecretKey(Buffer.from("abc")), "s", "i", 8)).toString("hex"));
// Arguments are validated SYNCHRONOUSLY, so the CALLBACK form throws out of the
// call rather than reporting through the callback.
t("pbkdf2-cb   ", () => c.pbkdf2("p", "s", 1, 8, "nope", () => console.log("  unreachable")));
t("scrypt-cb   ", () => c.scrypt("p", "s", 8, { N: 3 }, () => console.log("  unreachable")));
t("hkdf-sync   ", () => c.hkdfSync("nope", "k", "s", "i", 8));
t("scrypt-badN ", () => c.scryptSync("p", "s", 8, { N: 3 }));
// The good async path still delivers through the callback.
c.pbkdf2("p", "s", 10, 16, "sha256", (e, k) =>
  console.log("pbkdf2-async", JSON.stringify([e, k.toString("hex")])));
