// `Buffer.from` of an unsupported value throws ERR_INVALID_ARG_TYPE, a Node
// JS-layer error whose string form brackets the code.
for (const v of [5, null, undefined, true, Symbol("s")]) {
  try {
    Buffer.from(v);
  } catch (e) {
    console.log(String(e));
    console.log(e.code, e.name, Object.getOwnPropertyNames(e).join(","));
  }
}
