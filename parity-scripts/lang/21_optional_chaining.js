// Optional chaining and nullish coalescing.
const data = {
  user: {
    profile: {
      name: "Ada",
      address: { city: "London" },
    },
    settings: null,
  },
  items: [{ id: 1 }, { id: 2 }],
};

console.log(data.user?.profile?.name);
console.log(data.user?.profile?.address?.city);
console.log(data.user?.settings?.theme ?? "default-theme");
console.log(data.user?.missing?.deep?.value ?? "not found");

// Optional call.
const api = {
  getName: () => "callable",
};
console.log(api.getName?.());
console.log(api.getAge?.() ?? "no age method");

// Optional index access.
console.log(data.items?.[0]?.id);
console.log(data.items?.[99]?.id ?? "out of range");

// Nullish vs OR: 0 and "" are kept by ??.
const port = 0;
console.log("with ||:", port || 8080);
console.log("with ??:", port ?? 8080);

const label = "";
console.log("with ||:", label || "empty");
console.log("with ??:", label ?? "empty");

// Chained nullish.
const cfg = {};
const timeout = cfg.timeout ?? cfg.defaultTimeout ?? 3000;
console.log("timeout:", timeout);

// Short-circuit prevents deeper eval.
let sideEffects = 0;
const nested = null;
const val = nested?.[(sideEffects++, "key")];
console.log("val:", val, "sideEffects:", sideEffects);
