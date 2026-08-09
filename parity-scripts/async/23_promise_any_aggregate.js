// Promise.any -> AggregateError, Error.cause, and rejection-shape details.
Promise.any([Promise.reject(new Error("e1")), Promise.reject(new Error("e2"))]).catch((e) => {
  console.log(e.constructor.name, e instanceof AggregateError, e instanceof Error);
  console.log(e.message, e.errors.length, e.errors.map((x) => x.message).join(","));
  console.log(JSON.stringify(Object.keys(e)), JSON.stringify(Object.getOwnPropertyNames(e).sort()));
  console.log(JSON.stringify(e));
});

Promise.any([Promise.reject(1), Promise.resolve("ok"), Promise.reject(2)]).then((v) => console.log("any-ok", v));

const wrapped = new Error("outer", { cause: new Error("inner") });
console.log(wrapped.cause.message, JSON.stringify(Object.keys(wrapped)));
console.log(JSON.stringify(Object.getOwnPropertyDescriptor(wrapped, "cause")));

const plainCause = new TypeError("t", { cause: { code: 42 } });
console.log(plainCause.cause.code, plainCause.name, String(plainCause));

Promise.allSettled([Promise.resolve(1), Promise.reject(new Error("no"))]).then((rs) => {
  console.log(JSON.stringify(rs.map((r) => (r.status === "fulfilled" ? r.value : r.reason.message))));
});
