// An exception propagating out of a `finally` block is the SAME value that
// was thrown, even when the finalizer threw and caught one of its own. The
// pending value was dropped once the finalizer's own `catch` handled its
// throw, and an error synthesized from the message was rethrown instead:
// identity, `.code` and every other property were lost, and an awaited
// rejection's message came back as "Uncaught Error: …".
const fs = require("fs");
const o = { tag: 1 };
const clobber = () => {
  try {
    throw 2;
  } catch {}
};

function plain() {
  try {
    throw o;
  } finally {
    clobber();
  }
}
try { plain(); } catch (e) { console.log("sync     ", e === o); }

function builtin() {
  try {
    JSON.parse("{");
  } finally {
    clobber();
  }
}
try { builtin(); } catch (e) { console.log("builtin  ", e.name, e instanceof SyntaxError); }

function nested() {
  try {
    throw o;
  } finally {
    [1].forEach(() => clobber());
  }
}
try { nested(); } catch (e) { console.log("callback ", e === o); }

function viaCatch() {
  try {
    throw o;
  } catch (e) {
    try {
      throw e;
    } finally {
      clobber();
    }
  }
}
try { viaCatch(); } catch (e) { console.log("rethrown ", e === o); }

function returns() {
  try {
    return "r";
  } finally {
    clobber();
  }
}
console.log("return   ", returns());

function* gen() {
  try {
    yield 1;
  } finally {
    clobber();
  }
}
const it = gen();
it.next();
try {
  it.throw(Object.assign(new Error("gm"), { code: "G" }));
} catch (e) {
  console.log("generator", e.code, e.message);
}

(async () => {
  try {
    await (async () => fs.fstatSync(-1))();
  } finally {
    clobber();
  }
})().catch((e) => console.log("await    ", e.name, e.code));

(async () => {
  try {
    await null;
    throw Object.assign(new Error("hm"), { code: "H" });
  } finally {
    clobber();
  }
})().catch((e) => console.log("async    ", e.code, e.message));
