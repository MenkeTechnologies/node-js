// try/catch/finally, custom Error subclasses, throw.
class ValidationError extends Error {
  constructor(field, message) {
    super(message);
    this.name = "ValidationError";
    this.field = field;
  }
}

class NotFoundError extends Error {
  constructor(id) {
    super(`resource ${id} not found`);
    this.name = "NotFoundError";
    this.id = id;
  }
}

function validate(user) {
  if (!user.name) throw new ValidationError("name", "name is required");
  if (user.age < 0) throw new ValidationError("age", "age must be positive");
  return true;
}

function lookup(id) {
  const db = { 1: "Alice", 2: "Bob" };
  if (!(id in db)) throw new NotFoundError(id);
  return db[id];
}

const cases = [{ name: "X", age: 5 }, { name: "", age: 1 }, { name: "Y", age: -3 }];
for (const c of cases) {
  try {
    validate(c);
    console.log("valid:", c.name);
  } catch (e) {
    console.log(`${e.name} on ${e.field}: ${e.message}`);
  }
}

let cleanup = 0;
for (const id of [1, 2, 99]) {
  try {
    console.log("found:", lookup(id));
  } catch (e) {
    console.log("error:", e.message, "isNotFound:", e instanceof NotFoundError);
  } finally {
    cleanup++;
  }
}
console.log("cleanup ran:", cleanup);

// Re-throw and nested catch.
function outer() {
  try {
    throw new Error("inner");
  } catch (e) {
    throw new Error("wrapped: " + e.message);
  }
}
try {
  outer();
} catch (e) {
  console.log(e.message);
}
