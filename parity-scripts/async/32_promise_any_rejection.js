// Promise.any's AggregateError: built inside the builtin with no JS frames
// (so its stack is the header alone and inspect brackets it), and an empty
// iterable rejects immediately.
Promise.any([]).then(
  (v) => console.log("fulfilled", v),
  (e) => console.log("empty", e.message, e.errors, JSON.stringify(e.stack)),
);
Promise.any([Promise.reject(1), Promise.reject({ two: 2 })]).catch((e) => {
  console.log(JSON.stringify(e.stack));
  console.log(e.errors.length, e.errors[0], e.errors[1].two);
  console.log(Object.getOwnPropertyNames(e));
  console.log(e);
});
Promise.any([Promise.reject(1), Promise.resolve(2)]).then((v) => console.log("won", v));
