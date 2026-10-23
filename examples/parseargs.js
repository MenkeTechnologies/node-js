// `util.parseArgs` is a port of node's `lib/internal/util/parse_args`. It was a
// reimplementation that emitted no `tokens`, skipped every config validation,
// never raised the "argument is ambiguous" usage error, stored `__proto__`,
// and returned `values` with an ordinary prototype.
const { parseArgs } = require("util");

const show = (label, f) => {
  try {
    console.log(label, JSON.stringify(f()));
  } catch (e) {
    console.log(label, e.name, e.code, JSON.stringify(e.message));
  }
};

const opts = {
  foo: { type: "string", short: "f" },
  bar: { type: "string" },
  a: { type: "boolean", short: "a" },
  b: { type: "boolean", short: "b" },
  c: { type: "string", short: "c" },
};

// The token stream: index counts ARGS (a short group shares one index; a
// value taken from the next arg advances it), rawName is what was typed.
show("tokens  ", () => parseArgs({ args: ["-f", "x", "--bar=1", "pos", "--", "-z"], options: opts, allowPositionals: true, tokens: true }));
show("group   ", () => parseArgs({ args: ["-ab", "-acVAL", "-ba"], options: opts, tokens: true }));
show("lone    ", () => parseArgs({ args: ["-", "--=x", "--bar="], options: opts, strict: false, tokens: true }));
show("negate  ", () => parseArgs({ args: ["--no-a", "--a"], options: { a: { type: "boolean", multiple: true } }, allowNegative: true, tokens: true }));
show("defaults", () => parseArgs({ args: ["--m", "a", "--m=b"], options: { m: { type: "string", multiple: true, default: ["z"] }, d: { type: "boolean", default: true } } }));
show("proto   ", () => parseArgs({ args: ["--__proto__", "--toString"], strict: false }));
show("noargs  ", () => parseArgs({ strict: false }));
const r = parseArgs({ args: ["--a"], options: opts });
console.log("nullpro ", Object.getPrototypeOf(r.values), Object.keys(r).join(","));

// Strict-mode usage errors.
show("ambig   ", () => parseArgs({ args: ["--foo", "-x"], options: opts }));
show("ambigsh ", () => parseArgs({ args: ["-f", "-x"], options: opts }));
show("missing ", () => parseArgs({ args: ["-c"], options: opts }));
show("noarg   ", () => parseArgs({ args: ["--a=1"], options: opts }));
show("negarg  ", () => parseArgs({ args: ["--no-a=x"], options: opts, allowNegative: true }));
show("negstr  ", () => parseArgs({ args: ["--no-bar"], options: opts, allowNegative: true }));
show("unknown ", () => parseArgs({ args: ["--x"], allowPositionals: true }));
show("posit   ", () => parseArgs({ args: ["p"] }));
show("posnum  ", () => parseArgs({ args: [5] }));

// Config validation, in node's order and wording.
show("v.args  ", () => parseArgs({ args: "x" }));
show("v.strict", () => parseArgs({ args: [], strict: 1 }));
show("v.tokens", () => parseArgs({ args: [], tokens: "y" }));
show("v.allowp", () => parseArgs({ args: [], allowPositionals: {} }));
show("v.opts  ", () => parseArgs({ args: [], options: [] }));
show("v.opt   ", () => parseArgs({ args: [], options: { a: null } }));
show("v.type  ", () => parseArgs({ args: [], options: { a: {} } }));
show("v.short ", () => parseArgs({ args: [], options: { a: { type: "string", short: 5 } } }));
show("v.shortl", () => parseArgs({ args: [], options: { a: { type: "string", short: "ab" } } }));
show("v.multi ", () => parseArgs({ args: [], options: { a: { type: "string", multiple: "y" } } }));
show("v.def   ", () => parseArgs({ args: [], options: { a: { type: "boolean", default: "y" } } }));
show("v.defarr", () => parseArgs({ args: [], options: { a: { type: "boolean", multiple: true, default: true } } }));
show("v.defel ", () => parseArgs({ args: [], options: { a: { type: "string", multiple: true, default: ["x", 1] } } }));
show("v.inherit", () => parseArgs(Object.create({ args: ["--q"] })));
