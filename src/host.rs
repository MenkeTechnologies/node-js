//! The JavaScript object heap and runtime, reached from fusevm through
//! registered builtins (`register_builtin`) and the strict numeric hook.
//!
//! node-js owns no VM and no JIT: the compiler lowers JS to `fusevm::Chunk`, and
//! every JS-specific operation the VM can't do natively is a builtin call that
//! lands here. Local variables live in `Rc<RefCell>` environments chained
//! parent-to-child, so a nested function/closure captures its enclosing scope by
//! reference.
//!
//! Value representation:
//!   - immediate: `Value::Float` (every JS number — one IEEE-754 f64 type),
//!     `Value::Bool` (true/false), `Value::Undef` (undefined);
//!   - heap `Value::Obj(u32)` handles: string, array, object, function,
//!     builtin-namespace, and the canonical `null` — the reference types.

use fusevm::{Chunk, NumOp, VMResult, Value, VM};
use indexmap::IndexMap;
use std::cell::RefCell;
use std::collections::HashMap;
use std::collections::HashSet;
use std::rc::Rc;
use std::sync::mpsc::{Receiver, Sender};
use std::time::{Duration, Instant};

/// A unit of I/O work handed from a background I/O thread to the main-thread
/// event loop. It is a boxed closure so `host.rs` stays agnostic of `net`/`http`:
/// the I/O thread captures only plain `Send` data (bytes, ids, `TcpStream`s) and
/// the closure runs the JS-touching dispatch on the main thread (where the
/// thread-local host lives). I/O threads NEVER touch the host directly.
pub type IoTask = Box<dyn FnOnce() -> Result<(), String> + Send>;

/// Builtin ids emitted by the compiler and registered on every VM. The compiler
/// (`compiler.rs`) and the handler table (`builtins.rs::install`) must agree on
/// these exactly.
pub mod ops {
    pub const GETLOCAL: u16 = 1; // [name] -> value (scope-chain read)
    pub const SETLOCAL: u16 = 2; // [name, value] -> value (assignment)
    pub const DECLARE: u16 = 3; // [name, value] -> value (let/const/var into current scope)
    pub const DELNAME: u16 = 4; // [name]
    pub const GETATTR: u16 = 5; // [recv, name] -> value (member .x)
    pub const SETATTR: u16 = 6; // [recv, name, value]
    pub const GETITEM: u16 = 7; // [recv, idx] -> value (computed [k])
    pub const SETITEM: u16 = 8; // [recv, idx, value]
    pub const DELITEM: u16 = 9; // [recv, idx] -> Bool (delete obj[k])
    pub const MKSTR: u16 = 10; // [parts...] -> str (concat)
    pub const MKARR: u16 = 11; // [items...] -> array
    pub const MKOBJ: u16 = 12; // [tag,k,v,...] -> object (tag 1 = ...spread of k)
    pub const CALL: u16 = 13; // [name, args...] -> resolve name & call
    pub const CALL_METHOD: u16 = 14; // [recv, name, args...]
    pub const CALL_VALUE: u16 = 15; // [callable, args...]
    pub const NEW: u16 = 16; // [ctor, args...] -> instance
    pub const TRUTHY: u16 = 17; // [v] -> Bool (JS truthiness)
    pub const TOSTR: u16 = 18; // [v] -> str via String(v)
    pub const MKFUNC: u16 = 19; // [func_id, defaults...] -> closure
    pub const GETITER: u16 = 20; // [iterable] -> iterator (left on stack)
    pub const FORITER: u16 = 21; // peek iterator -> pushes value + Bool(has_next)
    pub const FORIN_KEYS: u16 = 22; // [obj] -> array of enumerable keys
    pub const CONTAINS: u16 = 23; // [key, obj] -> Bool (`in`)
    pub const SIG_RETURN: u16 = 24; // [v] -> return v from the function
    pub const BINOP: u16 = 25; // [tag, a, b] -> bitwise/shift result (JS int32 semantics)
    pub const UNARY: u16 = 26; // [tag, v] -> unary +/~ result
    pub const STRICT_EQ: u16 = 27; // [a, b] -> Bool (===)
    pub const LOOSE_EQ: u16 = 28; // [a, b] -> Bool (==)
    pub const TYPEOF: u16 = 29; // [v] -> str
    pub const LOAD_NULL: u16 = 30; // [] -> the canonical null
    pub const THROW: u16 = 31; // [v] -> throw
    pub const TRY: u16 = 32; // [try_id] -> run a try/catch/finally block
    pub const NULLISH: u16 = 33; // [v] -> Bool (v is null or undefined)
    pub const UNPACK: u16 = 34; // [iterable, count, star] -> pushes count values
    pub const BUILD_ARGS: u16 = 35; // [tag,val,...] -> flat array (tag 1 = ...spread)
    pub const THIS: u16 = 36; // [] -> current `this`
    pub const INSTANCEOF: u16 = 37; // [a, b] -> Bool
    pub const DELPROP_NAME: u16 = 38; // [recv, name] -> Bool (delete obj.name)
    pub const APPLY: u16 = 39; // [callable, argsArray] -> call with spread args
    pub const APPLY_METHOD: u16 = 40; // [recv, name, argsArray] -> method call with spread
    pub const OBJ_REST: u16 = 41; // [obj, excludedKeys] -> object of remaining keys
    pub const DIV: u16 = 42; // [a, b] -> IEEE `a / b` (JS: x/0 = ±Infinity, 0/0 = NaN)
    pub const MKCLASS: u16 = 43; // [parent_or_undef, ctor_fn] -> class constructor value
    pub const DEF_MEMBER: u16 = 44; // [class, name, kind, is_static, fn] -> define method/get/set
    pub const SUPER_CALL: u16 = 45; // [args...] -> invoke parent ctor on `this`, then init fields
    pub const SUPER_GET: u16 = 46; // [name] -> resolve `super.name` (method up the parent chain)
    pub const YIELD: u16 = 47; // [v] -> suspend the running generator, yield v
    pub const PROPKEY: u16 = 48; // [v] -> property-key string (Symbol -> internal key, else String())
    pub const NEW_TARGET: u16 = 49; // [] -> the current frame's new.target (undefined if not `new`)
    pub const DEF_FIELD: u16 = 50; // [class, name, thunk] -> register an instance field initializer
    pub const AWAIT: u16 = 51; // [v] -> await v (suspend the async coroutine until v settles)
    pub const DEF_ACCESSOR: u16 = 52; // [obj, name, kind, fn] -> install a getter/setter on obj
    pub const DBG_LINE: u16 = 53; // [line] -> DAP statement marker (debug only)
    pub const MKBIGINT: u16 = 54; // [decimal_str] -> heap BigInt value
    pub const MKREGEX: u16 = 55; // [pattern, flags] -> heap RegExp value
    pub const TAG_TMPL: u16 = 56; // [tag, cooked..., raw..., n, values...] -> tagged-template call
    pub const GET_ASYNC_ITER: u16 = 57; // [iterable] -> async iterator (for-await-of)
    pub const ASYNC_STEP: u16 = 58; // [asyncIterator] -> Promise of {value, done}
    pub const NUM_STEP: u16 = 59; // [tag(±1), old] -> pushes ToNumeric(old), returns old±1 (type-preserving; BigInt-aware ++/--)
    pub const ITER_CLOSE: u16 = 60; // [iterator] -> close it (for-of break: run a generator's finally / call .return())
    pub const TYPEOF_NAME: u16 = 61; // [name] -> str; `typeof <ident>` reads the name WITHOUT throwing (unbound -> "undefined")
    pub const SIG_BREAK: u16 = 62; // [label|""] -> raise a Break signal and halt the chunk (break out of a `try`)
    pub const SIG_CONTINUE: u16 = 63; // [label|""] -> raise a Continue signal and halt the chunk
    pub const SIG_UNWIND: u16 = 64; // [tag] -> 0 none / 1 break here / 2 continue here; halts the chunk to propagate
    pub const PUSH_SCOPE: u16 = 65; // [] -> enter a fresh block scope (`let`/`const` live here)
    pub const POP_SCOPE: u16 = 66; // [] -> leave the innermost block scope
    pub const COPY_SCOPE: u16 = 67; // [] -> replace the innermost block scope with a COPY (per-iteration `let`)
    pub const DECLARE_VAR: u16 = 68; // [name, value] -> declare at FUNCTION scope, ignoring block scopes (`var`)
    pub const NAMED_EVAL: u16 = 69; // [key, kind, fn] -> fn; SetFunctionName for a COMPUTED key (kind picks the `get `/`set ` prefix)
}

/// `SIG_UNWIND` scope tags: what the emitting site is nested in.
pub mod unwind {
    /// No enclosing loop in this chunk — any pending signal propagates outward.
    pub const NO_LOOP: &str = "";
    /// An enclosing UNLABELED loop in this chunk.
    pub const PLAIN_LOOP: &str = "\u{0}";
    /// `SIG_UNWIND` result codes.
    pub const NONE: i64 = 0;
    pub const BREAK: i64 = 1;
    pub const CONTINUE: i64 = 2;
}

/// `DEF_MEMBER` member-kind tags.
pub mod member {
    pub const METHOD: i64 = 0;
    pub const GET: i64 = 1;
    pub const SET: i64 = 2;
}

/// Bitwise/shift op tags carried by `ops::BINOP` (JS ToInt32/ToUint32 rules).
pub mod binop {
    pub const BITAND: i64 = 0;
    pub const BITOR: i64 = 1;
    pub const BITXOR: i64 = 2;
    pub const SHL: i64 = 3;
    pub const SHR: i64 = 4;
    pub const USHR: i64 = 5;
}

/// Unary op tags carried by `ops::UNARY`.
pub mod unop {
    pub const POS: i64 = 0; // unary +
    pub const BITNOT: i64 = 1; // ~
}

// ── heap objects ───────────────────────────────────────────────────────────

/// A compiled function template: parameter shape + body chunk. Shared by every
/// closure created from the same function/arrow.
#[derive(Clone, serde::Serialize, serde::Deserialize)]
pub struct FuncDef {
    pub name: String,
    /// Parameter binding templates (destructuring lowered by the compiler into
    /// the body prologue; here we only track the simple arg slots).
    pub params: Vec<ParamSlot>,
    pub chunk: Chunk,
    pub is_arrow: bool,
    /// True for a `function*`/`*method`/generator arrow: calling it builds a
    /// suspended generator instead of running the body.
    pub is_generator: bool,
    /// True for an `async` function/method/arrow: calling it drives a coroutine
    /// and returns a Promise; `await` inside suspends via the same yielder.
    pub is_async: bool,
    /// True for a MethodDefinition (`{ m(){} }`, a class method/accessor). A
    /// non-generator method is not a constructor, so it owns no `prototype`.
    #[serde(default)]
    pub is_method: bool,
    /// True for a NAMED function *expression* (`const f = function fact(n) {…}`):
    /// the closure gets an extra environment binding its own name to itself, so
    /// the body can recurse through that name even when the outer binding differs.
    #[serde(default)]
    pub self_name: bool,
}

/// One parameter slot. `name` is the simple bound name; a destructuring pattern
/// is lowered to a synthetic `.arg{i}` name plus body prologue code.
#[derive(Clone, serde::Serialize, serde::Deserialize)]
pub struct ParamSlot {
    pub name: String,
    /// True for the `...rest` collector.
    pub rest: bool,
    /// True if this slot has a default expression (applied in the body prologue).
    pub has_default: bool,
}

/// A compiled `try`/`catch`/`finally` block. Bodies are bare chunks run in the
/// current scope.
#[derive(Clone, serde::Serialize, serde::Deserialize)]
pub struct TryDef {
    pub block: Chunk,
    /// `(catch_param_name, catch_body)`.
    pub handler: Option<(Option<String>, Chunk)>,
    pub finalizer: Option<Chunk>,
}

/// A live closure value.
#[derive(Clone)]
pub struct FuncVal {
    pub def_id: usize,
    /// Captured lexical environment (enclosing scope chain), for free vars.
    pub env: Option<Env>,
    /// `this` captured at definition time (arrow functions).
    pub this: Option<Value>,
    pub is_arrow: bool,
    /// The owning class name for a method (drives `super` resolution). `None` for
    /// plain functions/arrows.
    pub home_class: Option<String>,
}

/// A heap object.
#[derive(Clone)]
pub enum JsObj {
    Str(String),
    Array(Vec<Value>),
    Object(IndexMap<String, Value>),
    Func(FuncVal),
    /// A first-class reference to a builtin function or namespace
    /// (`console.log`, `Math`, `parseInt`).
    Builtin(String),
    /// A bound method value (`obj.method` captured then called): dispatches
    /// through `call_method(recv, name, args)` when invoked.
    BoundMethod {
        recv: Value,
        name: String,
    },
    /// The single canonical `null`.
    Null,
    /// A live iterator over a sequence, with a cursor.
    Iter {
        items: Vec<Value>,
        idx: usize,
    },
    /// A bound function (`fn.bind(thisArg, ...preargs)`).
    BoundFunc {
        target: Value,
        this: Value,
        args: Vec<Value>,
    },
    /// A class constructor value: the runtime object produced by a `class`.
    Class(ClassVal),
    /// A `Symbol` — a unique property key. `registered` marks a `Symbol.for`
    /// key (shared) vs a fresh `Symbol()`.
    Symbol {
        desc: Option<String>,
        id: u64,
    },
    /// A `Map` (or `WeakMap` when `weak`): insertion-ordered key→value entries.
    Map {
        entries: IndexMap<MapKey, (Value, Value)>,
        weak: bool,
    },
    /// A `Set` (or `WeakSet` when `weak`): insertion-ordered unique values.
    Set {
        entries: IndexMap<MapKey, Value>,
        weak: bool,
    },
    /// A live generator, backed by a stackful `corosensei` coroutine in
    /// `JsHost.generators`.
    Generator {
        id: u32,
    },
    /// A Promise, backed by a `PromiseCell` in `JsHost.promises`.
    Promise {
        id: u32,
    },
    /// An arbitrary-precision `BigInt` (`typeof === "bigint"`).
    BigInt(num_bigint::BigInt),
    /// A compiled regular expression (`/pat/flags` or `new RegExp(...)`).
    RegExp(Box<RegExpObj>),
}

/// Which variant a heap object is, carrying none of its contents.
///
/// Property access has to pick a branch by variant, but the code inside a branch
/// re-enters the host (`bound_method`, `lookup_chain`, `invoke`), so it cannot
/// hold a `&JsObj` borrow across the match. The way out used to be
/// `h.get(v).cloned()` — which deep-copies the entire backing store (a whole
/// `Vec<Value>`, `IndexMap`, or `String`) just to read its tag. That made one
/// property read O(len) and any loop over a collection O(n^2). This type is the
/// same discriminant with nothing attached, so the probe is O(1) and each branch
/// re-borrows for only the one field it actually needs.
/// The well-known symbols node-js actually honors. `Symbol.<name>` is the
/// interned symbol `@@Symbol.<name>`, and using it as a property key stores
/// under the sentinel string `@@<name>` (`property_key`) so the internal
/// lookups (`@@iterator`, `@@toPrimitive`, …) can find it without a symbol
/// table walk. Symbols V8 defines but node-js does not act on are deliberately
/// absent: `Symbol.hasInstance` reading as a symbol while `instanceof` ignored
/// it would be a silent fake.
pub const WELL_KNOWN_SYMBOLS: &[&str] =
    &["iterator", "asyncIterator", "toPrimitive", "toStringTag"];

/// Whether the internal key `k` came from a SYMBOL used as a property key
/// (`@@sym:<id>`, or a well-known `@@iterator`), as opposed to one of node-js's
/// hidden slots (`@@native`, `@@bytes`, `@@ms`, `@@kind`, …). Only the former
/// is an observable JavaScript property.
pub fn is_symbol_key(k: &str) -> bool {
    match k.strip_prefix("@@") {
        Some(rest) => rest
            .strip_prefix("sym:")
            .map(|i| i.parse::<u64>().is_ok())
            .unwrap_or_else(|| WELL_KNOWN_SYMBOLS.contains(&rest)),
        None => false,
    }
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum ObjKind {
    Str,
    Array,
    Object,
    Func,
    Builtin,
    BoundMethod,
    Null,
    Iter,
    BoundFunc,
    Class,
    Symbol,
    Map,
    Set,
    Generator,
    Promise,
    BigInt,
    RegExp,
}

impl JsObj {
    /// This object's variant, without touching its contents.
    pub fn kind(&self) -> ObjKind {
        match self {
            JsObj::Str(_) => ObjKind::Str,
            JsObj::Array(_) => ObjKind::Array,
            JsObj::Object(_) => ObjKind::Object,
            JsObj::Func(_) => ObjKind::Func,
            JsObj::Builtin(_) => ObjKind::Builtin,
            JsObj::BoundMethod { .. } => ObjKind::BoundMethod,
            JsObj::Null => ObjKind::Null,
            JsObj::Iter { .. } => ObjKind::Iter,
            JsObj::BoundFunc { .. } => ObjKind::BoundFunc,
            JsObj::Class(_) => ObjKind::Class,
            JsObj::Symbol { .. } => ObjKind::Symbol,
            JsObj::Map { .. } => ObjKind::Map,
            JsObj::Set { .. } => ObjKind::Set,
            JsObj::Generator { .. } => ObjKind::Generator,
            JsObj::Promise { .. } => ObjKind::Promise,
            JsObj::BigInt(_) => ObjKind::BigInt,
            JsObj::RegExp(_) => ObjKind::RegExp,
        }
    }
}

/// A `RegExp` object: the compiled `fancy_regex::Regex` plus the JS-visible
/// source, flag booleans, and the mutable `lastIndex` cursor (used by `g`/`y`
/// matching). fancy-regex adds lookaround + backreferences on top of the Rust
/// `regex` fast path, so the JS grammar node-js can accept is a near-superset.
#[derive(Clone)]
pub struct RegExpObj {
    /// The translated regex. Construction of a pattern fancy-regex still cannot
    /// express (documented in BUGS.md) throws at `RegExp` build time, so a live
    /// `RegExpObj` always holds a compiled engine.
    pub re: fancy_regex::Regex,
    pub source: String,
    pub flags: String,
    pub global: bool,
    pub ignore_case: bool,
    pub multiline: bool,
    pub dot_all: bool,
    pub sticky: bool,
    pub unicode: bool,
    /// `lastIndex`, in UTF-16 code units; advanced by `exec`/`test` under the
    /// `g`/`y` flags. The newtype keeps it from being confused with the regex
    /// engine's byte offsets, which are the same shape and differ off the BMP.
    pub last_index: crate::utf16::U16Index,
}

/// A Promise's settled state and pending reactions.
pub struct PromiseCell {
    pub state: PromiseState,
    pub value: Value,
    /// Reactions registered while still pending; drained (as microtasks) on
    /// settle.
    pub reactions: Vec<PromiseReaction>,
    /// True once a rejection has been observed by a handler (`.then`/`.catch`),
    /// so the loop doesn't report it as unhandled.
    pub handled: bool,
}

/// A pending Promise reaction: a user `.then` (JS handlers + a result promise) or
/// a native continuation (Promise chaining / async `await` resumption).
pub enum PromiseReaction {
    Js {
        on_ful: Value,
        on_rej: Value,
        result: Value,
    },
    Native(Box<dyn FnOnce(PromiseState, Value) -> Result<(), String>>),
}

#[derive(Default, Clone, Copy, PartialEq, Eq)]
pub enum PromiseState {
    #[default]
    Pending,
    Fulfilled,
    Rejected,
}

/// A live class constructor. The prototype object (holding instance methods) and
/// the static-side own properties live on the heap; `parent` is the superclass
/// constructor value (`None` for a base class).
#[derive(Clone)]
pub struct ClassVal {
    pub name: String,
    /// The constructor function value (a `JsObj::Func`), or `None` for a class
    /// with only a synthesized default constructor.
    pub ctor: Option<Value>,
    pub parent: Option<Value>,
    /// `C.prototype` — the object instances delegate to.
    pub proto: Value,
    /// Static own properties (static methods/fields), plus `name`/`prototype`.
    pub statics: IndexMap<String, Value>,
    /// Instance field initializers: `(name, thunk_fn, name_anon_init)`, run
    /// per-instance after `super()` (or at construction start for a base class).
    /// `name_anon_init` records the SYNTACTIC fact that the initializer was an
    /// anonymous function definition, so 15.7.10 NamedEvaluation applies to its
    /// result — it cannot be re-derived at run time (a field initialised from an
    /// already-anonymous function held elsewhere must not be renamed).
    pub fields: Vec<(String, Value, bool)>,
}

/// The result of resolving `super.name`: a getter to invoke (accessor property)
/// or a directly-usable value (method / data property).
pub enum SuperRef {
    Getter(Value),
    Data(Value),
}

/// A `Map`/`Set` key under SameValueZero: `NaN` collapses to one key, `-0` and
/// `+0` are the same key, primitives compare by value, objects by heap identity.
#[derive(Clone, PartialEq, Eq, Hash)]
pub enum MapKey {
    Undef,
    Null,
    Bool(bool),
    /// f64 bit pattern with `NaN` canonicalized and `-0` normalized to `+0`.
    Num(u64),
    /// A `BigInt` key, by its decimal string (SameValueZero: `1n` is one key).
    Big(String),
    Str(String),
    /// Heap identity (objects, arrays, functions, symbols).
    Ref(u32),
}

// ── environments ─────────────────────────────────────────────────────────────

/// A local-variable environment, shared (by `Rc`) between a frame and any nested
/// function that captures it.
pub struct EnvData {
    pub vars: IndexMap<String, Value>,
    pub parent: Option<Env>,
}
pub type Env = Rc<RefCell<EnvData>>;

/// An accessor property: `(getter, setter)`, either optional.
pub type Accessor = (Option<Value>, Option<Value>);

/// Prefix of the hidden property-map entry that reserves an accessor's slot in
/// own-key insertion order (see `set_accessor`).
pub const ORD_MARKER: &str = "@@ord:";

/// The three ECMAScript own-property attributes. `PropAttrs::default()` is the
/// all-true shape a plain `o.k = v` assignment produces, which is why only
/// deviations need storing.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct PropAttrs {
    pub writable: bool,
    pub enumerable: bool,
    pub configurable: bool,
}

impl Default for PropAttrs {
    fn default() -> Self {
        PropAttrs {
            writable: true,
            enumerable: true,
            configurable: true,
        }
    }
}

impl PropAttrs {
    /// The attribute shape V8 gives an internal-but-inspectable slot such as
    /// `Error.prototype.message`, `err.stack` or a `Buffer`'s view metadata:
    /// readable and replaceable, but never enumerated.
    pub const HIDDEN: PropAttrs = PropAttrs {
        writable: true,
        enumerable: false,
        configurable: true,
    };
}

fn new_env(parent: Option<Env>) -> Env {
    Rc::new(RefCell::new(EnvData {
        vars: IndexMap::new(),
        parent,
    }))
}

/// A fresh empty scope chained under `parent`.
pub fn child_env(parent: Env) -> Env {
    new_env(Some(parent))
}

/// One function activation.
pub struct Frame {
    pub env: Env,
    /// The env this activation started in — the FUNCTION scope. `var` and hoisted
    /// function declarations bind here no matter how many block scopes are open.
    pub base_env: Env,
    pub this_obj: Option<Value>,
    /// `new.target` for this activation (the constructor when invoked via `new`).
    pub new_target: Option<Value>,
    /// The class value owning the running method (drives `super`); `None` outside
    /// a class method/constructor.
    pub home_class: Option<Value>,
    /// Source line the frame is currently executing (updated by the DAP line hook
    /// under `--dap`; stays 0 on ordinary runs).
    pub line: u32,
    /// The function name that owns this frame, for the DAP `stackTrace`; `None`
    /// for the module frame and anonymous activations.
    pub owner: Option<String>,
    /// True ONLY for the program's module frame. A generator/async body runs on a
    /// coroutine whose swapped-in context holds just ITS OWN frame, so the frame
    /// COUNT cannot tell "module scope" from "coroutine body scope" — without this
    /// flag every top-level `let`/`var` in such a body declared a GLOBAL, shared
    /// across concurrent activations of the same function.
    pub is_module: bool,
}

/// A non-local control signal. `Break`/`Continue` carry the optional loop label
/// and are only raised when the target loop lives in an ENCLOSING chunk (a
/// `break` inside a `try` block, which the host runs as its own chunk); a
/// same-chunk `break` is a plain compiler-resolved jump.
#[derive(Clone)]
pub enum Signal {
    Return(Value),
    Break(Option<String>),
    Continue(Option<String>),
}

/// The JavaScript runtime.
pub struct JsHost {
    heap: Vec<JsObj>,
    /// Function templates, indexed by def id.
    pub funcs: Vec<FuncDef>,
    /// try/catch/finally block templates, indexed by try id.
    pub tries: Vec<TryDef>,
    /// Module-level (global) names.
    globals: IndexMap<String, Value>,
    /// The frame stack (bottom = module).
    frames: Vec<Frame>,
    /// The program's top-level scope — the scope runtime-compiled source runs in
    /// (`new Function`, indirect `eval`, `vm.runInThisContext`; see
    /// `run_chunk_in_global_scope`), as opposed to whatever function frame
    /// happens to be executing when that source is compiled.
    ///
    /// Held as its own field rather than read off `frames[0]` because a coroutine
    /// body runs with `frames` SWAPPED for its own one-frame context
    /// (`install_gen_ctx`), so the bottom frame is not the top-level frame there.
    ///
    /// Note this is node-js's ONE top-level scope. Node distinguishes the global
    /// scope from a CommonJS module's scope (a module body is a wrapper
    /// function), so in Node a file's top-level `var` is invisible to dynamic
    /// code; here the entry file is evaluated with Script semantics, so it stays
    /// visible. That is the same entry-file-is-a-Script divergence `BUGS.md`
    /// records for top-level `return`, not a separate one — and `node -e`, which
    /// really is a Script, matches Node exactly.
    global_env: Env,
    pub error: Option<String>,
    /// The in-flight thrown value, if any (JS `throw`).
    pub exc: Option<Value>,
    pub signal: Option<Signal>,
    /// Promises that settled REJECTED this tick. Drained at each microtask
    /// checkpoint: any still without a handler is an unhandled rejection.
    pub pending_rejections: Vec<u32>,
    /// `process.on(event, fn)` listeners, by event name. Only the events the
    /// runtime actually emits (`unhandledRejection`) can fire.
    pub process_listeners: IndexMap<String, Vec<Value>>,
    /// The canonical `null` handle (allocated once).
    null_val: Value,
    /// `[[Prototype]]` link per heap object, by heap index. Absent = default
    /// (`Object.prototype` for objects, `null` for the root).
    protos: HashMap<u32, Value>,
    /// Heap objects whose `[[Prototype]]` is *explicitly* null — via
    /// `Object.create(null)` or `Object.setPrototypeOf(o, null)`. Distinct from a
    /// bare `{}` (absent from `protos` but conceptually `Object.prototype`), which
    /// is why `Object.create(null) instanceof Object` can read `false`.
    null_proto_objs: HashSet<u32>,
    /// Own properties of function objects (functions are objects in JS): a live
    /// closure's `name`/`prototype`/static-ish members. Keyed by heap index.
    fn_props: HashMap<u32, IndexMap<String, Value>>,
    /// Accessor (getter/setter) properties per owning object, by heap index then
    /// key: `(get, set)`. Class `get x()`/`set x()` install here on the prototype.
    accessors: HashMap<u32, IndexMap<String, Accessor>>,
    /// Own-property attributes that deviate from the plain-assignment default
    /// (`{writable, enumerable, configurable}` all true), by heap index then key.
    /// Only non-default entries are stored, so an ordinary object costs nothing;
    /// `prop_attrs` returns the default for any key absent here. This is what
    /// makes `Object.defineProperty(o, k, {enumerable: false})` invisible to
    /// `Object.keys`/`for-in`/`JSON.stringify` while `getOwnPropertyNames` still
    /// reports it, and what hides `Error`'s `message`/`stack` the way V8 does.
    prop_attrs: HashMap<u32, IndexMap<String, PropAttrs>>,
    /// Heap objects sealed against new properties by `Object.preventExtensions`,
    /// `Object.seal` or `Object.freeze`.
    non_extensible: HashSet<u32>,
    /// User-assigned static properties on a builtin namespace/constructor, keyed
    /// by namespace name then property (`Error` → `prepareStackTrace`,
    /// `stackTraceLimit`). Each bare `Error` reference allocates a fresh
    /// `Builtin` handle, so these cannot live in `fn_props` (which is per-heap-
    /// index); this stable side table lets `Error.prepareStackTrace = fn` persist.
    builtin_statics: HashMap<String, IndexMap<String, Value>>,
    /// The shared well-known `Object.prototype` object (chain root for objects).
    object_proto: Value,
    /// Class name of each class `prototype` object, by heap index — lets an
    /// instance recover its constructor name (for `util.inspect` prefix and
    /// `obj.constructor.name`).
    proto_class: HashMap<u32, Value>,
    /// Class constructor values by name, so a running method's `home_class` name
    /// resolves to its class value (for `super`).
    class_registry: HashMap<String, Value>,
    /// Well-known prototype objects for the builtin error constructors, by name.
    error_protos: HashMap<String, Value>,
    /// Real prototype *objects* for the builtin exotics whose instances need a
    /// genuine `[[Prototype]]` link (`Buffer`, `Uint8Array`). Most builtin
    /// prototypes are `Builtin("<Ctor>.prototype")` thunk namespaces, which
    /// cannot appear on a prototype chain and report `typeof "function"`.
    native_protos: HashMap<String, Value>,
    /// `Symbol.for` registry: description → symbol value.
    symbol_registry: HashMap<String, Value>,
    /// Monotonic id source for fresh `Symbol()` values.
    next_symbol: u64,
    /// Every live symbol by its id, so a `@@sym:<id>` property key can be
    /// turned back into the symbol VALUE for `Object.getOwnPropertySymbols`.
    symbols_by_id: HashMap<u64, Value>,
    /// Well-known symbol ids (`Symbol.iterator` …) to their ECMAScript name.
    /// Identity is by id, not description, so a user `Symbol("Symbol.iterator")`
    /// is a distinct key.
    well_known_ids: HashMap<u64, String>,
    /// Suspended generator coroutines, indexed by `JsObj::Generator.id`.
    generators: Vec<GenCell>,
    /// Promise cells, indexed by `JsObj::Promise.id`.
    promises: Vec<PromiseCell>,
    /// `process.nextTick` callbacks (drained before promise microtasks).
    pub nextticks: std::collections::VecDeque<Task>,
    /// Promise-reaction / `queueMicrotask` microtasks.
    pub microtasks: std::collections::VecDeque<Task>,
    /// `setTimeout`/`setInterval`/`setImmediate` macrotasks.
    pub macrotasks: Vec<Timer>,
    /// Monotonic timer-id source.
    next_timer: u64,
    /// Cloned by I/O worker threads to post `IoTask`s back to the main-thread
    /// event loop. Kept alive for the host's lifetime so the loop's `recv` never
    /// sees a spurious `Disconnected` while a server is running.
    io_tx: Sender<IoTask>,
    /// Owned by the event loop (taken out for the blocking `recv`). Receives the
    /// `IoTask`s posted by I/O threads.
    io_rx: Option<Receiver<IoTask>>,
    /// Ref-count of "things keeping the process alive": open listeners, live
    /// sockets, ref'd handles. The loop exits only when this is `0` AND both task
    /// queues are empty. A pure script never touches it, so it exits exactly as
    /// before.
    open_handles: usize,
    /// In-process output sink. When `Some`, everything the program writes to
    /// stdout/stderr is appended here instead of reaching the process streams —
    /// what an embedder (a TUI that owns the terminal) needs so a `console.log`
    /// cannot corrupt its display. `None` (the default) is the ordinary
    /// standalone `node` behaviour: writes go straight to the real streams.
    capture: Option<String>,
}

/// A queued unit of work: either a JS callback invocation (`queueMicrotask`,
/// `nextTick`, timer body) or a native step (Promise reaction / async resume).
pub enum Task {
    Js { cb: Value, args: Vec<Value> },
    Native(Box<dyn FnOnce() -> Result<(), String>>),
}

impl Task {
    fn run(self) -> Result<(), String> {
        match self {
            Task::Js { cb, args } => invoke(&cb, args, None).map(|_| ()),
            Task::Native(f) => f(),
        }
    }
}

/// A scheduled macrotask (`setTimeout`/`setInterval`/`setImmediate`). Ordering
/// is by `(delay, seq)` — a deterministic virtual clock, never wall time.
pub struct Timer {
    pub id: u64,
    pub delay: f64,
    pub seq: u64,
    pub callback: Value,
    pub args: Vec<Value>,
    pub cancelled: bool,
    /// Repeat period in ms for a `setInterval` timer; `None` for the one-shot
    /// `setTimeout`/`setImmediate`. A repeating timer is re-armed with a fresh
    /// deadline each time it fires, so it keeps the loop alive indefinitely —
    /// exactly like Node, where `setInterval` runs until cleared.
    pub interval: Option<f64>,
    /// Node's `ref`/`unref` handle bit. Only a *referenced* pending timer keeps
    /// the event loop alive; an unref'd one still fires while the loop happens
    /// to be alive for another reason, but never holds it open by itself.
    pub refed: bool,
    /// Real wall-clock deadline (`now + delay`), used only on the real-clock
    /// path (an open handle or a pending interval). On the pure virtual clock
    /// this is ignored.
    pub deadline: Instant,
}

/// One suspended generator. `coro` is `None` only while actively running (taken
/// out across `Coroutine::resume`); `ctx` holds its volatile execution context
/// (frames/signal/error/exc) while suspended.
struct GenCell {
    coro: Option<corosensei::Coroutine<Value, Value, Result<Value, String>>>,
    /// Raw pointer to the coroutine body's `Yielder`, published on entry (same
    /// thread → valid for the body's life). Read by `yield` to suspend.
    yielder: *const (),
    ctx: GenContext,
    done: bool,
    /// True once the body has been resumed at least once (so it is suspended at a
    /// `yield`). `.return()`/`.throw()` only unwind a *started* generator.
    started: bool,
    /// A completion injected by `.return(v)` / `.throw(e)`: consumed by the next
    /// `yield` resume so the body unwinds (running any pending `finally`).
    inject: Option<GenInject>,
    /// True for an `async function*` body, where `await` AND `yield` share one
    /// coroutine yielder: `await` wraps its operand in an await marker so the
    /// driver can tell an internal suspension from a real yield.
    async_gen: bool,
    /// `[[AsyncGeneratorQueue]]` — pending requests as
    /// `(completion, step promise id)`. ECMA-262 27.6.3.6 keeps this queue so
    /// overlapping requests resume the body ONE AT A TIME and settle in request
    /// order; without it a second request issued before the first settles races
    /// past it and the results arrive swapped. `.next`, `.return` AND `.throw`
    /// all enqueue — a `.return()` that skipped the queue would terminate the
    /// body while an earlier `.next()` was still suspended on an `await`, and
    /// that `.next()` would then wrongly report `{done: true}`.
    queue: std::collections::VecDeque<(GenReq, u32)>,
    /// True while a queued request is being driven.
    running: bool,
}

/// A forced completion pushed into a suspended generator by `.return()`/`.throw()`.
enum GenInject {
    Return(Value),
    Throw(Value),
}

/// One queued `[[AsyncGeneratorQueue]]` request. ECMA-262 27.6.3.6
/// `AsyncGeneratorEnqueue` records a *completion*, not just a sent value, which
/// is why `.return()` and `.throw()` queue behind pending `.next()` calls
/// instead of unwinding the body on the spot.
#[derive(Clone)]
pub enum GenReq {
    /// `.next(v)` — resume normally with `v`.
    Next(Value),
    /// `.return(v)` — resume with a forced return completion.
    Return(Value),
    /// `.throw(e)` — resume with a forced throw completion.
    Throw(Value),
}

/// The mutable "execution registers" swapped at every generator resume/suspend
/// boundary so a suspended generator's half-finished frame/signal state never
/// leaks into the resuming caller. The heap, function/class tables and globals
/// are shared and never swapped.
#[derive(Default)]
struct GenContext {
    frames: Vec<Frame>,
    error: Option<String>,
    exc: Option<Value>,
    signal: Option<Signal>,
}

thread_local! {
    /// Id of the generator whose body is currently executing, or `None` at the
    /// root. `yield` suspends this generator.
    static CUR_GEN: std::cell::Cell<Option<u32>> = const { std::cell::Cell::new(None) };
}

thread_local! {
    static HOST: RefCell<JsHost> = RefCell::new(JsHost::new());
}

/// Run `f` with mutable access to the thread-local host.
pub fn with_host<R>(f: impl FnOnce(&mut JsHost) -> R) -> R {
    HOST.with(|h| f(&mut h.borrow_mut()))
}

/// Reset the host to a clean slate (fresh module frame).
pub fn reset_host() {
    with_host(|h| *h = JsHost::new());
    // Drop any cached module handles / factory closure — they index the old heap.
    crate::module::reset();
}

impl Default for JsHost {
    fn default() -> Self {
        Self::new()
    }
}

impl JsHost {
    pub fn new() -> JsHost {
        let global_env = new_env(None);
        let (io_tx, io_rx) = std::sync::mpsc::channel();
        let mut h = JsHost {
            heap: Vec::new(),
            funcs: Vec::new(),
            tries: Vec::new(),
            globals: IndexMap::new(),
            frames: vec![Frame {
                env: global_env.clone(),
                base_env: global_env.clone(),
                this_obj: None,
                new_target: None,
                home_class: None,
                line: 0,
                owner: None,
                is_module: true,
            }],
            global_env,
            error: None,
            exc: None,
            signal: None,
            pending_rejections: Vec::new(),
            process_listeners: IndexMap::new(),
            null_val: Value::Undef,
            protos: HashMap::new(),
            null_proto_objs: HashSet::new(),
            fn_props: HashMap::new(),
            accessors: HashMap::new(),
            prop_attrs: HashMap::new(),
            non_extensible: HashSet::new(),
            builtin_statics: HashMap::new(),
            object_proto: Value::Undef,
            proto_class: HashMap::new(),
            class_registry: HashMap::new(),
            error_protos: HashMap::new(),
            native_protos: HashMap::new(),
            symbol_registry: HashMap::new(),
            next_symbol: 1,
            symbols_by_id: HashMap::new(),
            well_known_ids: HashMap::new(),
            generators: Vec::new(),
            promises: Vec::new(),
            microtasks: std::collections::VecDeque::new(),
            nextticks: std::collections::VecDeque::new(),
            macrotasks: Vec::new(),
            next_timer: 1,
            io_tx,
            io_rx: Some(io_rx),
            open_handles: 0,
            capture: None,
        };
        h.null_val = h.alloc(JsObj::Null);
        // `Object.prototype`: the chain root, its own `[[Prototype]]` is null.
        h.object_proto = h.new_object(IndexMap::new());
        h
    }

    // ── prototype chain ──────────────────────────────────────────────────
    /// The `[[Prototype]]` of a heap value, if explicitly linked.
    pub fn proto_of(&self, v: &Value) -> Option<Value> {
        if let Value::Obj(i) = v {
            self.protos.get(i).cloned()
        } else {
            None
        }
    }
    /// Set `v`'s `[[Prototype]]` to `proto`. Null links the object as an explicit
    /// null-prototype object (recorded so `instanceof Object` reads false);
    /// undefined just clears any link without the null marker.
    pub fn set_proto(&mut self, v: &Value, proto: Value) {
        if let Value::Obj(i) = v {
            if self.is_null(&proto) {
                self.protos.remove(i);
                self.null_proto_objs.insert(*i);
            } else if matches!(proto, Value::Undef) {
                self.protos.remove(i);
            } else {
                self.protos.insert(*i, proto);
                self.null_proto_objs.remove(i);
            }
        }
    }
    /// Whether `v`'s `[[Prototype]]` was explicitly set to null.
    pub fn has_null_proto(&self, v: &Value) -> bool {
        matches!(v, Value::Obj(i) if self.null_proto_objs.contains(i))
    }
    pub fn object_proto(&self) -> Value {
        self.object_proto.clone()
    }
    /// Record that the prototype object `proto` belongs to the class constructor
    /// `class_val` (so instances can recover their constructor).
    pub fn tag_proto_class(&mut self, proto: &Value, class_val: Value) {
        if let Value::Obj(i) = proto {
            self.proto_class.insert(*i, class_val);
        }
    }
    /// The class constructor value nearest in `obj`'s prototype chain, if any.
    pub fn class_of(&self, obj: &Value) -> Option<Value> {
        let mut cur = self.proto_of(obj);
        while let Some(p) = cur {
            if let Value::Obj(i) = &p {
                if let Some(c) = self.proto_class.get(i) {
                    return Some(c.clone());
                }
            }
            cur = self.proto_of(&p);
        }
        None
    }
    /// The constructor display name of `obj` for `util.inspect` (empty ⇒ plain
    /// object, no prefix).
    pub fn ctor_name(&self, obj: &Value) -> String {
        if let Some(c) = self.class_of(obj) {
            if let Some(JsObj::Class(cv)) = self.get(&c) {
                return cv.name.clone();
            }
        }
        // A `function F(){}` constructor is not a `class`, so it has no
        // `proto_class` entry. V8's `getConstructorName` walks the prototype
        // chain for an own `constructor` that is a named function — which is
        // what makes `console.log(new F())` print `F { y: 2 }`.
        let mut cur = self.proto_of(obj);
        while let Some(p) = cur {
            let ctor = match self.get(&p) {
                Some(JsObj::Object(props)) => props.get("constructor").cloned(),
                Some(JsObj::Func(_)) | Some(JsObj::Class(_)) => self.fn_prop(&p, "constructor"),
                _ => None,
            };
            if let Some(f) = ctor {
                let n = self.callable_name(&f);
                if !n.is_empty() {
                    return n;
                }
            }
            cur = self.proto_of(&p);
        }
        String::new()
    }

    /// Whether a callable owns a `prototype` property. `MakeConstructor`
    /// (10.2.5) runs for an ordinary function definition and for every
    /// generator; an arrow, a `MethodDefinition`, an async function and a bound
    /// function are not constructors and own none.
    pub fn owns_prototype(&self, v: &Value) -> bool {
        match self.get(v) {
            Some(JsObj::Class(_)) => true,
            Some(JsObj::Func(f)) => match self.funcs.get(f.def_id) {
                Some(d) => d.is_generator || !(d.is_arrow || d.is_async || d.is_method),
                None => false,
            },
            _ => false,
        }
    }

    /// A function's own-property table (created on demand).
    pub fn fn_prop(&self, v: &Value, name: &str) -> Option<Value> {
        if let Value::Obj(i) = v {
            self.fn_props.get(i).and_then(|m| m.get(name).cloned())
        } else {
            None
        }
    }

    /// A class static member, inherited down the constructor chain: a subclass
    /// sees its superclass's `static` methods/fields (`Sub.create` → `Base.create`).
    pub fn class_static(&self, class_val: &Value, name: &str) -> Option<Value> {
        let mut cur = class_val.clone();
        loop {
            if let Some(v) = self.fn_prop(&cur, name) {
                return Some(v);
            }
            match self.get(&cur) {
                Some(JsObj::Class(c)) => cur = c.parent.clone()?,
                _ => return None,
            }
        }
    }
    pub fn set_fn_prop(&mut self, v: &Value, name: &str, val: Value) {
        if let Value::Obj(i) = v {
            self.fn_props
                .entry(*i)
                .or_default()
                .insert(name.to_string(), val);
        }
        // `name` and `prototype` are own properties of every function/class, but
        // never enumerable ones (SetFunctionName 10.2.9, MakeConstructor
        // 10.2.5), so `Object.keys(fn)` and `for (k in fn)` report only what a
        // script assigned. An ARRAY receiver reaching the same side table has no
        // such exotic keys — `arr.name = 'x'` is an ordinary enumerable property.
        if !matches!(self.kind_of(v), Some(ObjKind::Func) | Some(ObjKind::Class)) {
            return;
        }
        let attrs = match name {
            "name" => PropAttrs {
                writable: false,
                enumerable: false,
                configurable: true,
            },
            "prototype" => PropAttrs {
                writable: true,
                enumerable: false,
                configurable: false,
            },
            _ => return,
        };
        self.set_prop_attrs(v, name, attrs);
    }
    /// A user-assigned static on a builtin namespace (`Error.prepareStackTrace`).
    pub fn builtin_static(&self, ns: &str, name: &str) -> Option<Value> {
        self.builtin_statics
            .get(ns)
            .and_then(|m| m.get(name).cloned())
    }
    /// Assign a static on a builtin namespace (persists across fresh `Builtin`
    /// handles for the same namespace).
    pub fn set_builtin_static(&mut self, ns: &str, name: &str, val: Value) {
        self.builtin_statics
            .entry(ns.to_string())
            .or_default()
            .insert(name.to_string(), val);
    }
    /// Drop an own property from the side table (`delete arr.foo`,
    /// `delete fn.tag`). Reports whether the key was there.
    pub fn remove_fn_prop(&mut self, v: &Value, name: &str) -> bool {
        match v {
            Value::Obj(i) => self
                .fn_props
                .get_mut(i)
                .map(|m| m.shift_remove(name).is_some())
                .unwrap_or(false),
            _ => false,
        }
    }
    pub fn fn_prop_keys(&self, v: &Value) -> Vec<String> {
        if let Value::Obj(i) = v {
            self.fn_props
                .get(i)
                .map(|m| m.keys().cloned().collect())
                .unwrap_or_default()
        } else {
            Vec::new()
        }
    }

    /// Install an accessor `(get, set)` for `key` on the object `owner`.
    pub fn set_accessor(
        &mut self,
        owner: &Value,
        key: &str,
        get: Option<Value>,
        set: Option<Value>,
    ) {
        if let Value::Obj(i) = owner {
            // Accessors live in their own table, but JS reports own keys in a
            // single insertion order across data AND accessor properties. Drop an
            // ordering marker into the property map so
            // `{ a: 1, get b() {}, c: 3 }` enumerates a, b, c — not a, c, b.
            // The marker is `@@`-prefixed, so it is invisible to every reader.
            let marker = format!("{ORD_MARKER}{key}");
            if let Some(JsObj::Object(props)) = self.get_mut(owner) {
                if !props.contains_key(key) && !props.contains_key(&marker) {
                    props.insert(marker, Value::Undef);
                }
            }
            let slot = self
                .accessors
                .entry(*i)
                .or_default()
                .entry(key.to_string())
                .or_insert((None, None));
            if get.is_some() {
                slot.0 = get;
            }
            if set.is_some() {
                slot.1 = set;
            }
        }
    }
    /// The accessor `(get, set)` for `key` directly on `owner` (no chain walk).
    pub fn own_accessor(&self, owner: &Value, key: &str) -> Option<(Option<Value>, Option<Value>)> {
        if let Value::Obj(i) = owner {
            self.accessors.get(i).and_then(|m| m.get(key).cloned())
        } else {
            None
        }
    }

    /// The own accessor-property keys of `owner`, in installation order.
    pub fn own_accessor_keys(&self, owner: &Value) -> Vec<String> {
        match owner {
            Value::Obj(i) => self
                .accessors
                .get(i)
                .map(|m| m.keys().cloned().collect())
                .unwrap_or_default(),
            _ => Vec::new(),
        }
    }

    // ── own-property attributes ──────────────────────────────────────────

    /// Record non-default attributes for `owner[key]`. Storing the default shape
    /// clears the entry so the table only ever holds deviations.
    pub fn set_prop_attrs(&mut self, owner: &Value, key: &str, attrs: PropAttrs) {
        if let Value::Obj(i) = owner {
            if attrs == PropAttrs::default() {
                if let Some(m) = self.prop_attrs.get_mut(i) {
                    m.shift_remove(key);
                }
            } else {
                self.prop_attrs
                    .entry(*i)
                    .or_default()
                    .insert(key.to_string(), attrs);
            }
        }
    }

    /// Copy every recorded property attribute from `from` to `to`. A pass that
    /// rebuilds an object (`JSON.stringify`'s `toJSON` walk) must carry them
    /// across or the copy silently re-exposes non-enumerable slots.
    pub fn copy_prop_attrs(&mut self, from: &Value, to: &Value) {
        if let (Value::Obj(f), Value::Obj(_)) = (from, to) {
            if let Some(m) = self.prop_attrs.get(f).cloned() {
                for (k, a) in m {
                    self.set_prop_attrs(to, &k, a);
                }
            }
        }
    }

    /// The attributes of own property `owner[key]` (all-true when unrecorded).
    pub fn prop_attrs(&self, owner: &Value, key: &str) -> PropAttrs {
        // An array's `length` is the array exotic's own property (10.4.2):
        // writable, but never enumerated and never configurable.
        if key == "length" && matches!(self.get(owner), Some(JsObj::Array(_))) {
            return PropAttrs {
                writable: true,
                enumerable: false,
                configurable: false,
            };
        }
        match owner {
            Value::Obj(i) => self
                .prop_attrs
                .get(i)
                .and_then(|m| m.get(key))
                .copied()
                .unwrap_or_default(),
            _ => PropAttrs::default(),
        }
    }

    /// Whether own property `owner[key]` shows up in `for-in`/`Object.keys`.
    /// Internal slots (`@@…`) and private class fields (`#…`) never do.
    pub fn is_enumerable(&self, owner: &Value, key: &str) -> bool {
        !key.starts_with("@@") && !key.starts_with('#') && self.prop_attrs(owner, key).enumerable
    }

    /// Mark `owner[key]` non-enumerable, leaving it writable/configurable — the
    /// shape of every V8 "hidden but real" own property.
    pub fn hide_prop(&mut self, owner: &Value, key: &str) {
        self.set_prop_attrs(owner, key, PropAttrs::HIDDEN);
    }

    /// Whether a plain `owner[key] = v` assignment is allowed to land. A
    /// non-writable data property silently ignores the write in sloppy mode,
    /// which is the mode every script here runs in; so does adding a *new* key to
    /// a non-extensible object.
    pub fn can_write_prop(&self, owner: &Value, key: &str) -> bool {
        if !self.prop_attrs(owner, key).writable {
            return false;
        }
        if self.is_extensible(owner) {
            return true;
        }
        match self.get(owner) {
            Some(JsObj::Object(p)) => p.contains_key(key),
            _ => true,
        }
    }

    /// Mark `v` closed to new properties (`Object.preventExtensions`).
    pub fn prevent_extensions(&mut self, v: &Value) {
        if let Value::Obj(i) = v {
            self.non_extensible.insert(*i);
        }
    }

    pub fn is_extensible(&self, v: &Value) -> bool {
        !matches!(v, Value::Obj(i) if self.non_extensible.contains(i))
    }

    /// Apply `Object.seal` (`freeze == false`) or `Object.freeze` (`true`): close
    /// the object and strip `configurable` — and, when freezing, `writable` —
    /// from every own property, data and accessor alike.
    pub fn seal_object(&mut self, v: &Value, freeze: bool) {
        self.prevent_extensions(v);
        let mut keys = match self.get(v) {
            Some(JsObj::Object(p)) => p.keys().cloned().collect::<Vec<_>>(),
            _ => Vec::new(),
        };
        keys.extend(self.own_accessor_keys(v));
        for k in keys {
            let mut a = self.prop_attrs(v, &k);
            a.configurable = false;
            if freeze {
                a.writable = false;
            }
            self.set_prop_attrs(v, &k, a);
        }
    }

    /// `Object.isSealed` (`freeze == false`) / `Object.isFrozen` (`true`).
    pub fn is_sealed(&self, v: &Value, freeze: bool) -> bool {
        if self.is_extensible(v) {
            return false;
        }
        let mut keys = match self.get(v) {
            Some(JsObj::Object(p)) => p.keys().cloned().collect::<Vec<_>>(),
            _ => Vec::new(),
        };
        keys.extend(self.own_accessor_keys(v));
        keys.iter().all(|k| {
            let a = self.prop_attrs(v, k);
            !a.configurable && (!freeze || !a.writable)
        })
    }

    /// A fresh unique `Symbol(desc)` value.
    pub fn new_symbol(&mut self, desc: Option<String>) -> Value {
        let id = self.next_symbol;
        self.next_symbol += 1;
        let v = self.alloc(JsObj::Symbol { desc, id });
        self.symbols_by_id.insert(id, v.clone());
        v
    }

    /// The symbol VALUE an internal symbol property key (`@@sym:<id>` or a
    /// well-known `@@iterator`) came from.
    pub fn symbol_of_key(&self, k: &str) -> Option<Value> {
        if let Some(id) = k.strip_prefix("@@sym:").and_then(|i| i.parse::<u64>().ok()) {
            return self.symbols_by_id.get(&id).cloned();
        }
        let name = k.strip_prefix("@@")?;
        WELL_KNOWN_SYMBOLS
            .contains(&name)
            .then(|| {
                self.symbol_registry
                    .get(&format!("@@Symbol.{name}"))
                    .cloned()
            })
            .flatten()
    }

    /// The own symbol-keyed property keys of `v` as SYMBOL values —
    /// `Object.getOwnPropertySymbols` / the symbol half of `Reflect.ownKeys`.
    pub fn own_symbol_keys(&self, v: &Value) -> Vec<Value> {
        let keys: Vec<String> = match self.get(v) {
            Some(JsObj::Object(p)) => p.keys().cloned().collect(),
            // An Array/Function receiver has no property map: its non-index own
            // properties — symbol-keyed ones included — live in the fn-prop side
            // table, and are just as much own properties as an object's.
            Some(_) => self.fn_prop_keys(v),
            None => return Vec::new(),
        };
        keys.iter().filter_map(|k| self.symbol_of_key(k)).collect()
    }

    /// The own SYMBOL-keyed enumerable `(internal key, value)` pairs of `v` —
    /// what `CopyDataProperties` (object spread, `Object.assign`) copies
    /// alongside the string keys, and what `Object.keys` / `for-in` /
    /// `JSON.stringify` deliberately skip.
    pub fn own_symbol_entries(&self, v: &Value) -> Vec<(String, Value)> {
        match self.get(v) {
            Some(JsObj::Object(p)) => p
                .iter()
                .filter(|(k, _)| is_symbol_key(k) && self.prop_attrs(v, k).enumerable)
                .map(|(k, val)| (k.clone(), val.clone()))
                .collect(),
            // Array/Function: the side table (see `own_symbol_keys`).
            Some(_) => self
                .fn_prop_keys(v)
                .into_iter()
                .filter(|k| is_symbol_key(k) && self.prop_attrs(v, k).enumerable)
                .map(|k| {
                    let val = self.fn_prop(v, &k).unwrap_or(Value::Undef);
                    (k, val)
                })
                .collect(),
            None => Vec::new(),
        }
    }
    /// The shared `Symbol.for(key)` value (interned by description).
    pub fn symbol_for(&mut self, key: &str) -> Value {
        if let Some(v) = self.symbol_registry.get(key) {
            return v.clone();
        }
        let s = self.new_symbol(Some(key.to_string()));
        self.symbol_registry.insert(key.to_string(), s.clone());
        s
    }
    /// The well-known `Symbol.iterator` (a fixed shared symbol whose internal
    /// property key is `@@iterator`).
    pub fn well_known_iterator(&mut self) -> Value {
        self.symbol_for("@@Symbol.iterator")
    }
    /// The well-known `Symbol.asyncIterator` (internal key `@@asyncIterator`).
    pub fn well_known_async_iterator(&mut self) -> Value {
        self.symbol_for("@@Symbol.asyncIterator")
    }
    /// A well-known symbol by its ECMAScript name (`toPrimitive`,
    /// `toStringTag`, …). Its internal property key is `@@<name>` — see
    /// [`WELL_KNOWN_SYMBOLS`] and `property_key`.
    ///
    /// Its DESCRIPTION is `Symbol.<name>`, so `String(Symbol.iterator)` prints
    /// `Symbol(Symbol.iterator)` as V8 does, while the registry key keeps the
    /// `@@` prefix — `Symbol.for('Symbol.iterator')` therefore stays a
    /// different symbol, and identification is by id, so a user-made
    /// `Symbol('Symbol.iterator')` is not mistaken for the well-known one.
    pub fn well_known_symbol(&mut self, name: &str) -> Value {
        let key = format!("@@Symbol.{name}");
        if let Some(v) = self.symbol_registry.get(&key) {
            return v.clone();
        }
        let s = self.new_symbol(Some(format!("Symbol.{name}")));
        if let Some(JsObj::Symbol { id, .. }) = self.get(&s) {
            self.well_known_ids.insert(*id, name.to_string());
        }
        self.symbol_registry.insert(key, s.clone());
        s
    }
    /// The internal property-key string for a value used as a key. A `Symbol`
    /// maps to a stable per-symbol string so symbol-keyed props round-trip;
    /// `Symbol.iterator` maps to the sentinel `@@iterator`.
    pub fn property_key(&self, v: &Value) -> String {
        if let Some(JsObj::Symbol { id, .. }) = self.get(v) {
            if let Some(n) = self.well_known_ids.get(id) {
                return format!("@@{n}");
            }
            return format!("@@sym:{id}");
        }
        self.str_of(v)
    }

    pub fn null(&self) -> Value {
        self.null_val.clone()
    }
    pub fn is_null(&self, v: &Value) -> bool {
        matches!(self.get(v), Some(JsObj::Null))
    }

    // ── program loading ──────────────────────────────────────────────────
    pub fn program_offsets(&self) -> (usize, usize) {
        (self.funcs.len(), self.tries.len())
    }
    pub fn load_program(&mut self, funcs: Vec<FuncDef>, tries: Vec<TryDef>) {
        self.funcs.extend(funcs);
        self.tries.extend(tries);
    }
    pub fn try_def(&self, id: usize) -> Option<TryDef> {
        self.tries.get(id).cloned()
    }

    // ── heap allocation / accessors ──────────────────────────────────────
    pub fn alloc(&mut self, obj: JsObj) -> Value {
        self.heap.push(obj);
        Value::Obj((self.heap.len() - 1) as u32)
    }
    pub fn get(&self, v: &Value) -> Option<&JsObj> {
        if let Value::Obj(i) = v {
            self.heap.get(*i as usize)
        } else {
            None
        }
    }
    pub fn get_mut(&mut self, v: &Value) -> Option<&mut JsObj> {
        if let Value::Obj(i) = v {
            self.heap.get_mut(*i as usize)
        } else {
            None
        }
    }
    /// Which variant `v` points at, without copying its contents. Use this in
    /// place of `get(v).cloned()` whenever only the tag is needed — see
    /// [`ObjKind`].
    pub fn kind_of(&self, v: &Value) -> Option<ObjKind> {
        self.get(v).map(JsObj::kind)
    }
    pub fn new_str(&mut self, s: impl Into<String>) -> Value {
        self.alloc(JsObj::Str(s.into()))
    }
    pub fn new_array(&mut self, items: Vec<Value>) -> Value {
        self.alloc(JsObj::Array(items))
    }
    pub fn new_object(&mut self, mut props: IndexMap<String, Value>) -> Value {
        // Integer-index keys enumerate ascending-first regardless of the order
        // they were supplied in (object literal, spread, Object.assign result).
        canonicalize_own_keys(&mut props);
        self.alloc(JsObj::Object(props))
    }
    pub fn as_str(&self, v: &Value) -> Option<String> {
        match v {
            Value::Str(s) => Some((**s).clone()),
            Value::Obj(_) => match self.get(v) {
                Some(JsObj::Str(s)) => Some(s.clone()),
                _ => None,
            },
            _ => None,
        }
    }

    // ── scope / names ────────────────────────────────────────────────────
    fn frame(&self) -> &Frame {
        self.frames.last().unwrap()
    }
    fn cur_env(&self) -> Env {
        self.frame().env.clone()
    }

    // ── DAP debug introspection (used only under `--dap`) ────────────────────
    /// Number of active call frames (the debugger's step-depth reference).
    pub fn frame_depth(&self) -> usize {
        self.frames.len()
    }
    /// Record the source line the innermost frame is executing (DAP line hook).
    pub fn set_cur_line(&mut self, line: u32) {
        if let Some(f) = self.frames.last_mut() {
            f.line = line;
        }
    }
    /// The `.stack` tail for an error created right now: one `    at <name>`
    /// line per live frame, innermost first, ending at the module frame.
    ///
    /// These are the REAL user frames — node-js has no `file:line:column` (the
    /// per-frame line is only tracked under `--dap`) and no Node-internal
    /// module-loader frames, so `.stack` names the call chain but can never be
    /// byte-identical to V8's. The names are what makes a thrown error
    /// diagnosable; the missing positions are documented in BUGS.md.
    pub fn stack_frames(&self) -> String {
        let mut out = String::new();
        for (i, f) in self.frames.iter().enumerate().rev() {
            let name = match (&f.owner, i) {
                (Some(n), _) => n.clone(),
                (None, 0) => "Object.<anonymous>".to_string(),
                (None, _) => "<anonymous>".to_string(),
            };
            out.push_str("\n    at ");
            out.push_str(&name);
        }
        if out.is_empty() {
            out.push_str("\n    at <anonymous>");
        }
        out
    }

    /// The call stack as (frame name, line) pairs, innermost first — for the DAP
    /// `stackTrace`. `owner` carries the function name where known.
    pub fn dbg_stack(&self) -> Vec<(String, u32)> {
        self.frames
            .iter()
            .rev()
            .map(|f| {
                let name = f.owner.clone().unwrap_or_else(|| "<module>".to_string());
                (name, f.line)
            })
            .collect()
    }
    /// The innermost frame's locals as (name, inspect) pairs — for DAP `variables`.
    pub fn dbg_locals(&self) -> Vec<(String, String)> {
        let env = self.cur_env();
        let names: Vec<String> = env.borrow().vars.keys().cloned().collect();
        names
            .into_iter()
            .map(|n| {
                let v = self.read_name(&n).unwrap_or(Value::Undef);
                (n, self.inspect(&v))
            })
            .collect()
    }

    /// Scope-chain read: local + enclosing chain, then globals.
    pub fn read_name(&self, name: &str) -> Option<Value> {
        let mut env = Some(self.cur_env());
        while let Some(e) = env {
            if let Some(v) = e.borrow().vars.get(name) {
                return Some(v.clone());
            }
            env = e.borrow().parent.clone();
        }
        self.globals.get(name).cloned()
    }
    pub fn read_global(&self, name: &str) -> Option<Value> {
        self.globals.get(name).cloned()
    }

    /// Assign to an existing binding up the scope chain, else create a global
    /// (JS assignment to an undeclared name targets the global object).
    pub fn set_name(&mut self, name: &str, val: Value) {
        let mut env = Some(self.cur_env());
        while let Some(e) = env {
            if e.borrow().vars.contains_key(name) {
                e.borrow_mut().vars.insert(name.to_string(), val);
                return;
            }
            env = e.borrow().parent.clone();
        }
        self.globals.insert(name.to_string(), val);
    }

    /// Declare a new binding in the current scope (`let`/`const`). At the top of
    /// the module frame there is no local env, so those names become globals; once
    /// a block scope is open the binding belongs to that block.
    pub fn declare_name(&mut self, name: &str, val: Value) {
        let f = self.frame();
        if f.is_module && Rc::ptr_eq(&f.env, &f.base_env) {
            self.globals.insert(name.to_string(), val);
        } else {
            self.cur_env()
                .borrow_mut()
                .vars
                .insert(name.to_string(), val);
        }
    }

    /// Declare a `var` (or a hoisted function declaration): FUNCTION-scoped, so it
    /// skips every open block scope and lands in the activation's base env.
    pub fn declare_var_name(&mut self, name: &str, val: Value) {
        if self.frame().is_module {
            self.globals.insert(name.to_string(), val);
            return;
        }
        let base = self.frame().base_env.clone();
        base.borrow_mut().vars.insert(name.to_string(), val);
    }

    /// Enter a fresh block scope.
    pub fn push_scope(&mut self) {
        let env = self.cur_env();
        self.frames.last_mut().unwrap().env = child_env(env);
    }

    /// Leave the innermost block scope (never pops past the activation's base).
    pub fn pop_scope(&mut self) {
        let cur = self.cur_env();
        if Rc::ptr_eq(&cur, &self.frame().base_env) {
            return;
        }
        let parent = cur.borrow().parent.clone();
        if let Some(p) = parent {
            self.frames.last_mut().unwrap().env = p;
        }
    }

    /// Replace the innermost block scope with a fresh copy of its bindings — the
    /// per-iteration environment a `for (let i …)` loop creates, so a closure made
    /// in one iteration keeps that iteration's value.
    pub fn copy_scope(&mut self) {
        let cur = self.cur_env();
        if Rc::ptr_eq(&cur, &self.frame().base_env) {
            return;
        }
        let parent = cur.borrow().parent.clone();
        let fresh = new_env(parent);
        fresh.borrow_mut().vars = cur.borrow().vars.clone();
        self.frames.last_mut().unwrap().env = fresh;
    }

    /// The current block-scope env, for save/restore across a nested chunk.
    pub fn scope_snapshot(&self) -> Env {
        self.cur_env()
    }
    pub fn restore_scope(&mut self, env: Env) {
        self.frames.last_mut().unwrap().env = env;
    }
    pub fn set_global(&mut self, name: &str, val: Value) {
        self.globals.insert(name.to_string(), val);
    }

    // ── output capture ───────────────────────────────────────────────────
    //
    // Every write a *program* makes — `console.log`, `process.stdout.write`,
    // `print` — funnels through `write_out`, so turning capture on redirects all
    // of them at once. Diagnostics the runtime itself emits (the REPL banner, a
    // crash traceback from `main`) deliberately do not: they belong to the
    // process, not to the program.

    /// Start capturing program output in-process. Any text already captured is
    /// discarded, so each run starts clean.
    pub fn begin_capture(&mut self) {
        self.capture = Some(String::new());
    }

    /// Stop capturing and take everything written since [`begin_capture`],
    /// returning the empty string when capture was not on.
    ///
    /// [`begin_capture`]: JsHost::begin_capture
    pub fn end_capture(&mut self) -> String {
        self.capture.take().unwrap_or_default()
    }

    /// Whether output is being captured — the one thing a caller needs to know
    /// before asking the real stream a question (`isTTY`, cursor position).
    pub fn capturing(&self) -> bool {
        self.capture.is_some()
    }

    /// Write program output: into the capture buffer when capturing, else to the
    /// process stream `stderr` selects. `s` is written verbatim — callers add
    /// their own line ending, as `console.log` does and `process.stdout.write`
    /// does not.
    pub fn write_out(&mut self, s: &str, stderr: bool) {
        if let Some(buf) = &mut self.capture {
            buf.push_str(s);
            return;
        }
        use std::io::Write as _;
        if stderr {
            let mut e = std::io::stderr();
            let _ = e.write_all(s.as_bytes());
            let _ = e.flush();
        } else {
            let mut o = std::io::stdout();
            let _ = o.write_all(s.as_bytes());
            let _ = o.flush();
        }
    }
    pub fn del_name(&mut self, name: &str) {
        if self
            .cur_env()
            .borrow_mut()
            .vars
            .shift_remove(name)
            .is_some()
        {
            return;
        }
        self.globals.shift_remove(name);
    }

    pub fn current_this(&self) -> Option<Value> {
        self.frame().this_obj.clone()
    }
    pub fn current_env_capture(&self) -> Env {
        self.frame().env.clone()
    }
    pub fn current_new_target(&self) -> Option<Value> {
        self.frame().new_target.clone()
    }
    fn current_home_class(&self) -> Option<Value> {
        self.frame().home_class.clone()
    }

    /// The `(parent_ctor, this_class_fields)` for a running constructor's
    /// `super(...)`, derived from the frame's home class.
    pub fn super_context(&self) -> (Option<Value>, Vec<(String, Value, bool)>) {
        match self.current_home_class() {
            Some(cv) => match self.get(&cv) {
                Some(JsObj::Class(c)) => (c.parent.clone(), c.fields.clone()),
                _ => (None, Vec::new()),
            },
            None => (None, Vec::new()),
        }
    }

    /// Resolve `super.name` to either the parent-prototype getter (to be invoked
    /// by the caller, outside any host borrow) or a directly-usable value.
    pub fn super_resolve(&self, name: &str) -> SuperRef {
        let parent = match self
            .current_home_class()
            .and_then(|cv| match self.get(&cv) {
                Some(JsObj::Class(c)) => c.parent.clone(),
                _ => None,
            }) {
            Some(p) => p,
            None => return SuperRef::Data(Value::Undef),
        };
        let parent_proto = match self.get(&parent) {
            Some(JsObj::Class(pc)) => pc.proto.clone(),
            _ => self.fn_prop(&parent, "prototype").unwrap_or(Value::Undef),
        };
        if let Some((Some(getter), _)) = lookup_accessor(self, &parent_proto, name) {
            return SuperRef::Getter(getter);
        }
        SuperRef::Data(lookup_chain(self, &parent_proto, name).unwrap_or(Value::Undef))
    }

    // ── signals / errors ─────────────────────────────────────────────────
    pub fn take_error(&mut self) -> Option<String> {
        self.error.take()
    }
    pub fn raise_str(&mut self, class: &str, msg: &str) -> String {
        let s = if msg.is_empty() {
            class.to_string()
        } else {
            format!("{class}: {msg}")
        };
        self.error = Some(s.clone());
        s
    }
}

// ── error constructors ───────────────────────────────────────────────────────

pub fn type_error(msg: &str) -> String {
    format!("TypeError: {msg}")
}
pub fn ref_error(name: &str) -> String {
    format!("ReferenceError: {name} is not defined")
}
pub fn range_error(msg: &str) -> String {
    format!("RangeError: {msg}")
}

// ── the fusevm run plumbing ──────────────────────────────────────────────────

thread_local! {
    static DEBUG_MODE: std::cell::Cell<bool> = const { std::cell::Cell::new(false) };
}

/// Enable/disable DAP debug execution (`node --dap`).
pub fn set_debug_mode(on: bool) {
    DEBUG_MODE.with(|d| d.set(on));
}

/// Register every node-js builtin + the numeric hook on a VM, then run it.
pub fn run_chunk_on(chunk: Chunk) -> Result<Value, String> {
    let mut vm = VM::new(chunk);
    crate::builtins::install(&mut vm);
    vm.set_numeric_hook(std::sync::Arc::new(|op, a, b| {
        crate::builtins::numeric_hook(op, a, b)
    }));
    // Under `--dap` the tracing JIT would compile hot loops and skip the
    // per-statement `DBG_LINE` markers, so debug runs stay on the pure
    // interpreter. The `DBG_LINE` builtin fires the debugger line hook; the
    // extension seam mirrors pythonrs should the marker emission ever switch.
    if DEBUG_MODE.with(|d| d.get()) {
        vm.set_extension_handler(Box::new(|vm, id, _| {
            crate::dap::on_ext(vm, id);
        }));
    } else {
        vm.enable_tracing_jit();
    }
    let outcome = vm.run();
    if let Some(e) = with_host(|h| h.take_error()) {
        return Err(e);
    }
    match outcome {
        VMResult::Ok(v) => Ok(v),
        VMResult::Halted => Ok(vm.stack.last().cloned().unwrap_or(Value::Undef)),
        VMResult::Error(e) => Err(e),
    }
}

/// Run `chunk` in the GLOBAL scope instead of the caller's.
///
/// `run_chunk_on` executes on whatever frame is current, so a nested run sees —
/// and can shadow — the *calling function's* locals. That is right for a direct
/// `eval`, and wrong for every other runtime-source construct: a `new Function`
/// body, an indirect `eval` and `vm.runInThisContext` are all specified to run
/// in the global scope (ECMA-262 19.2.1.1 `PerformEval` with a null
/// `strictCaller`/`direct` pair; `FunctionBody` is instantiated with the *global*
/// environment, 20.2.1.1.1 step 26). Measured against node v26.7.0,
/// `function outer(){ let loc = 42; return vm.runInThisContext('typeof loc'); }`
/// is `"undefined"` there and was `"number"` here.
///
/// A `var` the chunk itself declares lands in the top-level scope and persists,
/// so successive `vm.runInThisContext` calls share it.
pub fn run_chunk_in_global_scope(chunk: Chunk) -> Result<Value, String> {
    let global_env = with_host(|h| h.global_env.clone());
    with_host(|h| {
        h.frames.push(Frame {
            env: global_env.clone(),
            base_env: global_env,
            this_obj: None,
            new_target: None,
            home_class: None,
            line: 0,
            owner: None,
            is_module: true,
        })
    });
    let r = run_chunk_on(chunk);
    with_host(|h| {
        h.frames.pop();
    });
    r
}

/// Run the top-level program chunk, then drain the event loop (microtasks +
/// timers) until quiescent — matching Node, which keeps the process alive while
/// pending async work remains.
pub fn run_main(chunk: Chunk) -> Result<Value, String> {
    let r = run_chunk_on(chunk);
    with_host(|h| h.signal = None);
    if r.is_ok() {
        run_event_loop()?;
    }
    r
}

// ── formatting ───────────────────────────────────────────────────────────────

/// Format a JS number exactly as `Number.prototype.toString` does for the common
/// range (no exponential-notation threshold handling for very large/small).
pub fn fmt_number(f: f64) -> String {
    if f.is_nan() {
        return "NaN".into();
    }
    if f.is_infinite() {
        return if f > 0.0 { "Infinity" } else { "-Infinity" }.into();
    }
    if f == 0.0 {
        // Covers -0.0 too: (-0).toString() === "0".
        return "0".into();
    }
    if f < 0.0 {
        return format!("-{}", js_number_repr(-f));
    }
    js_number_repr(f)
}

/// If `k` is an array-index property key, return its numeric value. Per
/// ECMAScript, a String property key `P` is an array index iff
/// `ToString(ToUint32(P)) === P` and `ToUint32(P) !== 2^32 - 1` — i.e. a
/// canonical decimal (no leading zeros, no sign) in the range `0..=2^32-2`.
pub fn array_index(k: &str) -> Option<u32> {
    if k.is_empty() {
        return None;
    }
    if k == "0" {
        return Some(0);
    }
    // A leading '0' (other than the lone "0" above) is non-canonical.
    if k.as_bytes()[0] == b'0' {
        return None;
    }
    if !k.bytes().all(|b| b.is_ascii_digit()) {
        return None;
    }
    match k.parse::<u64>() {
        // Array index must be < 2^32-1; u32::MAX == 2^32-1 is excluded.
        Ok(n) if n < u32::MAX as u64 => Some(n as u32),
        _ => None,
    }
}

/// Compare two own-property keys for `OrdinaryOwnPropertyKeys` enumeration order:
/// integer-index keys sort ascending-numeric and precede all string keys; two
/// non-index keys compare `Equal` so a *stable* sort leaves them in insertion
/// order. (Symbols are stored as `@@…`/`#…` string keys and are non-index, so
/// they also fall into the stable-insertion-order tail.)
pub fn key_order_cmp(a: &str, b: &str) -> std::cmp::Ordering {
    use std::cmp::Ordering;
    match (array_index(a), array_index(b)) {
        (Some(x), Some(y)) => x.cmp(&y),
        (Some(_), None) => Ordering::Less,
        (None, Some(_)) => Ordering::Greater,
        (None, None) => Ordering::Equal,
    }
}

/// Reorder an object's own-property map into `OrdinaryOwnPropertyKeys` order in
/// place: array-index keys ascending first, then the remaining keys in their
/// existing (insertion) order. A no-op unless at least one index key is present,
/// so the overwhelmingly common all-string-key object keeps its exact order and
/// pays nothing. `IndexMap::sort_by` is a stable sort.
pub fn canonicalize_own_keys(props: &mut IndexMap<String, Value>) {
    if props.keys().any(|k| array_index(k).is_some()) {
        props.sort_by(|ak, _, bk, _| key_order_cmp(ak, bk));
    }
}

/// ECMAScript `Number::toString` layout for a positive, finite, nonzero value.
///
/// Rust's `Display`/`LowerExp` give the shortest round-trip decimal digits, but
/// NOT JavaScript's exponential-vs-fixed threshold: Rust prints `1e21` as
/// `1000000000000000000000` and `1e-7` as `0.0000001`, whereas JS prints `1e+21`
/// and `1e-7`. So we take the shortest digits from `{:e}` and re-lay them out per
/// the spec (steps 5–10 of Number::toString): `k` significant digits `s` with
/// decimal exponent `n` (value = s × 10^(n−k)); exponential form only when
/// `n > 21` or `n ≤ -6`.
fn js_number_repr(a: f64) -> String {
    // `{:e}` yields `d[.ddd]e<exp>` with the mantissa in [1, 10) and shortest
    // round-trip digits. Split it into the digit string `s` and exponent `E`.
    let sci = format!("{a:e}");
    let (mant, exp_str) = sci.split_once('e').expect("LowerExp always has 'e'");
    let e: i32 = exp_str.parse().expect("LowerExp exponent is an integer");
    let s: String = mant.chars().filter(|c| *c != '.').collect();
    let k = s.len() as i32; // number of significant digits
    let n = e + 1; // value = s × 10^(n−k), 10^(k−1) ≤ s < 10^k

    if k <= n && n <= 21 {
        // Integer with trailing zeros: all digits, then n−k zeros.
        let mut out = s;
        out.push_str(&"0".repeat((n - k) as usize));
        out
    } else if 0 < n && n <= 21 {
        // Decimal point inside the digit run: n digits, '.', the rest.
        format!("{}.{}", &s[..n as usize], &s[n as usize..])
    } else if -6 < n && n <= 0 {
        // Leading "0." then (−n) zeros then all digits.
        format!("0.{}{}", "0".repeat((-n) as usize), s)
    } else {
        // Exponential form. Exponent digit is n−1, always signed.
        let exp = n - 1;
        let sign = if exp >= 0 { '+' } else { '-' };
        let mag = exp.abs();
        if k == 1 {
            format!("{s}e{sign}{mag}")
        } else {
            format!("{}.{}e{sign}{mag}", &s[..1], &s[1..])
        }
    }
}

impl JsHost {
    /// The `typeof` string for `v`.
    pub fn type_of(&self, v: &Value) -> &'static str {
        match v {
            Value::Undef => "undefined",
            Value::Bool(_) => "boolean",
            Value::Int(_) | Value::Float(_) => "number",
            Value::Str(_) => "string",
            Value::Obj(_) => match self.get(v) {
                Some(JsObj::Str(_)) => "string",
                Some(JsObj::Func(_))
                | Some(JsObj::BoundMethod { .. })
                | Some(JsObj::BoundFunc { .. })
                | Some(JsObj::Class(_)) => "function",
                // A Builtin is a callable (`Array`, `parseInt`, `Math.floor`) —
                // `typeof === "function"` — EXCEPT the non-callable namespace
                // objects (`Math`, `JSON`, `require('fs')`, …) which are "object".
                Some(JsObj::Builtin(n)) => {
                    const NON_CALLABLE_NS: &[&str] = &[
                        "Math",
                        "JSON",
                        "console",
                        "Reflect",
                        "process",
                        "Atomics",
                        "performance",
                        "fs",
                        "path",
                        "os",
                        "util",
                        "crypto",
                        "querystring",
                        "events",
                        "stream",
                        "timers",
                        "perf_hooks",
                        "async_hooks",
                        "diagnostics_channel",
                        "v8",
                        "dns",
                        "punycode",
                        "child_process",
                        "tty",
                        "url",
                        "zlib",
                        "string_decoder",
                        "assert",
                        "http",
                        "net",
                        "buffer",
                    ];
                    if NON_CALLABLE_NS.contains(&n.as_str()) {
                        "object"
                    } else {
                        "function"
                    }
                }
                Some(JsObj::Symbol { .. }) => "symbol",
                Some(JsObj::BigInt(_)) => "bigint",
                _ => "object", // arrays, objects, null, Map/Set, generators
            },
            _ => "object",
        }
    }

    /// JS truthiness: false / 0 / -0 / NaN / "" / null / undefined are falsy.
    pub fn truthy(&self, v: &Value) -> bool {
        match v {
            Value::Undef => false,
            Value::Bool(b) => *b,
            Value::Int(n) => *n != 0,
            Value::Float(f) => *f != 0.0 && !f.is_nan(),
            Value::Str(s) => !s.is_empty(),
            Value::Obj(_) => match self.get(v) {
                Some(JsObj::Str(s)) => !s.is_empty(),
                Some(JsObj::Null) => false,
                Some(JsObj::BigInt(b)) => !num_traits::Zero::is_zero(b),
                _ => true, // arrays, objects, functions
            },
            _ => true,
        }
    }

    /// Coerce to a number (`ToNumber`): the arithmetic-context conversion.
    pub fn to_number(&self, v: &Value) -> f64 {
        match v {
            Value::Undef => f64::NAN,
            Value::Bool(b) => {
                if *b {
                    1.0
                } else {
                    0.0
                }
            }
            Value::Int(n) => *n as f64,
            Value::Float(f) => *f,
            Value::Str(s) => str_to_number(s),
            Value::Obj(_) => match self.get(v) {
                Some(JsObj::Str(s)) => str_to_number(s),
                Some(JsObj::Null) => 0.0,
                Some(JsObj::BigInt(b)) => bigint_to_f64(b),
                Some(JsObj::Array(items)) => {
                    // [] -> 0, [x] -> ToNumber(x), else NaN.
                    if items.is_empty() {
                        0.0
                    } else if items.len() == 1 {
                        self.to_number(&items[0])
                    } else {
                        f64::NAN
                    }
                }
                _ => f64::NAN,
            },
            _ => f64::NAN,
        }
    }

    /// `String(v)` — the string-coercion form (raw, unquoted).
    pub fn str_of(&self, v: &Value) -> String {
        match v {
            Value::Undef => "undefined".into(),
            Value::Bool(b) => if *b { "true" } else { "false" }.into(),
            Value::Int(n) => n.to_string(),
            Value::Float(f) => fmt_number(*f),
            Value::Str(s) => (**s).clone(),
            Value::Obj(_) => match self.get(v) {
                Some(JsObj::Str(s)) => s.clone(),
                Some(JsObj::Null) => "null".into(),
                Some(JsObj::BigInt(b)) => b.to_string(),
                Some(JsObj::RegExp(r)) => format!("/{}/{}", r.source, r.flags),
                Some(JsObj::Array(items)) => {
                    // Array.prototype.toString: comma-join, null/undefined -> "".
                    let parts: Vec<String> = items
                        .iter()
                        .map(|x| match x {
                            Value::Undef => String::new(),
                            _ if self.is_null(x) => String::new(),
                            _ => self.str_of(x),
                        })
                        .collect();
                    parts.join(",")
                }
                Some(JsObj::Object(props)) => {
                    // A native `Buffer` stringifies to its decoded (utf-8)
                    // contents, matching `buf.toString()` — needed for `'' + buf`,
                    // template interpolation, and `data += chunk` (the pattern
                    // Express/body-parser use to read a request body).
                    if props.get("@@native").map(|t| self.str_of(t)).as_deref() == Some("Buffer") {
                        let bytes: Vec<u8> = match props.get("@@bytes").and_then(|b| self.get(b)) {
                            Some(JsObj::Array(items)) => {
                                items.iter().map(|x| self.to_number(x) as u8).collect()
                            }
                            _ => Vec::new(),
                        };
                        String::from_utf8_lossy(&bytes).into_owned()
                    } else if let Some(s) = self.error_to_string(v) {
                        s
                    } else {
                        "[object Object]".into()
                    }
                }
                Some(JsObj::Func(f)) => {
                    // A function built from runtime source (`new Function`,
                    // `vm.compileFunction`) retains the exact text V8 synthesizes
                    // for it, so `Function.prototype.toString` reports what Node
                    // reports. Ordinary functions carry no source here (the
                    // compiler keeps no spans), so they fall back to a placeholder.
                    if let Some(src) = self.fn_prop(v, "@@source") {
                        return self.str_of(&src);
                    }
                    let name = self
                        .funcs
                        .get(f.def_id)
                        .map(|d| d.name.clone())
                        .unwrap_or_default();
                    format!("function {name}() {{ [code] }}")
                }
                Some(JsObj::Builtin(n)) => format!("function {n}() {{ [native code] }}"),
                Some(JsObj::BoundMethod { .. }) | Some(JsObj::BoundFunc { .. }) => {
                    "function () { [native code] }".into()
                }
                Some(JsObj::Class(c)) => format!("class {} {{ }}", c.name),
                Some(JsObj::Symbol { desc, .. }) => {
                    // `String(sym)` is allowed (unlike implicit coercion) and yields
                    // `Symbol(desc)`.
                    match desc {
                        Some(d) => format!("Symbol({d})"),
                        None => "Symbol()".into(),
                    }
                }
                _ => "[object Object]".into(),
            },
            _ => "[object Object]".into(),
        }
    }

    /// The `Symbol.toStringTag` string `util.inspect` renders as a `[Tag]`
    /// prefix. V8 suppresses the tag when it is an OWN ENUMERABLE property,
    /// because it is then already listed as a `Symbol(Symbol.toStringTag): …`
    /// entry and showing it twice would be wrong.
    ///
    /// Only a DATA property is seen. A tag supplied by a prototype getter
    /// (`class C { get [Symbol.toStringTag]() { … } }`) would need a JS call,
    /// which cannot run under the host borrow `inspect` holds — such an object
    /// prints without the prefix.
    fn inspect_tag(&self, v: &Value) -> Option<String> {
        let own = matches!(self.get(v), Some(JsObj::Object(p)) if p.contains_key("@@toStringTag"));
        if own && self.prop_attrs(v, "@@toStringTag").enumerable {
            return None;
        }
        let t = lookup_chain(self, v, "@@toStringTag")?;
        self.as_str(&t)
    }

    /// `console.log`-style rendering of a top-level argument: bare strings print
    /// raw; everything else uses `inspect`.
    pub fn console_format(&self, v: &Value) -> String {
        match v {
            Value::Str(_) => self.str_of(v),
            Value::Obj(_) if matches!(self.get(v), Some(JsObj::Str(_))) => self.str_of(v),
            _ => self.inspect(v),
        }
    }

    /// `util.inspect`-style rendering (nested; strings quoted).
    pub fn inspect(&self, v: &Value) -> String {
        self.inspect_lvl(v, 0)
    }

    /// `util.inspect` at a given indentation level (drives array multi-line
    /// grouping and nested indentation).
    fn inspect_lvl(&self, v: &Value, indent: usize) -> String {
        match v {
            Value::Undef => "undefined".into(),
            Value::Bool(b) => if *b { "true" } else { "false" }.into(),
            Value::Int(n) => n.to_string(),
            // `util.inspect` distinguishes negative zero; `String(-0)` does not.
            Value::Float(f) if *f == 0.0 && f.is_sign_negative() => "-0".into(),
            Value::Float(f) => fmt_number(*f),
            Value::Str(s) => quote_str(s),
            Value::Obj(_) => match self.get(v) {
                Some(JsObj::Str(s)) => quote_str(s),
                Some(JsObj::Null) => "null".into(),
                // `util.inspect` renders a bigint with the `n` suffix, a regex bare.
                Some(JsObj::BigInt(b)) => format!("{b}n"),
                Some(JsObj::RegExp(r)) => format!("/{}/{}", r.source, r.flags),
                Some(JsObj::Array(items)) => {
                    // Own enumerable non-index string props (e.g. a `str.match(re)`
                    // result's `index`/`input`/`groups`, or a user-assigned
                    // `arr.foo`) render after the elements, as `key: value`.
                    let prop_keys: Vec<String> = self
                        .fn_prop_keys(v)
                        .into_iter()
                        .filter(|k| {
                            !k.starts_with("@@")
                                && !k.starts_with('#')
                                && self.prop_attrs(v, k).enumerable
                        })
                        .collect();
                    // An own enumerable SYMBOL-keyed property renders after the
                    // string keys as `Symbol(desc): value`, as it does on an
                    // object receiver.
                    let sym_entries = self.own_symbol_entries(v);
                    if items.is_empty() && prop_keys.is_empty() && sym_entries.is_empty() {
                        return "[]".into();
                    }
                    // Node's default inspect depth is 2 (root = depth 0); deeper
                    // nesting collapses to `[Array]`. indent grows by 2 per level.
                    if indent > 2 * inspect_max_depth() {
                        return "[Array]".into();
                    }
                    let mut inner: Vec<String> = items
                        .iter()
                        .map(|x| self.inspect_lvl(x, indent + 2))
                        .collect();
                    let has_props = !prop_keys.is_empty() || !sym_entries.is_empty();
                    for k in &prop_keys {
                        let val = self.fn_prop(v, k).unwrap_or(Value::Undef);
                        inner.push(format!(
                            "{}: {}",
                            fmt_key(k),
                            self.inspect_lvl(&val, indent + 2)
                        ));
                    }
                    for (k, val) in &sym_entries {
                        let label = match self.symbol_of_key(k) {
                            Some(s) => self.inspect(&s),
                            None => continue,
                        };
                        inner.push(format!("{label}: {}", self.inspect_lvl(val, indent + 2)));
                    }
                    self.render_array(&inner, items, indent, has_props)
                }
                // A `Buffer` renders as `<Buffer 01 02 03>` — hex bytes, capped
                // at 50 with a `... N more byte(s)` tail, exactly as
                // `util.inspect` does. Without this a `console.log(buf)` (the
                // single most common thing anyone does with a Buffer) printed
                // the internal `{ length, byteLength, … }` bookkeeping.
                Some(JsObj::Object(props))
                    if props.get("@@native").map(|t| self.str_of(t)).as_deref()
                        == Some("Buffer") =>
                {
                    let bytes: Vec<u8> = match props.get("@@bytes").and_then(|b| self.get(b)) {
                        Some(JsObj::Array(items)) => {
                            items.iter().map(|x| self.to_number(x) as u8).collect()
                        }
                        _ => Vec::new(),
                    };
                    const MAX: usize = 50;
                    let shown: Vec<String> =
                        bytes.iter().take(MAX).map(|b| format!("{b:02x}")).collect();
                    let mut out = format!("<Buffer {}", shown.join(" "));
                    if bytes.len() > MAX {
                        let more = bytes.len() - MAX;
                        let unit = if more == 1 { "byte" } else { "bytes" };
                        out.push_str(&format!(" ... {more} more {unit}"));
                    }
                    out.push('>');
                    out
                }
                // An Error inspects as its `.stack` — never as an object literal
                // exposing the internal `message`/`stack` slots. Any own property
                // a script added beyond those follows in braces, as V8 renders
                // it: `Error: x\n    at … { code: 'C' }`.
                Some(JsObj::Object(_)) if self.error_to_string(v).is_some() => {
                    let stack = lookup_chain(self, v, "stack")
                        .map(|s| self.str_of(&s))
                        .unwrap_or_else(|| self.error_to_string(v).unwrap_or_default());
                    let extra: Vec<String> = self
                        .own_enum_key_names(v)
                        .into_iter()
                        .filter(|k| k != "name")
                        .map(|k| {
                            let val = self.fn_prop(v, &k).unwrap_or_else(|| match self.get(v) {
                                Some(JsObj::Object(p)) => {
                                    p.get(&k).cloned().unwrap_or(Value::Undef)
                                }
                                _ => Value::Undef,
                            });
                            format!("{}: {}", fmt_key(&k), self.inspect_lvl(&val, indent + 2))
                        })
                        .collect();
                    if extra.is_empty() {
                        stack
                    } else {
                        format!("{stack} {{ {} }}", extra.join(", "))
                    }
                }
                Some(JsObj::Object(props)) => {
                    // Instances print with their constructor name as a prefix
                    // (`C { x: 1 }`); plain objects have none; a null-prototype
                    // object (e.g. an `Object.groupBy` result) is tagged
                    // `[Object: null prototype]`.
                    let ctor = match self.ctor_name(v) {
                        n if n.is_empty() => "Object".to_string(),
                        n => n,
                    };
                    let plain_prefix = if ctor == "Object" {
                        String::new()
                    } else {
                        format!("{ctor} ")
                    };
                    let prefix = if self.has_null_proto(v) {
                        "[Object: null prototype] ".to_string()
                    } else {
                        // An inherited `Symbol.toStringTag` shows as `Ctor [Tag] `.
                        match self.inspect_tag(v) {
                            Some(t) if t != ctor => format!("{ctor} [{t}] "),
                            _ => plain_prefix.clone(),
                        }
                    };
                    // Skip node-js's internal slots (`@@native`, `@@bytes`, …) and
                    // private class fields; a real symbol-keyed own property is a
                    // visible one and renders as `Symbol(desc): value`.
                    let mut shown: Vec<(String, &Value)> = props
                        .iter()
                        .filter(|(k, _)| !k.starts_with("@@") && !k.starts_with('#'))
                        .map(|(k, val)| (fmt_key(k), val))
                        .collect();
                    shown.extend(props.iter().filter_map(|(k, val)| {
                        let sym = self.symbol_of_key(k)?;
                        self.prop_attrs(v, k)
                            .enumerable
                            .then(|| (self.inspect(&sym), val))
                    }));
                    if shown.is_empty() {
                        return format!("{prefix}{{}}");
                    }
                    // Depth limit (Node default 2): deeper objects collapse to
                    // `[Object]` (or `[ClassName]` for a named instance).
                    if indent > 2 * inspect_max_depth() {
                        return if self.has_null_proto(v) {
                            // Already bracketed (`[Object: null prototype]`).
                            prefix.trim_end().to_string()
                        } else if plain_prefix.is_empty() {
                            "[Object]".into()
                        } else {
                            format!("[{}]", plain_prefix.trim_end())
                        };
                    }
                    let inner: Vec<String> = shown
                        .iter()
                        .map(|(k, val)| format!("{k}: {}", self.inspect_lvl(val, indent + 2)))
                        .collect();
                    self.render_object(&inner, &prefix, indent)
                }
                Some(JsObj::Symbol { desc, .. }) => match desc {
                    Some(d) => format!("Symbol({d})"),
                    None => "Symbol()".into(),
                },
                Some(JsObj::Class(c)) => {
                    let base = if c.parent.is_some() {
                        let pname = c
                            .parent
                            .as_ref()
                            .map(|p| self.callable_name(p))
                            .unwrap_or_default();
                        format!("[class {} extends {}]", c.name, pname)
                    } else {
                        format!("[class {}]", c.name)
                    };
                    self.with_callable_props(v, base, indent)
                }
                Some(JsObj::Map { entries, .. }) => {
                    if entries.is_empty() {
                        return "Map(0) {}".into();
                    }
                    let inner: Vec<String> = entries
                        .values()
                        .map(|(k, val)| format!("{} => {}", self.inspect(k), self.inspect(val)))
                        .collect();
                    format!("Map({}) {{ {} }}", entries.len(), inner.join(", "))
                }
                Some(JsObj::Set { entries, .. }) => {
                    if entries.is_empty() {
                        return "Set(0) {}".into();
                    }
                    let inner: Vec<String> = entries.values().map(|v| self.inspect(v)).collect();
                    format!("Set({}) {{ {} }}", entries.len(), inner.join(", "))
                }
                Some(JsObj::Generator { .. }) => "Object [Generator] {}".into(),
                Some(JsObj::Promise { id }) => match self.promises.get(*id as usize) {
                    Some(c) => match c.state {
                        PromiseState::Pending => "Promise { <pending> }".into(),
                        PromiseState::Fulfilled => {
                            format!("Promise {{ {} }}", self.inspect(&c.value))
                        }
                        PromiseState::Rejected => {
                            format!("Promise {{ <rejected> {} }}", self.inspect(&c.value))
                        }
                    },
                    None => "Promise { <pending> }".into(),
                },
                Some(JsObj::Func(_)) => {
                    // `callable_name`, not the FuncDef name: an anonymous
                    // function expression gets its name by inference from the
                    // binding it initialises (`const f = function(){}`), and
                    // that lands as an own `name` property.
                    let name = self.callable_name(v);
                    let base = if name.is_empty() {
                        "[Function (anonymous)]".to_string()
                    } else {
                        format!("[Function: {name}]")
                    };
                    self.with_callable_props(v, base, indent)
                }
                Some(JsObj::Builtin(n)) => {
                    let short = n.rsplit('.').next().unwrap_or(n);
                    format!("[Function: {short}]")
                }
                Some(JsObj::BoundMethod { .. }) => "[Function (anonymous)]".into(),
                Some(JsObj::BoundFunc { target, .. }) => {
                    let n = self.callable_name(target);
                    if n.is_empty() {
                        "[Function: bound ]".into()
                    } else {
                        format!("[Function: bound {n}]")
                    }
                }
                _ => "undefined".into(),
            },
            _ => "undefined".into(),
        }
    }

    /// Append a callable's own enumerable properties to its `[Function: f]` /
    /// `[class C]` base, the way `util.inspect` does: `[Function: f] { a: 1 }`.
    /// A callable with none renders as the bare base.
    fn with_callable_props(&self, v: &Value, base: String, indent: usize) -> String {
        let mut inner: Vec<String> = self
            .own_enum_key_names(v)
            .into_iter()
            .map(|k| {
                let val = self.fn_prop(v, &k).unwrap_or(Value::Undef);
                format!("{}: {}", fmt_key(&k), self.inspect_lvl(&val, indent + 2))
            })
            .collect();
        for (k, val) in self.own_symbol_entries(v) {
            if let Some(sym) = self.symbol_of_key(&k) {
                inner.push(format!(
                    "{}: {}",
                    self.inspect(&sym),
                    self.inspect_lvl(&val, indent + 2)
                ));
            }
        }
        if inner.is_empty() {
            return base;
        }
        self.render_object(&inner, &format!("{base} "), indent)
    }

    /// Render a non-empty array's already-formatted element strings, applying
    /// Node's `util.inspect` layout: a single line when it fits, else a multi-line
    /// grid via `groupArrayElements` (for >6 entries), else one element per line.
    /// `values` is the raw element list (drives numeric right-alignment); `indent`
    /// is the array's own indentation level.
    fn render_array(
        &self,
        output: &[String],
        values: &[Value],
        indent: usize,
        has_props: bool,
    ) -> String {
        // Group array elements together if the array has more than six entries.
        // Arrays carrying extra own props (`index`/`input`/… on a match result)
        // are never grid-grouped — Node lays those out plainly.
        let entries = output.len();
        let (lines, grouped) = if entries > 6 && !has_props {
            group_array_elements(self, output, values, indent)
        } else {
            (output.to_vec(), false)
        };
        // If no grouping happened, try to line everything up on a single line.
        if !grouped {
            // start = output.length + indentationLvl + braces[0].len(1) + base(0) + 10
            let start = output.len() + indent + 1 + 10;
            if is_below_break_length(output, start) {
                return format!("[ {} ]", output.join(", "));
            }
        }
        // Otherwise: one (grouped or single) entry per line, indented by indent+2.
        let pad = " ".repeat(indent);
        let sep = format!(",\n{pad}  ");
        format!("[\n{pad}  {}\n{pad}]", lines.join(&sep))
    }

    /// Render a non-empty object's already-formatted `key: value` strings with
    /// Node's `util.inspect` layout: a single line when it fits `breakLength`,
    /// else one property per line indented by `indent + 2`. `prefix` is the
    /// constructor/`[Object: null prototype]` tag (with trailing space) or empty.
    /// Mirrors `render_array`'s break decision. (Node's `compact` depth gate is a
    /// no-op at `console.log`'s default depth of 2, so only length matters here.)
    fn render_object(&self, output: &[String], prefix: &str, indent: usize) -> String {
        // start = output.length + indentationLvl + braces[0].len + base(0) + 10.
        // For a tagged object Node folds the tag into `braces[0]` (e.g.
        // `"Point {"`, `"[Object: null prototype] {"`), so its length is the
        // prefix (which carries the trailing space) plus the `{`.
        let braces0 = prefix.chars().count() + 1;
        let start = output.len() + indent + braces0 + 10;
        if is_below_break_length(output, start) {
            return format!("{prefix}{{ {} }}", output.join(", "));
        }
        let pad = " ".repeat(indent);
        let sep = format!(",\n{pad}  ");
        format!("{prefix}{{\n{pad}  {}\n{pad}}}", output.join(&sep))
    }

    /// The `.name` of any callable (function/class/builtin/bound).
    pub fn callable_name(&self, v: &Value) -> String {
        // A user-set `.name` own property wins.
        if let Some(n) = self.fn_prop(v, "name") {
            return self.str_of(&n);
        }
        match self.get(v) {
            Some(JsObj::Func(f)) => self
                .funcs
                .get(f.def_id)
                .map(|d| d.name.clone())
                .unwrap_or_default(),
            Some(JsObj::Class(c)) => c.name.clone(),
            Some(JsObj::Builtin(n)) => n.rsplit('.').next().unwrap_or(n).to_string(),
            Some(JsObj::BoundFunc { target, .. }) => {
                format!("bound {}", self.callable_name(target))
            }
            _ => String::new(),
        }
    }

    // ── equality / comparison / arithmetic (numeric-hook + builtin paths) ──

    /// Strict equality (`===`): same type and same value, no coercion.
    pub fn strict_eq(&self, a: &Value, b: &Value) -> bool {
        match (a, b) {
            (Value::Undef, Value::Undef) => true,
            (Value::Bool(x), Value::Bool(y)) => x == y,
            (Value::Str(x), Value::Str(y)) => x == y,
            _ => {
                // Numbers (NaN !== NaN, +0 === -0).
                let an = matches!(a, Value::Int(_) | Value::Float(_));
                let bn = matches!(b, Value::Int(_) | Value::Float(_));
                if an && bn {
                    let x = self.to_number(a);
                    let y = self.to_number(b);
                    return x == y;
                }
                // BigInt === BigInt compares by value (each literal is a distinct
                // heap cell, so reference identity would be wrong). BigInt is never
                // `===` a Number (different types).
                if let (Some(x), Some(y)) = (self.as_bigint(a), self.as_bigint(b)) {
                    return x == y;
                }
                // Heap values.
                if let (Some(sa), Some(sb)) = (self.as_str(a), self.as_str(b)) {
                    return sa == sb;
                }
                let na = self.is_null(a);
                let nb = self.is_null(b);
                if na || nb {
                    return na && nb;
                }
                // A builtin namespace/constructor/prototype is a SINGLETON in JS
                // (`Math === Math`, `Array.prototype === Array.prototype`), but
                // every bare reference here allocates a fresh handle, so compare
                // those by name rather than by heap index.
                if let (Some(JsObj::Builtin(x)), Some(JsObj::Builtin(y))) =
                    (self.get(a), self.get(b))
                {
                    return x == y;
                }
                // Reference identity for arrays/objects/functions.
                matches!((a, b), (Value::Obj(x), Value::Obj(y)) if x == y)
            }
        }
    }

    /// Whether `v` is `null` or `undefined`.
    pub fn is_nullish(&self, v: &Value) -> bool {
        matches!(v, Value::Undef) || self.is_null(v)
    }

    /// The ECMAScript "loose type" of `v` for the `==` algorithm: `"number"`,
    /// `"string"` (primitive or heap string), `"boolean"`, `"undefined"`,
    /// `"null"`, or `"object"` (array / plain object / function).
    fn js_type(&self, v: &Value) -> &'static str {
        match v {
            Value::Undef => "undefined",
            Value::Bool(_) => "boolean",
            Value::Int(_) | Value::Float(_) => "number",
            Value::Str(_) => "string",
            Value::Obj(_) => match self.get(v) {
                Some(JsObj::Str(_)) => "string",
                Some(JsObj::Null) => "null",
                Some(JsObj::BigInt(_)) => "bigint",
                _ => "object",
            },
            _ => "object",
        }
    }

    /// Loose equality (`==`) following the ECMAScript Abstract Equality Comparison.
    /// Objects reduce via `ToPrimitive` (which for our heap objects is always their
    /// string `toString`), so `[0] == "0"` is `true` (string compare of `"0"`) but
    /// `[0] == ""` is `false` — never a number coercion of the object.
    pub fn loose_eq(&self, a: &Value, b: &Value) -> bool {
        // Same type: identical to `===` (number==number, string==string, etc.).
        if self.strict_eq(a, b) {
            return true;
        }
        let ta = self.js_type(a);
        let tb = self.js_type(b);
        // null and undefined are loosely equal only to each other.
        if self.is_nullish(a) || self.is_nullish(b) {
            return self.is_nullish(a) && self.is_nullish(b);
        }
        // BigInt ⇄ (Number | String | Boolean | Object): compare mathematical
        // values (both-BigInt was already settled by the `strict_eq` above).
        if ta == "bigint" || tb == "bigint" {
            return self.bigint_loose_eq(a, b);
        }
        if ta == tb {
            // Same type but not strict-equal (and not nullish) ⇒ not equal.
            return false;
        }
        // number ⇄ string: compare as numbers.
        if (ta == "number" && tb == "string") || (ta == "string" && tb == "number") {
            return self.to_number(a) == self.to_number(b);
        }
        // boolean side coerces to number, then recompares.
        if ta == "boolean" {
            return self.loose_eq(&Value::Float(self.to_number(a)), b);
        }
        if tb == "boolean" {
            return self.loose_eq(a, &Value::Float(self.to_number(b)));
        }
        // object ⇄ (number|string): ToPrimitive the object (→ its string form),
        // then recompare as string==string or number==string.
        if ta == "object" && (tb == "number" || tb == "string") {
            let pa = self.str_of(a);
            return if tb == "string" {
                pa == self.str_of(b)
            } else {
                str_to_number(&pa) == self.to_number(b)
            };
        }
        if tb == "object" && (ta == "number" || ta == "string") {
            let pb = self.str_of(b);
            return if ta == "string" {
                self.str_of(a) == pb
            } else {
                self.to_number(a) == str_to_number(&pb)
            };
        }
        false
    }

    /// The numeric-hook arithmetic/relational fallback for non-native operands
    /// (called by fusevm when at least one operand isn't `Int`/`Float`).
    pub fn arith(&mut self, op: NumOp, a: &Value, b: &Value) -> Result<Value, String> {
        use NumOp::*;
        match op {
            Add => {
                // `+`: if either operand is a string, concatenate string forms;
                // otherwise numeric addition.
                let a_str = self.prefers_string(a);
                let b_str = self.prefers_string(b);
                if a_str || b_str {
                    // String concatenation wins even with a bigint operand
                    // (`1n + "x"` → `"1x"`).
                    let s = format!("{}{}", self.str_of(a), self.str_of(b));
                    Ok(self.new_str(s))
                } else if self.is_bigint_val(a) || self.is_bigint_val(b) {
                    self.bigint_arith(op, a, b)
                } else {
                    Ok(Value::Float(self.to_number(a) + self.to_number(b)))
                }
            }
            Sub | Mul | Div | Mod | Pow if self.is_bigint_val(a) || self.is_bigint_val(b) => {
                self.bigint_arith(op, a, b)
            }
            Sub => Ok(Value::Float(self.to_number(a) - self.to_number(b))),
            Mul => Ok(Value::Float(self.to_number(a) * self.to_number(b))),
            Div => Ok(Value::Float(self.to_number(a) / self.to_number(b))),
            Mod => Ok(Value::Float(js_mod(self.to_number(a), self.to_number(b)))),
            Pow => Ok(Value::Float(self.to_number(a).powf(self.to_number(b)))),
            Neg if self.is_bigint_val(a) => self.bigint_arith(op, a, b),
            Neg => Ok(Value::Float(-self.to_number(a))),
            Lt | Le | Gt | Ge => Ok(Value::Bool(self.relational(op, a, b))),
            Eq => Ok(Value::Bool(self.loose_eq(a, b))),
            Ne => Ok(Value::Bool(!self.loose_eq(a, b))),
        }
    }

    /// Whether `v`'s primitive (`ToPrimitive` with the default hint) is a string,
    /// which drives `+` toward concatenation. Primitive strings qualify, and so
    /// do heap objects whose default `ToPrimitive` is their (string) `toString`:
    /// arrays (`[1,2,3]+3 → "1,2,33"`), plain objects (`{}+[] → "[object Object]"`),
    /// and functions. `null`/`undefined`/`boolean`/`number` do not.
    fn prefers_string(&self, v: &Value) -> bool {
        match v {
            Value::Str(_) => true,
            // A BigInt's `ToPrimitive` is the bigint itself (numeric), NOT a string,
            // so `1n + 2n` is bigint addition, not concatenation. `null` has no
            // string primitive either.
            Value::Obj(_) => !matches!(
                self.get(v),
                Some(JsObj::Null) | Some(JsObj::BigInt(_)) | None
            ),
            _ => false,
        }
    }

    /// Relational comparison (`< <= > >=`) with JS coercion: string/string is
    /// lexicographic, otherwise numeric (NaN yields false).
    fn relational(&self, op: NumOp, a: &Value, b: &Value) -> bool {
        use std::cmp::Ordering;
        let ord = if let (Some(x), Some(y)) = (self.as_bigint(a), self.as_bigint(b)) {
            // BigInt < BigInt: exact (no f64 precision loss for large magnitudes).
            x.cmp(&y)
        } else if let (Some(x), Some(y)) = (self.as_str(a), self.as_str(b)) {
            x.cmp(&y)
        } else {
            let x = self.to_number(a);
            let y = self.to_number(b);
            match x.partial_cmp(&y) {
                Some(o) => o,
                None => return false, // NaN operand
            }
        };
        match op {
            NumOp::Lt => ord == Ordering::Less,
            NumOp::Le => ord != Ordering::Greater,
            NumOp::Gt => ord == Ordering::Greater,
            NumOp::Ge => ord != Ordering::Less,
            _ => false,
        }
    }

    /// Bitwise/shift ops with JS ToInt32/ToUint32 semantics — or true
    /// arbitrary-width BigInt bitwise when both operands are BigInt (mixing a
    /// BigInt with a Number throws, matching Node).
    pub fn bitwise(&mut self, tag: i64, a: &Value, b: &Value) -> Result<Value, String> {
        if self.is_bigint_val(a) || self.is_bigint_val(b) {
            return self.bigint_bitwise(tag, a, b);
        }
        let x = to_int32(self.to_number(a));
        let y = to_int32(self.to_number(b));
        let r: i64 = match tag {
            binop::BITAND => (x & y) as i64,
            binop::BITOR => (x | y) as i64,
            binop::BITXOR => (x ^ y) as i64,
            binop::SHL => (x.wrapping_shl((y as u32) & 31)) as i64,
            binop::SHR => (x >> ((y as u32) & 31)) as i64,
            binop::USHR => (to_uint32(self.to_number(a)) >> ((y as u32) & 31)) as i64,
            _ => 0,
        };
        Ok(Value::Float(r as f64))
    }

    // ── BigInt operations ────────────────────────────────────────────────────
    /// Whether `v` is a heap `BigInt`.
    pub fn is_bigint_val(&self, v: &Value) -> bool {
        matches!(self.get(v), Some(JsObj::BigInt(_)))
    }
    /// The `BigInt` value of `v` (a heap bigint), else `None`.
    pub fn as_bigint(&self, v: &Value) -> Option<num_bigint::BigInt> {
        match self.get(v) {
            Some(JsObj::BigInt(b)) => Some(b.clone()),
            _ => None,
        }
    }
    /// Allocate a heap `BigInt`.
    pub fn new_bigint(&mut self, b: num_bigint::BigInt) -> Value {
        self.alloc(JsObj::BigInt(b))
    }

    /// BigInt arithmetic (`+ - * / % **`, unary `-`). Requires BOTH operands to be
    /// BigInt for a binary op; mixing a BigInt with a Number throws the exact Node
    /// `TypeError` (a string operand is handled as concatenation before we get
    /// here). Division/`%` truncate toward zero; `**` needs a non-negative
    /// exponent.
    fn bigint_arith(&mut self, op: NumOp, a: &Value, b: &Value) -> Result<Value, String> {
        use num_traits::{Signed, Zero};
        use NumOp::*;
        if op == Neg {
            let x = self.as_bigint(a).expect("bigint_arith Neg on non-bigint");
            return Ok(self.new_bigint(-x));
        }
        let (x, y) = match (self.as_bigint(a), self.as_bigint(b)) {
            (Some(x), Some(y)) => (x, y),
            // Exactly one side is a BigInt → the other is a Number/Boolean: illegal.
            _ => {
                return Err(type_error(
                    "Cannot mix BigInt and other types, use explicit conversions",
                ))
            }
        };
        let r = match op {
            Add => x + y,
            Sub => x - y,
            Mul => x * y,
            Div => {
                if y.is_zero() {
                    return Err("RangeError: Division by zero".into());
                }
                x / y // truncates toward zero (matches JS BigInt division)
            }
            Mod => {
                if y.is_zero() {
                    return Err("RangeError: Division by zero".into());
                }
                x % y // sign follows the dividend (truncated), like JS
            }
            Pow => {
                if y.is_negative() {
                    return Err("RangeError: Exponent must be positive".into());
                }
                let exp = num_traits::ToPrimitive::to_u32(&y)
                    .ok_or_else(|| "RangeError: Maximum BigInt size exceeded".to_string())?;
                num_traits::Pow::pow(x, exp)
            }
            _ => return Err(type_error("unsupported BigInt operation")),
        };
        Ok(self.new_bigint(r))
    }

    /// BigInt bitwise (`& | ^ << >>`); `>>>` has no BigInt form. Both operands must
    /// be BigInt (mixing throws).
    fn bigint_bitwise(&mut self, tag: i64, a: &Value, b: &Value) -> Result<Value, String> {
        let (x, y) = match (self.as_bigint(a), self.as_bigint(b)) {
            (Some(x), Some(y)) => (x, y),
            _ => {
                return Err(type_error(
                    "Cannot mix BigInt and other types, use explicit conversions",
                ))
            }
        };
        let r = match tag {
            binop::BITAND => x & y,
            binop::BITOR => x | y,
            binop::BITXOR => x ^ y,
            binop::SHL => {
                let n = num_traits::ToPrimitive::to_i64(&y).unwrap_or(0);
                if n >= 0 {
                    x << (n as usize)
                } else {
                    x >> ((-n) as usize)
                }
            }
            binop::SHR => {
                let n = num_traits::ToPrimitive::to_i64(&y).unwrap_or(0);
                if n >= 0 {
                    x >> (n as usize)
                } else {
                    x << ((-n) as usize)
                }
            }
            binop::USHR => {
                return Err(type_error(
                    "BigInts have no unsigned right shift, use >> instead",
                ))
            }
            _ => return Err(type_error("unsupported BigInt operation")),
        };
        Ok(self.new_bigint(r))
    }

    /// BigInt ⇄ (Number | Boolean | String | Object) loose equality (`==`). Both
    /// being BigInt was already handled by `strict_eq`.
    fn bigint_loose_eq(&self, a: &Value, b: &Value) -> bool {
        // Order so `big` is the BigInt side and `other` the counterpart.
        let (big, other) = match (self.as_bigint(a), self.as_bigint(b)) {
            (Some(x), _) => (x, b),
            (_, Some(y)) => (y, a),
            _ => return false,
        };
        match other {
            Value::Bool(bo) => big == num_bigint::BigInt::from(*bo as i64),
            Value::Int(n) => big == num_bigint::BigInt::from(*n),
            Value::Float(f) => {
                // Equal only when the float is an integer with the same value.
                if !f.is_finite() || f.fract() != 0.0 {
                    return false;
                }
                bigint_to_f64(&big) == *f
            }
            Value::Str(s) => match parse_bigint_str(s) {
                Some(bs) => big == bs,
                None => false,
            },
            Value::Obj(_) => match self.get(other) {
                // A heap string parses like a primitive string.
                Some(JsObj::Str(s)) => parse_bigint_str(s).map(|bs| big == bs).unwrap_or(false),
                _ => {
                    // Other objects reduce via ToPrimitive (their string form).
                    let s = self.str_of(other);
                    parse_bigint_str(&s).map(|bs| big == bs).unwrap_or(false)
                }
            },
            _ => false,
        }
    }
}

/// Parse a string to a BigInt under JS `StringToBigInt` rules: trimmed, empty →
/// `0n`, decimal or `0x`/`0o`/`0b` prefixed; any junk → `None`.
pub fn parse_bigint_str(s: &str) -> Option<num_bigint::BigInt> {
    let t = s.trim();
    if t.is_empty() {
        return Some(num_bigint::BigInt::from(0));
    }
    let (radix, digits) = if let Some(h) = t.strip_prefix("0x").or_else(|| t.strip_prefix("0X")) {
        (16, h)
    } else if let Some(o) = t.strip_prefix("0o").or_else(|| t.strip_prefix("0O")) {
        (8, o)
    } else if let Some(bb) = t.strip_prefix("0b").or_else(|| t.strip_prefix("0B")) {
        (2, bb)
    } else {
        (10, t)
    };
    num_bigint::BigInt::parse_bytes(digits.as_bytes(), radix)
}

/// Coerce a BigInt to `f64` (for `Number(bigint)` and mixed relational compares);
/// out-of-range magnitudes become ±Infinity, matching Node.
fn bigint_to_f64(b: &num_bigint::BigInt) -> f64 {
    num_traits::ToPrimitive::to_f64(b).unwrap_or_else(|| {
        if num_traits::Signed::is_negative(b) {
            f64::NEG_INFINITY
        } else {
            f64::INFINITY
        }
    })
}

/// JS `%` remainder (sign follows the dividend; matches `f64::rem`).
fn js_mod(a: f64, b: f64) -> f64 {
    a % b
}

thread_local! {
    /// The active `util.inspect` `depth` (nesting levels shown before collapsing
    /// to `[Object]`/`[Array]`). Node's default is 2; `util.inspect(v,{depth:N})`
    /// overrides it for one call, `console.log`/`util.format` use the default.
    static INSPECT_MAX_DEPTH: std::cell::Cell<usize> = const { std::cell::Cell::new(2) };
}

/// Set the `util.inspect` depth for the next render (restore to 2 after).
pub fn set_inspect_max_depth(d: usize) {
    INSPECT_MAX_DEPTH.with(|c| c.set(d));
}
fn inspect_max_depth() -> usize {
    INSPECT_MAX_DEPTH.with(|c| c.get())
}

fn to_int32(f: f64) -> i32 {
    if !f.is_finite() {
        return 0;
    }
    let n = f.trunc();
    (n as i64 as u32) as i32
}
fn to_uint32(f: f64) -> u32 {
    if !f.is_finite() {
        return 0;
    }
    f.trunc() as i64 as u32
}

/// Parse a string in numeric context (`ToNumber`): trimmed, empty -> 0.
fn str_to_number(s: &str) -> f64 {
    let t = s.trim();
    if t.is_empty() {
        return 0.0;
    }
    if let Some(hex) = t.strip_prefix("0x").or_else(|| t.strip_prefix("0X")) {
        return i64::from_str_radix(hex, 16)
            .map(|n| n as f64)
            .unwrap_or(f64::NAN);
    }
    if let Some(oct) = t.strip_prefix("0o").or_else(|| t.strip_prefix("0O")) {
        return i64::from_str_radix(oct, 8)
            .map(|n| n as f64)
            .unwrap_or(f64::NAN);
    }
    if let Some(bin) = t.strip_prefix("0b").or_else(|| t.strip_prefix("0B")) {
        return i64::from_str_radix(bin, 2)
            .map(|n| n as f64)
            .unwrap_or(f64::NAN);
    }
    match t {
        "Infinity" | "+Infinity" => f64::INFINITY,
        "-Infinity" => f64::NEG_INFINITY,
        _ => t.parse::<f64>().unwrap_or(f64::NAN),
    }
}

/// `util.inspect` break length (the width past which entries wrap). Node's default.
const BREAK_LENGTH: usize = 80;
/// Node's default `compact` setting (the `compact * 4` column cap term).
const COMPACT: usize = 3;

/// Whether `output` fits on a single line — a faithful port of Node's
/// `isBelowBreakLength` (no colors, no `base`). `start` is the caller's seed
/// length (braces + indentation + slack).
fn is_below_break_length(output: &[String], start: usize) -> bool {
    let mut total = output.len() + start;
    if total + output.len() > BREAK_LENGTH {
        return false;
    }
    for o in output {
        if o.contains('\n') {
            return false;
        }
        total += o.chars().count();
        if total > BREAK_LENGTH {
            return false;
        }
    }
    true
}

/// Faithful port of Node's `util.inspect` `groupArrayElements`: lay out the
/// already-formatted element strings into an aligned multi-column grid. Returns
/// `(lines, grouped)` — `grouped` is false when Node would leave the output
/// ungrouped (so the caller falls back to single-line / one-per-line).
fn group_array_elements(
    host: &JsHost,
    output: &[String],
    values: &[Value],
    indentation_lvl: usize,
) -> (Vec<String>, bool) {
    let separator_space = 2usize; // ", " between entries
    let output_length = output.len();
    let data_len: Vec<usize> = output.iter().map(|o| o.chars().count()).collect();
    let mut total_length = 0usize;
    let mut max_length = 0usize;
    for &len in &data_len {
        total_length += len + separator_space;
        if len > max_length {
            max_length = len;
        }
    }
    let actual_max = max_length + separator_space;
    // Only group when ≥3 entries fit across AND the entries aren't wildly uneven.
    if !(actual_max * 3 + indentation_lvl < BREAK_LENGTH
        && (total_length as f64 / actual_max as f64 > 5.0 || max_length <= 6))
    {
        return (output.to_vec(), false);
    }
    let approx_char_heights = 2.5f64;
    let average_bias = (actual_max as f64 - total_length as f64 / output_length as f64).sqrt();
    let biased_max = (actual_max as f64 - 3.0 - average_bias).max(1.0);
    // Ideally a square grid; capped by break length, compact*4, and 15 columns.
    let columns = [
        ((approx_char_heights * biased_max * output_length as f64).sqrt() / biased_max).round()
            as i64,
        ((BREAK_LENGTH - indentation_lvl) as f64 / actual_max as f64).floor() as i64,
        (COMPACT * 4) as i64,
        15,
    ]
    .into_iter()
    .min()
    .unwrap();
    if columns <= 1 {
        return (output.to_vec(), false);
    }
    let columns = columns as usize;
    // The widest entry (plus separator) in each column.
    let mut max_line_length = vec![0usize; columns];
    for (i, slot) in max_line_length.iter_mut().enumerate() {
        let mut line_length = 0;
        let mut j = i;
        while j < output_length {
            if data_len[j] > line_length {
                line_length = data_len[j];
            }
            j += columns;
        }
        *slot = line_length + separator_space;
    }
    // Right-align (padStart) only when every element is a number/bigint.
    let pad_start = values.iter().all(|v| {
        matches!(v, Value::Int(_) | Value::Float(_))
            || matches!(host.get(v), Some(JsObj::BigInt(_)))
    });
    let mut tmp = Vec::new();
    let mut i = 0;
    while i < output_length {
        let max = (i + columns).min(output_length);
        let mut str_line = String::new();
        let mut j = i;
        while j < max.saturating_sub(1) {
            // `output[j]` has no colors here, so padding == max_line_length[col].
            let col = j - i;
            let cell = format!("{}, ", output[j]);
            let target = max_line_length[col];
            str_line.push_str(&pad_to(&cell, target, pad_start));
            j += 1;
        }
        // The last cell of the row: right-aligned entries pad without the ", ".
        if pad_start {
            let col = j - i;
            let target = max_line_length[col] - separator_space;
            str_line.push_str(&pad_to(&output[j], target, true));
        } else {
            str_line.push_str(&output[j]);
        }
        tmp.push(str_line);
        i += columns;
    }
    (tmp, true)
}

/// Pad `s` to `width` chars: right-justified when `pad_start`, else left-justified.
/// (Padding is measured in chars; already ANSI-free here.)
fn pad_to(s: &str, width: usize, pad_start: bool) -> String {
    let len = s.chars().count();
    if len >= width {
        return s.to_string();
    }
    let fill = " ".repeat(width - len);
    if pad_start {
        format!("{fill}{s}")
    } else {
        format!("{s}{fill}")
    }
}

/// Quote a string the way `util.inspect` does — a port of `strEscape` in Node's
/// `lib/internal/util/inspect.js`.
///
/// The quote character is chosen so the contents need as little escaping as
/// possible: single quotes normally, double quotes when the string contains a
/// `'` but no `"`, and a backtick when it contains both (and neither a backtick
/// nor a `${`). Only the ACTIVE quote is backslash-escaped, alongside `\` and
/// the C0 controls + DEL, which use Node's `meta` table (`\n`, `\t`, `\b`,
/// `\f`, `\r` short forms; `\x0B`, `\x1F`, `\x7F` uppercase-hex otherwise).
fn quote_str(s: &str) -> String {
    let quote = if !s.contains('\'') {
        '\''
    } else if !s.contains('"') {
        '"'
    } else if !s.contains('`') && !s.contains("${") {
        '`'
    } else {
        '\''
    };
    let mut out = String::with_capacity(s.len() + 2);
    out.push(quote);
    for c in s.chars() {
        match c {
            _ if c == quote => {
                out.push('\\');
                out.push(c);
            }
            '\\' => out.push_str("\\\\"),
            '\u{8}' => out.push_str("\\b"),
            '\t' => out.push_str("\\t"),
            '\n' => out.push_str("\\n"),
            '\u{c}' => out.push_str("\\f"),
            '\r' => out.push_str("\\r"),
            '\u{0}'..='\u{1f}' | '\u{7f}' => out.push_str(&format!("\\x{:02X}", c as u32)),
            _ => out.push(c),
        }
    }
    out.push(quote);
    out
}

/// Render an object key: bare if it is a valid identifier, quoted otherwise.
fn fmt_key(k: &str) -> String {
    let ok = !k.is_empty()
        && k.chars()
            .next()
            .map(|c| c.is_ascii_alphabetic() || c == '_' || c == '$')
            .unwrap_or(false)
        && k.chars()
            .all(|c| c.is_ascii_alphanumeric() || c == '_' || c == '$');
    if ok {
        k.to_string()
    } else {
        quote_str(k)
    }
}

// ── iteration ────────────────────────────────────────────────────────────────

impl JsHost {
    /// Collect an iterable into a vector of values (arrays, strings, Map/Set).
    /// Generators and user `Symbol.iterator` objects go through `iter_all`, which
    /// holds no host borrow across resumes.
    pub fn iter_vec(&mut self, v: &Value) -> Result<Vec<Value>, String> {
        match self.get(v) {
            Some(JsObj::Array(items)) => Ok(items.clone()),
            Some(JsObj::Str(s)) => {
                let chars: Vec<String> = s.chars().map(|c| c.to_string()).collect();
                Ok(chars.into_iter().map(|c| self.new_str(c)).collect())
            }
            Some(JsObj::Iter { items, idx }) => Ok(items[*idx..].to_vec()),
            Some(JsObj::Set { entries, .. }) => Ok(entries.values().cloned().collect()),
            Some(JsObj::Map { entries, .. }) => {
                // Map iterates as `[key, value]` pairs.
                let pairs: Vec<(Value, Value)> = entries.values().cloned().collect();
                Ok(pairs
                    .into_iter()
                    .map(|(k, v)| self.new_array(vec![k, v]))
                    .collect())
            }
            // A `Buffer` iterates over its BYTES and a typed array over its
            // ELEMENTS — both are iterable in Node. Only `@@bytes` was handled
            // here, so `[...buf]` worked while `[...new Uint8Array([1])]` threw
            // "object is not iterable", which is the same invariant holding at
            // one of its two sites.
            Some(JsObj::Object(props))
                if props.contains_key("@@bytes") || props.contains_key("@@elems") =>
            {
                let field = if props.contains_key("@@bytes") {
                    "@@bytes"
                } else {
                    "@@elems"
                };
                match props
                    .get(field)
                    .cloned()
                    .and_then(|b| self.get(&b).cloned())
                {
                    Some(JsObj::Array(items)) => Ok(items),
                    _ => Ok(Vec::new()),
                }
            }
            _ => Err(type_error(&format!("{} is not iterable", self.type_of(v)))),
        }
    }

    /// Enumerable string keys of an object/array (for `for-in`). Internal
    /// symbol-keyed props (`@@…`) are not enumerable.
    /// `for-in` visits own enumerable keys, then every *inherited* enumerable key
    /// not already seen, walking the whole prototype chain. Class methods and the
    /// builtin prototypes are non-enumerable, so in practice this only surfaces
    /// keys a script put on a prototype itself (`F.prototype.y = 2`) — but that
    /// is exactly the constructor-function idiom older packages are written in.
    pub fn enum_keys(&mut self, v: &Value) -> Vec<Value> {
        let mut keys = self.own_enum_key_names(v);
        let mut cur = self.proto_of(v);
        let mut hops = 0;
        while let Some(p) = cur {
            // A cyclic or pathologically deep chain must not hang the loop.
            hops += 1;
            if hops > 100 || matches!(p, Value::Undef) || self.is_null(&p) {
                break;
            }
            for k in self.own_enum_key_names(&p) {
                if !keys.contains(&k) {
                    keys.push(k);
                }
            }
            cur = self.proto_of(&p);
        }
        keys.into_iter().map(|k| self.new_str(k)).collect()
    }

    /// The own *enumerable* string keys of `v`, in property order — the single
    /// source of truth behind `for-in`, `Object.keys`/`values`/`entries`,
    /// object spread, `Object.assign` and `JSON.stringify`. Internal slots
    /// (`@@…`), private fields (`#…`) and anything marked non-enumerable via
    /// `prop_attrs` are excluded.
    pub fn own_enum_key_names(&self, v: &Value) -> Vec<String> {
        self.own_key_names(v, true)
    }

    /// Own string keys of `v` in insertion order. `enum_only` drops the
    /// non-enumerable ones (`Object.keys`); otherwise every own key is reported
    /// (`getOwnPropertyNames`/`Reflect.ownKeys`).
    pub fn own_key_names(&self, v: &Value, enum_only: bool) -> Vec<String> {
        let mut keys = self.own_enum_data_keys(v, enum_only);
        // An accessor defined before its object had any ordering marker (a class
        // prototype accessor, say) still has to appear.
        for k in self.own_accessor_keys(v) {
            if (!enum_only || self.prop_attrs(v, &k).enumerable) && !keys.contains(&k) {
                keys.push(k);
            }
        }
        keys
    }

    /// The keys that own a slot in the object's property map, in insertion
    /// order, resolving accessor ordering markers back to their real key.
    fn own_enum_data_keys(&self, v: &Value, enum_only: bool) -> Vec<String> {
        match self.get(v) {
            // A `Buffer` is an index-keyed exotic: its own enumerable keys are
            // `"0".."len-1"` (the bytes live in the hidden `@@bytes` slot), never
            // the `length`/`byteLength` view metadata, which V8 keeps on the
            // prototype chain or as non-enumerable own slots.
            Some(JsObj::Object(props))
                if props.get("@@native").map(|t| self.str_of(t)).as_deref() == Some("Buffer") =>
            {
                let n = match props.get("@@bytes").and_then(|b| self.get(b)) {
                    Some(JsObj::Array(items)) => items.len(),
                    _ => 0,
                };
                (0..n).map(|i| i.to_string()).collect()
            }
            Some(JsObj::Object(props)) => props
                .keys()
                .filter_map(|k| match k.strip_prefix(ORD_MARKER) {
                    Some(real) => Some(real.to_string()),
                    None if !k.starts_with("@@") && !k.starts_with('#') => Some(k.clone()),
                    None => None,
                })
                .filter(|k| !enum_only || self.prop_attrs(v, k).enumerable)
                .collect(),
            // `OrdinaryOwnPropertyKeys` on an array exotic: the integer indices
            // ascending, then the exotic non-enumerable `length`, then the
            // ordinary string keys in insertion order. Those ordinary keys have
            // no property map to live in — a `str.match()` result's
            // `index`/`input`/`groups` and any user-assigned `arr.foo` are kept
            // in the fn-prop side table — so they are read back from there.
            Some(JsObj::Array(items)) => {
                let mut keys: Vec<String> = (0..items.len()).map(|i| i.to_string()).collect();
                if !enum_only {
                    keys.push("length".into());
                }
                keys.extend(self.fn_prop_keys(v).into_iter().filter(|k| {
                    !k.starts_with("@@")
                        && !k.starts_with('#')
                        && (!enum_only || self.prop_attrs(v, k).enumerable)
                }));
                keys
            }
            // A function/class keeps every own property in the side table. Its
            // exotic `name`/`length`/`prototype` and its class methods are all
            // non-enumerable, so under `enum_only` what is left is exactly what
            // a script assigned; `getOwnPropertyNames` reports the exotics too,
            // in V8's order (`length`, `name`, `prototype`, then the rest).
            Some(JsObj::Func(_)) | Some(JsObj::Class(_)) | Some(JsObj::BoundFunc { .. }) => {
                let mut keys: Vec<String> = Vec::new();
                if !enum_only {
                    keys.push("length".into());
                    keys.push("name".into());
                    if self.owns_prototype(v) {
                        keys.push("prototype".into());
                    }
                }
                let rest: Vec<String> = self
                    .fn_prop_keys(v)
                    .into_iter()
                    .filter(|k| {
                        !k.starts_with("@@")
                            && !k.starts_with('#')
                            && !keys.contains(k)
                            && (!enum_only || self.prop_attrs(v, k).enumerable)
                    })
                    .collect();
                keys.extend(rest);
                keys
            }
            // A builtin namespace (`require('buffer')`, `Buffer`) enumerates the
            // members node-js implements, so a package that copies a namespace
            // key-by-key gets the working set instead of an empty object.
            Some(JsObj::Builtin(ns)) => crate::stdlib::namespace_keys(&ns.clone()),
            _ => Vec::new(),
        }
    }

    /// The own enumerable `(key, value)` pairs of `v`. Buffer index keys resolve
    /// through the byte store; everything else reads the property map. Own
    /// accessor keys come back as `Undef` here — `own_enum_entries_deep` runs
    /// their getters, which cannot happen under the host borrow.
    pub fn own_enum_entries(&self, v: &Value) -> Vec<(String, Value)> {
        self.own_enum_key_names(v)
            .into_iter()
            .map(|k| {
                let val =
                    match self.get(v) {
                        // A Buffer's index keys read out of the hidden `@@bytes`
                        // array; resolve inline rather than through
                        // `buffer::byte_get`, which would re-borrow the host.
                        Some(JsObj::Object(props)) => props.get(&k).cloned().unwrap_or_else(|| {
                            match (
                                props.get("@@bytes").and_then(|b| self.get(b)),
                                k.parse::<usize>(),
                            ) {
                                (Some(JsObj::Array(items)), Ok(i)) => {
                                    items.get(i).cloned().unwrap_or(Value::Undef)
                                }
                                _ => Value::Undef,
                            }
                        }),
                        // An index reads the element; any other own key (`foo`,
                        // a match result's `index`) lives in the side table.
                        Some(JsObj::Array(items)) => k
                            .parse::<usize>()
                            .ok()
                            .and_then(|i| items.get(i).cloned())
                            .or_else(|| self.fn_prop(v, &k))
                            .unwrap_or(Value::Undef),
                        Some(JsObj::Func(_)) | Some(JsObj::Class(_)) => {
                            self.fn_prop(v, &k).unwrap_or(Value::Undef)
                        }
                        _ => Value::Undef,
                    };
                (k, val)
            })
            .collect()
    }
}

/// The own enumerable `(key, value)` pairs of `v` with every enumerable own
/// accessor's getter invoked — the observable shape `Object.values`,
/// `Object.entries`, object spread and `JSON.stringify` all need. Must be called
/// outside a `with_host` borrow because a getter re-enters the host.
pub fn own_enum_entries_deep(v: &Value) -> Vec<(String, Value)> {
    // A builtin namespace (`require('path')`, `Buffer`) has no property map at
    // all: its members are resolved on demand by `namespace_property`, which
    // re-enters the host and so cannot run inside `own_enum_entries`'s borrow.
    // Without this, spread and `Object.assign` copied the namespace's KEYS with
    // `undefined` for every value — measured against node v26.7.0,
    // `{...require('path')}.join` was `undefined` here and a function there,
    // while `Object.entries(require('path'))` (which resolves through
    // `builtins`, not through this borrow) was already correct. Two enumeration
    // paths, one of them silently value-less.
    if let Some(ns) = with_host(|h| match h.get(v) {
        Some(JsObj::Builtin(ns)) => Some(ns.clone()),
        _ => None,
    }) {
        return with_host(|h| h.own_enum_key_names(v))
            .into_iter()
            .map(|k| {
                let val = crate::builtins::namespace_property(&ns, &k);
                (k, val)
            })
            .collect();
    }
    let accessor_keys: Vec<String> = with_host(|h| {
        h.own_accessor_keys(v)
            .into_iter()
            .filter(|k| h.prop_attrs(v, k).enumerable)
            .collect()
    });
    let entries = with_host(|h| h.own_enum_entries(v));
    entries
        .into_iter()
        .map(|(k, val)| {
            if accessor_keys.contains(&k) {
                let got = get_prop_chain(v, &k).unwrap_or(Value::Undef);
                (k, got)
            } else {
                (k, val)
            }
        })
        .collect()
}

// ── function invocation ──────────────────────────────────────────────────────

/// Marshal a JS call argument into a native fusevm `Value` for `rust { }` FFI.
/// JS strings ride as `Value::Obj(JsObj::Str)` heap handles, which fusevm's
/// marshaller cannot read (it calls `Value::to_str`, which returns `"(obj:N)"`
/// for a handle); rewrite them to a native `Value::Str`. Numbers are already
/// native `Value::Int`/`Value::Float`, so they pass through (fusevm coerces
/// Float→i64/f64 per the export signature).
fn marshal_ffi_arg(v: &Value) -> Value {
    match v {
        Value::Obj(_) => match with_host(|h| h.as_str(v)) {
            Some(s) => Value::str(s),
            None => v.clone(),
        },
        _ => v.clone(),
    }
}

/// Resolve a bare name and call it (`f(args)`, `parseInt(args)`).
pub fn call_named(name: &str, args: Vec<Value>) -> Result<Value, String> {
    // Inline Rust FFI: the `rust { ... }` desugar emits `__rust_compile(b64,
    // line)`; compile + register the block's exported functions, returning JS
    // `undefined` (`Value::Undef`).
    if name == "__rust_compile" {
        let b64 = args
            .first()
            .map(|v| with_host(|h| h.str_of(v)))
            .unwrap_or_default();
        return fusevm::ffi::compile_and_register(&b64).map(|_| Value::Undef);
    }
    if let Some(v) = with_host(|h| h.read_name(name)) {
        return invoke(&v, args, None);
    }
    // A DIRECT eval — the literal `eval(src)` call form — is the ONLY one that
    // evaluates in the CALLER's scope; `(0, eval)(src)`, `const e = eval; e(src)`
    // and `[eval][0](src)` all reach the same function value but are INDIRECT
    // evals and evaluate in the global scope (ECMA-262 19.2.1.1 `PerformEval`).
    // This is the one place the two forms are distinguishable without a compiler
    // change: `call_named` is reached only from `ops::CALL`, which the compiler
    // emits exclusively for a bare-identifier callee, while every value-call form
    // goes through `invoke` → `call_builtin_function`. The `read_name` miss above
    // has already established that `eval` is not shadowed by a user binding.
    if name == "eval" {
        return crate::builtins::eval_source(args.first(), true);
    }
    if crate::builtins::is_known_builtin(name) {
        return crate::builtins::call_builtin_function(name, args);
    }
    // A `rust { ... }` block's exported functions are callable by bareword.
    // Reached only after user names/globals and builtins all miss, so JS code
    // always wins; the registry membership check keeps this off the hot path.
    if fusevm::ffi::is_registered(name) {
        let margs: Vec<Value> = args.iter().map(marshal_ffi_arg).collect();
        if let Some(r) = fusevm::ffi::try_call(name, &margs) {
            return r;
        }
    }
    Err(ref_error(name))
}

/// `recv.name(args)`.
pub fn call_method(recv: &Value, name: &str, args: Vec<Value>) -> Result<Value, String> {
    // Namespace builtins (`console`, `Math`, `JSON`, ...): dispatch by qualified
    // name.
    if let Some(ns) = with_host(|h| match h.get(recv) {
        Some(JsObj::Builtin(ns)) => Some(ns.clone()),
        _ => None,
    }) {
        let qualified = format!("{ns}.{name}");
        if crate::builtins::is_known_builtin(&qualified) {
            return crate::builtins::call_builtin_function(&qualified, args);
        }
    }
    // Object / instance: an accessor getter that yields a function, an own or
    // inherited method (class methods live on the prototype chain), then an
    // Object.prototype builtin (hasOwnProperty …). Resolve via `lookup_*`
    // directly — NOT get_property — so the Object.prototype-builtin fallback
    // never routes back through a BoundMethod and recurses.
    if with_host(|h| h.kind_of(recv)) == Some(ObjKind::Object) {
        // A native stdlib instance (`Buffer`/crypto `Hash`/`EventEmitter`/`URL`/
        // fs `Stats`/http `ServerResponse`…) carries a hidden `@@native` tag.
        // A user-added or reparented-prototype method takes precedence over the
        // native dispatcher — matching JS resolution order (own → prototype
        // chain). This is what lets Express work: it does
        // `Object.setPrototypeOf(res, app.response)` and calls `res.send(...)`,
        // where `send` is a plain function on the reparented prototype. Native
        // instance methods (`res.end`/`write`/…) are NOT stored as plain
        // function properties, so `lookup_chain` misses them and we fall through
        // to `instance_call` for the real native behavior.
        if let Some(tag) = crate::stdlib::native_tag(recv) {
            if let Some(f) = with_host(|h| lookup_chain(h, recv, name)) {
                if with_host(|h| is_callable(h, &f)) {
                    return invoke(&f, args, Some(recv.clone()));
                }
            }
            // `Object.prototype` methods reach a native instance too — a Buffer
            // inherits `hasOwnProperty`/`isPrototypeOf` through its prototype
            // chain, and the native dispatcher has no entry for them.
            if crate::builtins::is_object_builtin_method(name)
                && !crate::stdlib::instance_has_method(&tag, name)
            {
                return crate::builtins::object_builtin_method(recv, name, args);
            }
            return crate::stdlib::instance_call(&tag, recv, name, args);
        }
        if let Some((Some(getter), _)) = with_host(|h| lookup_accessor(h, recv, name)) {
            let f = invoke(&getter, Vec::new(), Some(recv.clone()))?;
            if with_host(|h| is_callable(h, &f)) {
                return invoke(&f, args, Some(recv.clone()));
            }
        }
        if let Some(f) = with_host(|h| lookup_chain(h, recv, name)) {
            if with_host(|h| is_callable(h, &f)) {
                return invoke(&f, args, Some(recv.clone()));
            }
            return Err(type_error(&format!("{name} is not a function")));
        }
        if crate::builtins::is_object_builtin_method(name) {
            return crate::builtins::object_builtin_method(recv, name, args);
        }
        if name == "constructor" {
            if let Some(r) = call_default_ctor(recv, &args) {
                return r;
            }
        }
        return Err(type_error(&format!("{name} is not a function")));
    }
    // Function value methods: call / apply / bind, then any static method stored
    // on the function object.
    if matches!(
        with_host(|h| h.kind_of(recv)),
        Some(ObjKind::Func)
            | Some(ObjKind::Class)
            | Some(ObjKind::BoundFunc)
            | Some(ObjKind::BoundMethod)
            | Some(ObjKind::Builtin)
    ) {
        if let Some(r) = crate::builtins::function_builtin_method(recv, name, &args)? {
            return Ok(r);
        }
        // A static method (own or inherited): `this` is the constructor (`recv`).
        let stat = if with_host(|h| h.kind_of(recv)) == Some(ObjKind::Class) {
            with_host(|h| h.class_static(recv, name))
        } else {
            with_host(|h| h.fn_prop(recv, name))
        };
        if let Some(f) = stat {
            if with_host(|h| is_callable(h, &f)) {
                return invoke(&f, args, Some(recv.clone()));
            }
        }
        // A method inherited via the function's [[Prototype]] chain (set with
        // `Object.setPrototypeOf(fn, proto)`) — the `router` package's router
        // functions inherit `route`/`use`/`get`/… from `Router.prototype`.
        if let Some(f) = with_host(|h| lookup_chain(h, recv, name)) {
            if with_host(|h| is_callable(h, &f)) {
                return invoke(&f, args, Some(recv.clone()));
            }
        }
        // An `Object.prototype` method invoked with a builtin namespace/prototype
        // as `this` (`hasOwnProperty.call(Map.prototype, 'get')`, the get-intrinsic
        // ownership probe) — dispatch it against the builtin receiver.
        if with_host(|h| h.kind_of(recv)) == Some(ObjKind::Builtin)
            && crate::builtins::is_object_builtin_method(name)
        {
            return crate::builtins::object_builtin_method(recv, name, args);
        }
    }
    if name == "constructor" {
        if let Some(r) = call_default_ctor(recv, &args) {
            return r;
        }
    }
    // Type methods (array/string/number, Map/Set/Symbol/generator methods).
    crate::builtins::call_type_method(recv, name, args)
}

/// `x.constructor(...)` invoked as a CALL when nothing on `x`'s prototype chain
/// owns a `constructor` slot.
///
/// Reading the property already resolves a builtin instance's native constructor
/// (the `constructor` arm of `builtins::get_property`), but the CALL path only
/// consulted the prototype chain, so the two disagreed:
/// `(function(){}).constructor === Function` read `true` while
/// `(function(){}).constructor('return 9')` threw
/// `TypeError: constructor is not a function`. That call form is exactly how
/// `get-intrinsic` — a transitive dependency of express — reaches the `Function`
/// constructor. Resolved here through the same one definition the read uses, so
/// the two can no longer drift apart. `None` means "not resolvable/callable",
/// leaving the caller's original error in place.
fn call_default_ctor(recv: &Value, args: &[Value]) -> Option<Result<Value, String>> {
    let ctor = crate::builtins::get_property(recv, "constructor").ok()?;
    with_host(|h| is_callable(h, &ctor)).then(|| invoke(&ctor, args.to_vec(), None))
}

/// Call any callable value.
pub fn invoke(callable: &Value, args: Vec<Value>, this: Option<Value>) -> Result<Value, String> {
    let obj = with_host(|h| h.get(callable).cloned());
    match obj {
        // A builtin-prototype method thunk (`Object.prototype.toString`): dispatch
        // against the invoke-time `this` (supplied by `.call`/`.apply`).
        Some(JsObj::Builtin(name)) if name.starts_with("@proto:") => {
            let recv = this.unwrap_or(Value::Undef);
            crate::builtins::proto_method(&recv, &name["@proto:".len()..], args)
        }
        // `NativeCtor.call(obj, …)` — ES5 "constructor stealing", still shipped by
        // libraries that predate `class`. `iconv-lite`'s internal codec is exactly
        // this:
        //
        //     function InternalDecoder(options, codec) { StringDecoder.call(this, codec.enc); }
        //     InternalDecoder.prototype = StringDecoder.prototype;
        //
        // A native constructor builds a fresh tagged object, so initializing the
        // SUPPLIED object means building one and moving its slots across.
        //
        // The guard is deliberately narrow: `obj` must already inherit from THIS
        // constructor's prototype, i.e. the subclass really did adopt it. Without
        // that, `Date.call(x)` and `Buffer.call(x)` — which in JS ignore `this` and
        // return a string / a buffer — would start mutating `x` instead.
        Some(JsObj::Builtin(ref name)) if steals_ctor(name, this.as_ref()) => {
            let target = this.expect("guard checked");
            let built = crate::stdlib::construct(name, &args)
                .expect("guard checked a native constructor")?;
            adopt_native_slots(&target, &built);
            Ok(Value::Undef)
        }
        Some(JsObj::Builtin(name)) => crate::builtins::call_builtin_function(&name, args),
        Some(JsObj::Func(fv)) => run_user_func(&fv, args, this),
        Some(JsObj::BoundMethod { recv, name }) => call_method(&recv, &name, args),
        Some(JsObj::BoundFunc {
            target,
            this: bthis,
            args: pre,
        }) => {
            let mut all = pre;
            all.extend(args);
            invoke(&target, all, Some(bthis))
        }
        Some(JsObj::Class(c)) => Err(type_error(&format!(
            "Class constructor {} cannot be invoked without 'new'",
            c.name
        ))),
        _ => Err(type_error(&format!(
            "{} is not a function",
            with_host(|h| h.str_of(callable))
        ))),
    }
}

/// Whether calling the native constructor `name` with `this` is the ES5
/// constructor-stealing pattern rather than an ordinary call.
///
/// True only when `name` really is a native stdlib constructor AND `this` is a
/// plain object that already inherits from that constructor's prototype — the
/// signature of `Sub.prototype = Native.prototype; Native.call(this, …)`. An
/// object that merely happens to be passed as `this` does not qualify, so
/// `Date.call(x)` / `Buffer.call(x)` keep their JS meaning (ignore `this`).
fn steals_ctor(name: &str, this: Option<&Value>) -> bool {
    let Some(target) = this else { return false };
    if !with_host(|h| matches!(h.get(target), Some(JsObj::Object(_)))) {
        return false;
    }
    // Already initialized (e.g. a re-entrant call) — nothing to steal.
    if crate::stdlib::native_tag(target).is_some() {
        return false;
    }
    let Some(proto) = with_host(|h| h.ensure_ctor_proto(name)) else {
        return false;
    };
    let mut cur = with_host(|h| h.proto_of(target));
    while let Some(p) = cur {
        if p == proto {
            return true;
        }
        cur = with_host(|h| h.proto_of(&p));
    }
    false
}

/// Move a freshly-constructed native instance's state onto `target`, so an
/// object built by a subclass constructor becomes a working instance of the
/// native class. Copies every own key the native constructor set — the hidden
/// `@@`-prefixed slots that carry the state AND the plain ones it exposes
/// (`StringDecoder`'s `encoding`) — without disturbing keys `target` already has.
fn adopt_native_slots(target: &Value, built: &Value) {
    let slots: Vec<(String, Value)> = with_host(|h| match h.get(built) {
        Some(JsObj::Object(p)) => p.iter().map(|(k, v)| (k.clone(), v.clone())).collect(),
        _ => Vec::new(),
    });
    with_host(|h| {
        if let Some(JsObj::Object(p)) = h.get_mut(target) {
            for (k, v) in slots {
                p.insert(k, v);
            }
        }
    });
}

/// Execute a user function/closure body on a fresh frame.
pub fn run_user_func(fv: &FuncVal, args: Vec<Value>, this: Option<Value>) -> Result<Value, String> {
    run_user_func_nt(fv, args, this, None)
}

/// As `run_user_func`, but with an explicit `new.target` (set by `new`).
pub fn run_user_func_nt(
    fv: &FuncVal,
    args: Vec<Value>,
    this: Option<Value>,
    new_target: Option<Value>,
) -> Result<Value, String> {
    let def = with_host(|h| h.funcs[fv.def_id].clone());
    let env = new_env(fv.env.clone());
    // Bind the simple/rest arg slots; destructuring + defaults run in the body
    // prologue (compiled ahead of the user statements).
    bind_params(&env, &def, args);
    // Arrow functions capture `this` lexically; regular functions receive it.
    let this_val = if fv.is_arrow { fv.this.clone() } else { this };
    // A generator function does not run its body on call — it returns a suspended
    // generator over the already-bound frame.
    if def.is_generator {
        let gen = make_generator(def.chunk.clone(), env, this_val, fv.home_class.clone());
        if def.is_async {
            if let Some(JsObj::Generator { id }) = with_host(|h| h.get(&gen).cloned()) {
                with_host(|h| h.generators[id as usize].async_gen = true);
            }
        }
        return Ok(gen);
    }
    // An async function runs on a coroutine and returns a Promise: it executes
    // synchronously up to the first `await`, then continues via microtasks.
    if def.is_async {
        let gen = make_generator(def.chunk.clone(), env, this_val, fv.home_class.clone());
        return Ok(run_async(gen));
    }
    let home = fv
        .home_class
        .as_ref()
        .and_then(|n| with_host(|h| h.class_registry.get(n).cloned()));
    with_host(|h| {
        h.frames.push(Frame {
            base_env: env.clone(),
            env,
            this_obj: this_val,
            new_target,
            home_class: home,
            line: 0,
            owner: Some(def.name.clone()),
            is_module: false,
        })
    });
    let r = run_chunk_on(def.chunk.clone());
    let sig = with_host(|h| {
        h.frames.pop();
        h.signal.take()
    });
    match r {
        Err(e) => Err(e),
        Ok(_) => Ok(match sig {
            Some(Signal::Return(v)) => v,
            _ => Value::Undef,
        }),
    }
}

/// Bind positional args into a fresh call environment. The compiler emits the
/// param names in `def.params`; a `...rest` slot collects the tail as an array.
fn bind_params(env: &Env, def: &FuncDef, args: Vec<Value>) {
    let mut vars: IndexMap<String, Value> = IndexMap::new();
    let mut i = 0;
    for slot in &def.params {
        if slot.rest {
            let rest: Vec<Value> = args.get(i..).map(|s| s.to_vec()).unwrap_or_default();
            let arr = with_host(|h| h.new_array(rest));
            vars.insert(slot.name.clone(), arr);
        } else {
            let v = args.get(i).cloned().unwrap_or(Value::Undef);
            vars.insert(slot.name.clone(), v);
            i += 1;
        }
    }
    // `arguments` array (simple approximation — see BUGS.md: it is a real
    // Array, not an Arguments exotic). An ARROW function never gets one:
    // `FunctionDeclarationInstantiation` (10.2.11) creates the binding only for
    // a non-arrow, so `arguments` inside an arrow resolves lexically to the
    // enclosing function's. Binding an empty one here made
    // `function f(){ const g = () => [...arguments]; }` see zero args.
    if !def.is_arrow {
        let args_arr = with_host(|h| h.new_array(args));
        vars.entry("arguments".to_string()).or_insert(args_arr);
    }
    env.borrow_mut().vars = vars;
}

/// Construct an instance with `new` — creates a fresh object, binds it as
/// `this`, runs the constructor, and returns the object (unless the constructor
/// returns its own object).
pub fn construct(ctor: &Value, args: Vec<Value>) -> Result<Value, String> {
    construct_nt(ctor, args, ctor.clone())
}

/// `new` with an explicit `new.target` (differs from `ctor` when a derived class
/// calls `super(...)` — the target stays the originally-`new`ed class).
pub fn construct_nt(ctor: &Value, args: Vec<Value>, new_target: Value) -> Result<Value, String> {
    let obj = with_host(|h| h.get(ctor).cloned());
    match obj {
        Some(JsObj::Class(_)) => construct_class(ctor, args, new_target),
        Some(JsObj::Func(fv)) => {
            // A plain constructor function: instance delegates to `fn.prototype`
            // (auto-created with a `.constructor` back-link if not yet accessed).
            let inst = with_host(|h| {
                let o = h.new_object(IndexMap::new());
                let proto = h.fn_prop(ctor, "prototype").unwrap_or_else(|| {
                    let p = h.new_object(IndexMap::new());
                    if let Some(JsObj::Object(pp)) = h.get_mut(&p) {
                        pp.insert("constructor".to_string(), ctor.clone());
                    }
                    // `F.prototype.constructor` is non-enumerable in JS.
                    h.hide_prop(&p, "constructor");
                    h.set_fn_prop(ctor, "prototype", p.clone());
                    p
                });
                h.set_proto(&o, proto);
                o
            });
            let r = run_user_func_nt(&fv, args, Some(inst.clone()), Some(new_target))?;
            if returns_object(&r) {
                Ok(r)
            } else {
                Ok(inst)
            }
        }
        Some(JsObj::Builtin(name)) => crate::builtins::construct_builtin(&name, args),
        Some(JsObj::BoundFunc {
            target, args: pre, ..
        }) => {
            let mut all = pre;
            all.extend(args);
            construct_nt(&target, all, new_target)
        }
        _ => Err(type_error(&format!(
            "{} is not a constructor",
            with_host(|h| h.str_of(ctor))
        ))),
    }
}

/// Whether a constructor's return value is an object (so `new` yields it instead
/// of the fresh instance). In JS "object" includes functions — the `router`
/// package's constructor `return router` (a function) must be honored, or the
/// returned router loses its callable identity.
fn returns_object(r: &Value) -> bool {
    matches!(
        with_host(|h| h.get(r).cloned()),
        Some(JsObj::Object(_))
            | Some(JsObj::Array(_))
            | Some(JsObj::Map { .. })
            | Some(JsObj::Set { .. })
            | Some(JsObj::Func(_))
            | Some(JsObj::Class(_))
            | Some(JsObj::BoundFunc { .. })
            | Some(JsObj::BoundMethod { .. })
            | Some(JsObj::RegExp(_))
    )
}

/// Construct a `class` instance: allocate the object linked to `C.prototype`,
/// run field initializers + the constructor (which may call `super(...)`).
fn construct_class(
    class_val: &Value,
    args: Vec<Value>,
    new_target: Value,
) -> Result<Value, String> {
    let cv = match with_host(|h| h.get(class_val).cloned()) {
        Some(JsObj::Class(c)) => c,
        _ => return Err(type_error("not a class")),
    };
    // Resolve the prototype of the *most-derived* class being `new`ed, so an
    // instance created through a `super()` chain still delegates to the leaf
    // prototype (correct method resolution).
    let leaf_proto = match with_host(|h| h.get(&new_target).cloned()) {
        Some(JsObj::Class(c)) => c.proto.clone(),
        _ => cv.proto.clone(),
    };
    let inst = with_host(|h| {
        let o = h.new_object(IndexMap::new());
        h.set_proto(&o, leaf_proto.clone());
        o
    });
    // A constructor that returns an object replaces the instance (`new` semantics).
    match run_class_ctor(&cv, &inst, args, &new_target)? {
        Some(obj) if returns_object(&obj) => Ok(obj),
        _ => Ok(inst),
    }
}

/// Run one class's field initializers then its constructor on an existing
/// instance. Returns the constructor's explicit object return (if any). For a
/// base class this is the whole init; for a derived class the constructor body
/// reaches `super(...)` which recurses into the parent.
fn run_class_ctor(
    cv: &ClassVal,
    inst: &Value,
    args: Vec<Value>,
    new_target: &Value,
) -> Result<Option<Value>, String> {
    // A derived class must run its fields AFTER super() returns; SUPER_CALL does
    // that. A base class initializes fields before the constructor body.
    if cv.parent.is_none() {
        init_fields(cv, inst)?;
    }
    match &cv.ctor {
        Some(ctor_fn) => {
            let fv = match with_host(|h| h.get(ctor_fn).cloned()) {
                Some(JsObj::Func(f)) => f,
                _ => return Err(type_error("class constructor is not a function")),
            };
            let r = run_user_func_nt(&fv, args, Some(inst.clone()), Some(new_target.clone()))?;
            return Ok(Some(r));
        }
        None => {
            // Default constructor: `constructor(...a){ super(...a); }` for a
            // derived class, empty for a base class.
            if let Some(parent) = &cv.parent {
                super_construct(parent, args, inst, new_target)?;
                init_fields(cv, inst)?;
            }
        }
    }
    Ok(None)
}

/// Evaluate and assign a class's instance-field initializers on `inst`.
fn init_fields(cv: &ClassVal, inst: &Value) -> Result<(), String> {
    for (name, thunk, name_anon) in &cv.fields {
        init_one_field(inst, name, thunk, *name_anon)?;
    }
    Ok(())
}

/// Evaluate ONE instance-field initializer thunk and install the result on
/// `inst`.
///
/// Shared by the base-class path ([`init_fields`]) and the derived-class path
/// that runs after `super(...)`; the two used to be separate loops, and only the
/// first canonicalized an array-index key.
///
/// `name_anon` carries 15.7.10's NamedEvaluation: `class C { f = function(){} }`
/// gives the function the name `f`. It is decided by the compiler from the
/// syntax, never from the value.
pub fn init_one_field(
    inst: &Value,
    name: &str,
    thunk: &Value,
    name_anon: bool,
) -> Result<(), String> {
    // The thunk is an arrow capturing the class scope; run it with `this`=inst
    // so `this.other`-referencing initializers work.
    let val = invoke(thunk, Vec::new(), Some(inst.clone()))?;
    with_host(|h| {
        if name_anon {
            let s = h.new_str(name.to_string());
            h.set_fn_prop(&val, "name", s);
        }
        if let Some(JsObj::Object(props)) = h.get_mut(inst) {
            let is_new = !props.contains_key(name);
            props.insert(name.to_string(), val);
            if is_new && array_index(name).is_some() {
                canonicalize_own_keys(props);
            }
        }
    });
    Ok(())
}

/// Run a parent constructor as part of `super(...)`: dispatch on the parent's
/// kind (class vs plain function vs builtin) using the existing instance.
pub fn super_construct(
    parent: &Value,
    args: Vec<Value>,
    inst: &Value,
    new_target: &Value,
) -> Result<(), String> {
    match with_host(|h| h.get(parent).cloned()) {
        Some(JsObj::Class(pcv)) => run_class_ctor(&pcv, inst, args, new_target).map(|_| ()),
        Some(JsObj::Func(fv)) => {
            run_user_func_nt(&fv, args, Some(inst.clone()), Some(new_target.clone()))?;
            Ok(())
        }
        Some(JsObj::Builtin(name)) => {
            // Extending a builtin (e.g. `class E extends Error`): copy the built
            // object's own props onto the instance so the subclass instance
            // carries them.
            let built = crate::builtins::construct_builtin(&name, args)?;
            let entries: Vec<(String, Value)> = with_host(|h| match h.get(&built) {
                Some(JsObj::Object(p)) => p.iter().map(|(k, v)| (k.clone(), v.clone())).collect(),
                _ => Vec::new(),
            });
            with_host(|h| {
                let keys: Vec<String> = entries.iter().map(|(k, _)| k.clone()).collect();
                if let Some(JsObj::Object(props)) = h.get_mut(inst) {
                    for (k, v) in entries {
                        props.insert(k, v);
                    }
                    canonicalize_own_keys(props);
                }
                // The copied slots keep the attributes the builtin gave them, so
                // `class E extends Error` instances hide `message`/`stack` too.
                for k in keys {
                    let a = h.prop_attrs(&built, &k);
                    h.set_prop_attrs(inst, &k, a);
                }
            });
            Ok(())
        }
        _ => Err(type_error("super is not a constructor")),
    }
}

// ── class construction (runtime) ─────────────────────────────────────────────

/// Build a class constructor value from its parts. The compiler emits (via
/// `MKCLASS`) the evaluated parent (or undefined) and the constructor closure (or
/// undefined for a default constructor); methods/getters/setters/statics/fields
/// are installed afterward by `DEF_MEMBER`/`DEF_FIELD`.
pub fn build_class(name: &str, parent: Value, ctor: Value) -> Value {
    with_host(|h| {
        let parent_opt = if matches!(parent, Value::Undef) {
            None
        } else {
            Some(parent.clone())
        };
        // The class prototype delegates to the parent's prototype (or
        // Object.prototype for a base class). Extending a builtin error links to
        // that error's prototype so `instanceof Error` holds for the subclass.
        let parent_proto = match &parent_opt {
            Some(p) => match h.get(p).cloned() {
                Some(JsObj::Class(pc)) => pc.proto.clone(),
                Some(JsObj::Builtin(bn)) => {
                    h.ensure_error_protos();
                    error_proto_of(h, &bn)
                        .or_else(|| h.fn_prop(p, "prototype"))
                        .unwrap_or_else(|| h.object_proto())
                }
                _ => h
                    .fn_prop(p, "prototype")
                    .unwrap_or_else(|| h.object_proto()),
            },
            None => h.object_proto(),
        };
        let proto = h.new_object(IndexMap::new());
        h.set_proto(&proto, parent_proto);
        let ctor_opt = if matches!(ctor, Value::Undef) {
            None
        } else {
            Some(ctor.clone())
        };
        // Give the constructor closure its home class (for `super.method()`), and
        // record its `.name`.
        if let Some(cf) = &ctor_opt {
            if let Some(JsObj::Func(f)) = h.get_mut(cf) {
                f.home_class = Some(name.to_string());
            }
        }
        let cval = ClassVal {
            name: name.to_string(),
            ctor: ctor_opt,
            parent: parent_opt,
            proto: proto.clone(),
            statics: IndexMap::new(),
            fields: Vec::new(),
        };
        let class_val = h.alloc(JsObj::Class(cval));
        h.class_registry.insert(name.to_string(), class_val.clone());
        // Link prototype → class (for instance display + `constructor`), and give
        // the class its own `prototype` fn-prop so `C.prototype` reads work.
        h.tag_proto_class(&proto, class_val.clone());
        h.set_fn_prop(&class_val, "prototype", proto.clone());
        // `Class.prototype.constructor === Class`.
        if let Some(JsObj::Object(p)) = h.get_mut(&proto) {
            p.insert("constructor".to_string(), class_val.clone());
        }
        h.hide_prop(&proto, "constructor");
        class_val
    })
}

/// Install a method / getter / setter on a class (`DEF_MEMBER`). `kind` is a
/// `member::*` tag; `is_static` targets the constructor side.
pub fn define_member(class_val: &Value, name: &str, kind: i64, is_static: bool, func: Value) {
    with_host(|h| {
        let cname = match h.get(class_val) {
            Some(JsObj::Class(c)) => c.name.clone(),
            _ => String::new(),
        };
        // Give the method its home class for `super.x()`.
        if let Some(JsObj::Func(f)) = h.get_mut(&func) {
            f.home_class = Some(cname);
        }
        // Static members live on the constructor (fn-props / static accessors);
        // instance members on the prototype.
        let target = if is_static {
            class_val.clone()
        } else {
            match h.get(class_val) {
                Some(JsObj::Class(c)) => c.proto.clone(),
                _ => return,
            }
        };
        match kind {
            member::GET => h.set_accessor(&target, name, Some(func), None),
            member::SET => h.set_accessor(&target, name, None, Some(func)),
            _ => {
                if is_static {
                    if let Some(JsObj::Class(c)) = h.get_mut(class_val) {
                        c.statics.insert(name.to_string(), func.clone());
                    }
                    h.set_fn_prop(class_val, name, func);
                } else if let Some(JsObj::Object(p)) = h.get_mut(&target) {
                    p.insert(name.to_string(), func);
                }
            }
        }
        // Class methods and accessors are non-enumerable (ES2015 ClassDefinition-
        // Evaluation), so `for (k in instance)` walking the prototype chain never
        // yields them and `Object.keys(C.prototype)` is empty.
        h.hide_prop(&target, name);
    });
}

/// Register an instance-field initializer thunk on a class (`DEF_FIELD`).
pub fn define_field(class_val: &Value, name: &str, thunk: Value, name_anon: bool) {
    with_host(|h| {
        if let Some(JsObj::Class(c)) = h.get_mut(class_val) {
            c.fields.push((name.to_string(), thunk, name_anon));
        }
    });
}

/// The `[[Prototype]]` object a constructor value hands to its instances
/// (`Ctor.prototype`), for `instanceof`.
fn ctor_prototype(h: &JsHost, ctor: &Value) -> Option<Value> {
    match h.get(ctor) {
        Some(JsObj::Class(c)) => Some(c.proto.clone()),
        Some(JsObj::Func(_)) => h.fn_prop(ctor, "prototype"),
        // A builtin's prototype lives in one of two registries: the error
        // prototypes, or the native exotic prototypes (`Buffer.prototype`,
        // `Uint8Array.prototype`). Consulting only the first made `instanceof`
        // blind to the real `Buffer.prototype → Uint8Array.prototype` chain, so
        // `Buffer.prototype instanceof Uint8Array` read false even though the
        // link was there — the instance case only passed via a native-tag
        // special case, which a prototype object does not carry.
        Some(JsObj::Builtin(name)) => h
            .error_protos
            .get(name)
            .or_else(|| h.native_protos.get(name))
            .cloned(),
        Some(JsObj::BoundFunc { target, .. }) => ctor_prototype(h, &target.clone()),
        _ => None,
    }
}

/// `obj instanceof ctor` — walk `obj`'s prototype chain looking for
/// `ctor.prototype`.
pub fn instance_of(obj: &Value, ctor: &Value) -> Result<bool, String> {
    // Not an object → never an instance (no error for our purposes).
    if !matches!(obj, Value::Obj(_)) {
        return Ok(false);
    }
    let ctor_callable = with_host(|h| {
        matches!(
            h.get(ctor),
            Some(JsObj::Func(_))
                | Some(JsObj::Class(_))
                | Some(JsObj::Builtin(_))
                | Some(JsObj::BoundFunc { .. })
        )
    });
    if !ctor_callable {
        return Err(type_error(
            "Right-hand side of 'instanceof' is not callable",
        ));
    }
    // Builtin constructors whose instances aren't prototype-linked in our model
    // (arrays/plain objects/functions) get a structural instanceof.
    if let Some(JsObj::Builtin(name)) = with_host(|h| h.get(ctor).cloned()) {
        let kind = with_host(|h| h.get(obj).cloned());
        match name.as_str() {
            "Array" => return Ok(matches!(kind, Some(JsObj::Array(_)))),
            "Function" => return Ok(with_host(|h| is_callable(h, obj))),
            // Map/Set/Promise instances are distinct heap variants, not
            // prototype-linked, so match them structurally (a WeakMap/WeakSet is a
            // Map/Set with `weak: true`, so `weakMap instanceof Map` is false).
            "Map" => return Ok(matches!(kind, Some(JsObj::Map { weak: false, .. }))),
            "WeakMap" => return Ok(matches!(kind, Some(JsObj::Map { weak: true, .. }))),
            "Set" => return Ok(matches!(kind, Some(JsObj::Set { weak: false, .. }))),
            "WeakSet" => return Ok(matches!(kind, Some(JsObj::Set { weak: true, .. }))),
            "Promise" => return Ok(matches!(kind, Some(JsObj::Promise { .. }))),
            // A RegExp is its own heap variant too, not a prototype-linked object.
            "RegExp" => return Ok(matches!(kind, Some(JsObj::RegExp(_)))),
            "Object" => {
                // Everything object-typed except a null-prototype object is an
                // Object instance.
                let is_obj = matches!(
                    kind,
                    Some(JsObj::Object(_))
                        | Some(JsObj::Array(_))
                        | Some(JsObj::Func(_))
                        | Some(JsObj::Class(_))
                        | Some(JsObj::Map { .. })
                        | Some(JsObj::Set { .. })
                        | Some(JsObj::Promise { .. })
                        | Some(JsObj::Generator { .. })
                        | Some(JsObj::RegExp(_))
                );
                if is_obj {
                    // A null-prototype object (Object.create(null) or
                    // setPrototypeOf(o, null)) is NOT an Object instance.
                    if with_host(|h| h.has_null_proto(obj)) {
                        return Ok(false);
                    }
                    return Ok(true);
                }
                return Ok(false);
            }
            // A Node `Buffer` IS a `Uint8Array` subclass instance.
            "Uint8Array" if crate::stdlib::native_tag(obj).as_deref() == Some("Buffer") => {
                return Ok(true);
            }
            // Every typed array carries the same `TypedArray` tag; the constructor
            // it is an instance of is its ELEMENT KIND.
            k if crate::stdlib::native_tag(obj).as_deref() == Some("TypedArray") => {
                return Ok(crate::stdlib::typedarray::kind_of(obj) == k);
            }
            // A native-tagged instance (`WeakRef`, `FinalizationRegistry`,
            // `TextEncoder`, …) is an instance of the builtin whose name matches
            // its hidden `@@native` tag.
            other => {
                if crate::stdlib::native_tag(obj).as_deref() == Some(other) {
                    return Ok(true);
                }
            }
        }
    }
    with_host(|h| h.ensure_error_protos());
    // The native exotic prototypes are built lazily; `instanceof` may be the
    // first thing to ask for them, so materialise them before the chain walk.
    with_host(|h| h.ensure_native_protos());
    let target = match with_host(|h| ctor_prototype(h, ctor)) {
        Some(p) => p,
        None => return Ok(false),
    };
    let mut cur = with_host(|h| h.proto_of(obj));
    while let Some(p) = cur {
        if with_host(|h| h.strict_eq(&p, &target)) {
            return Ok(true);
        }
        cur = with_host(|h| h.proto_of(&p));
    }
    Ok(false)
}

// ── generators (stackful coroutines, same-thread via corosensei) ─────────────

impl JsHost {
    /// Swap the volatile execution context in one shot, returning the previous
    /// one — installs a generator's context on resume, pulls it back on suspend.
    fn install_gen_ctx(&mut self, mut c: GenContext) -> GenContext {
        std::mem::swap(&mut self.frames, &mut c.frames);
        std::mem::swap(&mut self.error, &mut c.error);
        std::mem::swap(&mut self.exc, &mut c.exc);
        std::mem::swap(&mut self.signal, &mut c.signal);
        c
    }
    pub fn is_generator_val(&self, v: &Value) -> bool {
        matches!(self.get(v), Some(JsObj::Generator { .. }))
    }
    /// Whether `v` is an ASYNC generator object — the borrow-free form of
    /// [`is_async_generator`], usable from code already holding the host.
    pub fn is_async_gen_val(&self, v: &Value) -> bool {
        match self.get(v) {
            Some(JsObj::Generator { id }) => self
                .generators
                .get(*id as usize)
                .map(|g| g.async_gen)
                .unwrap_or(false),
            _ => false,
        }
    }
    pub fn gen_done(&self, id: u32) -> bool {
        self.generators
            .get(id as usize)
            .map(|g| g.done)
            .unwrap_or(true)
    }
    fn gen_started(&self, id: u32) -> bool {
        self.generators
            .get(id as usize)
            .map(|g| g.started)
            .unwrap_or(false)
    }
}

/// Build a suspended generator whose body is `chunk`, run in a frame with the
/// already-bound `env`. Nothing executes until the first `gen_resume`.
fn make_generator(
    chunk: Chunk,
    env: Env,
    this_val: Option<Value>,
    home_class: Option<String>,
) -> Value {
    let home = home_class
        .as_ref()
        .and_then(|n| with_host(|h| h.class_registry.get(n).cloned()));
    let frame = Frame {
        base_env: env.clone(),
        env,
        this_obj: this_val,
        new_target: None,
        home_class: home,
        line: 0,
        owner: None,
        is_module: false,
    };
    let id = with_host(|h| {
        let id = h.generators.len() as u32;
        h.generators.push(GenCell {
            coro: None,
            yielder: std::ptr::null(),
            ctx: GenContext {
                frames: vec![frame],
                ..GenContext::default()
            },
            done: false,
            started: false,
            inject: None,
            async_gen: false,
            queue: std::collections::VecDeque::new(),
            running: false,
        });
        id
    });
    let coro = corosensei::Coroutine::new(
        move |yielder: &corosensei::Yielder<Value, Value>, _first: Value| {
            // Same thread → publish the yielder so `yield` (deep in the body's VM)
            // can reach it. Valid for the whole body lifetime.
            with_host(|h| h.generators[id as usize].yielder = yielder as *const _ as *const ());
            let r = run_chunk_on(chunk);
            // A `return` inside the body leaves a Return signal carrying the final
            // value; capture it so `.next()` reports it as the completion value.
            let ret = with_host(|h| match h.signal.take() {
                Some(Signal::Return(v)) => v,
                _ => Value::Undef,
            });
            r.map(|_| ret)
        },
    );
    with_host(|h| h.generators[id as usize].coro = Some(coro));
    with_host(|h| h.alloc(JsObj::Generator { id }))
}

/// `yield v` — suspend the running generator, handing `v` to the resumer; returns
/// the value the next `gen_resume(x)` supplies (a `.next(x)` argument).
pub fn gen_yield(v: Value) -> Result<Value, String> {
    let id = match CUR_GEN.with(|c| c.get()) {
        Some(id) => id,
        None => return Err(type_error("yield outside a generator")),
    };
    let yp = with_host(|h| h.generators[id as usize].yielder);
    // SAFETY: same-thread coroutine; the yielder lives for the whole body, and we
    // only reach here from inside that body (its stack is live).
    let yielder = unsafe { &*(yp as *const corosensei::Yielder<Value, Value>) };
    let sent = yielder.suspend(v);
    // On resume, a `.return(v)`/`.throw(e)` may have queued a forced completion:
    // convert it into a Return signal / thrown value so the body unwinds and any
    // `finally` runs, exactly as a source-level `return`/`throw` would.
    if let Some(inj) = with_host(|h| h.generators[id as usize].inject.take()) {
        match inj {
            GenInject::Return(rv) => {
                with_host(|h| h.signal = Some(Signal::Return(rv)));
                return Ok(Value::Undef);
            }
            GenInject::Throw(ev) => {
                let msg = with_host(|h| crate::builtins::error_string(h, &ev));
                with_host(|h| h.exc = Some(ev));
                return Err(msg);
            }
        }
    }
    Ok(sent)
}

/// `generator.return(v)`: force the generator to complete, running any pending
/// `finally`. If it is already done (or never started) it just reports
/// `{value:v, done:true}` without executing the body.
pub fn gen_return(gen: &Value, v: Value) -> Result<GenStep, String> {
    let id = match with_host(|h| h.get(gen).cloned()) {
        Some(JsObj::Generator { id }) => id,
        _ => return Err(type_error("not a generator")),
    };
    // Not started yet (coro present, ctx never resumed) OR already done → no body
    // to unwind: complete immediately with the supplied value.
    let started = with_host(|h| h.gen_started(id));
    if with_host(|h| h.generators[id as usize].done) || !started {
        with_host(|h| h.generators[id as usize].done = true);
        return Ok(GenStep::Done(v));
    }
    with_host(|h| h.generators[id as usize].inject = Some(GenInject::Return(v)));
    gen_resume(gen, Value::Undef)
}

/// `generator.throw(e)`: inject a throw at the suspension point, running any
/// pending `finally` and letting an enclosing `try/catch` in the body handle it.
pub fn gen_throw(gen: &Value, e: Value) -> Result<GenStep, String> {
    let id = match with_host(|h| h.get(gen).cloned()) {
        Some(JsObj::Generator { id }) => id,
        _ => return Err(type_error("not a generator")),
    };
    let started = with_host(|h| h.gen_started(id));
    if with_host(|h| h.generators[id as usize].done) || !started {
        // A throw into a done/unstarted generator propagates to the caller.
        with_host(|h| h.generators[id as usize].done = true);
        let msg = with_host(|h| crate::builtins::error_string(h, &e));
        with_host(|h| h.exc = Some(e));
        return Err(msg);
    }
    with_host(|h| h.generators[id as usize].inject = Some(GenInject::Throw(e)));
    gen_resume(gen, Value::Undef)
}

/// Outcome of resuming a generator: a yielded value (not done), or the final
/// completion value (done).
pub enum GenStep {
    Yield(Value),
    Done(Value),
}

/// Resume a generator until its next `yield` or its body returns. Preserves the
/// shared host: the coroutine is taken out so the body re-enters `with_host`
/// freely, and the volatile context is swapped so the caller's frames/signal
/// survive the switch.
pub fn gen_resume(gen: &Value, send: Value) -> Result<GenStep, String> {
    let id = match with_host(|h| h.get(gen).cloned()) {
        Some(JsObj::Generator { id }) => id,
        _ => return Err(type_error("not a generator")),
    };
    if with_host(|h| h.generators[id as usize].done) {
        return Ok(GenStep::Done(Value::Undef));
    }
    let mut coro = match with_host(|h| h.generators[id as usize].coro.take()) {
        Some(c) => c,
        None => return Err("TypeError: generator already executing".into()),
    };
    with_host(|h| h.generators[id as usize].started = true);
    let gen_ctx = with_host(|h| std::mem::take(&mut h.generators[id as usize].ctx));
    let caller_ctx = with_host(|h| h.install_gen_ctx(gen_ctx));
    let prev = CUR_GEN.with(|c| c.replace(Some(id)));

    let out = coro.resume(send); // no host borrow held; body drives its own VM

    CUR_GEN.with(|c| c.set(prev));
    let mut gen_ctx = with_host(|h| h.install_gen_ctx(caller_ctx));
    // A `throw` inside the body left the thrown VALUE in the generator's context,
    // which the swap above just stashed away. Hand it to the caller so the
    // rejection/catch keeps the original error object instead of a string rebuild.
    let thrown = gen_ctx.exc.take();
    with_host(|h| {
        if let Some(v) = thrown {
            h.exc = Some(v);
        }
        h.generators[id as usize].ctx = gen_ctx;
        h.generators[id as usize].coro = Some(coro);
    });

    match out {
        corosensei::CoroutineResult::Yield(y) => Ok(GenStep::Yield(y)),
        corosensei::CoroutineResult::Return(r) => {
            with_host(|h| h.generators[id as usize].done = true);
            match r {
                Ok(v) => Ok(GenStep::Done(v)),
                Err(e) => Err(e),
            }
        }
    }
}

/// Force a generator to completion (used by `.return()` and abandoned loops):
/// marks it done without running further.
pub fn gen_close(gen: &Value) {
    if let Some(JsObj::Generator { id }) = with_host(|h| h.get(gen).cloned()) {
        with_host(|h| h.generators[id as usize].done = true);
    }
}

// ── iteration protocol (arrays, strings, Map/Set, generators, Symbol.iterator) ─

/// Convert a Map/Set key value into a `MapKey` under SameValueZero.
pub fn map_key(h: &JsHost, v: &Value) -> MapKey {
    match v {
        Value::Undef => MapKey::Undef,
        Value::Bool(b) => MapKey::Bool(*b),
        Value::Int(n) => MapKey::Num(norm_num_bits(*n as f64)),
        Value::Float(f) => MapKey::Num(norm_num_bits(*f)),
        Value::Str(s) => MapKey::Str((**s).clone()),
        Value::Obj(i) => match h.get(v) {
            Some(JsObj::Str(s)) => MapKey::Str(s.clone()),
            Some(JsObj::Null) => MapKey::Null,
            Some(JsObj::BigInt(b)) => MapKey::Big(b.to_string()),
            _ => MapKey::Ref(*i),
        },
        _ => MapKey::Undef,
    }
}

/// Canonical bit pattern for a Map/Set numeric key: `NaN` → one value, `-0` → `+0`.
fn norm_num_bits(f: f64) -> u64 {
    if f.is_nan() {
        return f64::NAN.to_bits();
    }
    if f == 0.0 {
        return 0.0f64.to_bits(); // fold -0 into +0
    }
    f.to_bits()
}

/// Fully materialize any iterable into a vector of values.
pub fn iter_all(v: &Value) -> Result<Vec<Value>, String> {
    // Generators / user iterators must resume without a live host borrow.
    if with_host(|h| h.is_generator_val(v)) {
        let mut out = Vec::new();
        while let GenStep::Yield(x) = gen_resume(v, Value::Undef)? {
            out.push(x);
        }
        return Ok(out);
    }
    // Object with a user-defined Symbol.iterator: drive its iterator protocol.
    if let Some(iter_fn) = user_iterator_fn(v) {
        let iterator = invoke(&iter_fn, Vec::new(), Some(v.clone()))?;
        return drain_iterator(&iterator);
    }
    with_host(|h| h.iter_vec(v))
}

// ── async iteration (`for await (… of …)`) ───────────────────────────────────

/// Obtain an async iterator for `for await`. If `src` has a `Symbol.asyncIterator`
/// method, use it (its `.next()` returns a promise of `{value, done}`); otherwise
/// fall back to the sync iterable, materialized into a `JsObj::Iter` whose values
/// are awaited one at a time by `async_step`.
pub fn get_async_iterator(src: &Value) -> Result<Value, String> {
    if let Some(f) = user_async_iterator_fn(src) {
        return invoke(&f, Vec::new(), Some(src.clone()));
    }
    // An `async function*` object IS its own async iterator; draining it into a
    // list here would run the whole body (and any `finally`) before the consumer
    // sees the first value.
    if let Some(JsObj::Generator { id }) = with_host(|h| h.get(src).cloned()) {
        if with_host(|h| h.generators[id as usize].async_gen) {
            return Ok(src.clone());
        }
    }
    let items = iter_all(src)?;
    Ok(with_host(|h| h.alloc(JsObj::Iter { items, idx: 0 })))
}

/// If `v` has an own/inherited `Symbol.asyncIterator` method, return it.
fn user_async_iterator_fn(v: &Value) -> Option<Value> {
    let is_plain = with_host(|h| matches!(h.get(v), Some(JsObj::Object(_))));
    if !is_plain {
        return None;
    }
    let f = with_host(|h| lookup_chain(h, v, "@@asyncIterator"));
    match f {
        Some(f) if with_host(|h| is_callable(h, &f)) => Some(f),
        _ => None,
    }
}

/// One step of a `for await` loop: return a Promise that settles to a
/// `{value, done}` record. For a native async iterator this is `iter.next()`
/// (already a promise of the record). For the sync fallback it pops the next raw
/// value, awaits it, and packages `{value: resolved, done:false}` (or
/// `{done:true}` at exhaustion).
pub fn async_step(iterator: &Value) -> Result<Value, String> {
    // An `async function*` object: resume it through the await-aware driver.
    if let Some(JsObj::Generator { id }) = with_host(|h| h.get(iterator).cloned()) {
        if with_host(|h| h.generators[id as usize].async_gen) {
            return Ok(async_gen_step(iterator, Value::Undef));
        }
    }
    // Sync-fallback iterator: drive it here, awaiting each yielded value.
    if let Some(JsObj::Iter { items, idx }) = with_host(|h| h.get(iterator).cloned()) {
        if idx >= items.len() {
            // `AsyncFromSyncIteratorContinuation` resolves the record THROUGH a
            // promise even at exhaustion, so the `done: true` step costs the same
            // two microtask ticks a value step does.
            let step = with_host(|h| h.new_promise());
            let sid = with_host(|h| h.promise_id(&step).unwrap());
            with_host(|h| {
                h.queue_micro_native(Box::new(move || {
                    resolve_promise_val(sid, iter_record(Value::Undef, true));
                    Ok(())
                }))
            });
            return Ok(step);
        }
        let raw = items[idx].clone();
        with_host(|h| {
            if let Some(JsObj::Iter { idx, .. }) = h.get_mut(iterator) {
                *idx += 1;
            }
        });
        // Await the raw value (adopts a promise's resolution), then wrap.
        let step = with_host(|h| h.new_promise());
        let sid = with_host(|h| h.promise_id(&step).unwrap());
        let raw_p = promise_of(&raw);
        let raw_id = with_host(|h| h.promise_id(&raw_p).unwrap());
        subscribe_native(
            raw_id,
            Box::new(move |state, val| {
                if state == PromiseState::Rejected {
                    reject_promise_val(sid, val);
                } else {
                    resolve_promise_val(sid, iter_record(val, false));
                }
                Ok(())
            }),
        );
        return Ok(step);
    }
    // Native async iterator: `iter.next()` returns the {value,done} promise.
    let r = call_method(iterator, "next", Vec::new())?;
    Ok(promise_of(&r))
}

/// If `v` has an own/inherited `Symbol.iterator` method (internal key
/// `@@iterator`), return it. Arrays/strings use the native fast path instead.
fn user_iterator_fn(v: &Value) -> Option<Value> {
    let is_plain = with_host(|h| matches!(h.get(v), Some(JsObj::Object(_))));
    if !is_plain {
        return None;
    }
    let f = with_host(|h| lookup_chain(h, v, "@@iterator"));
    match f {
        Some(f) if with_host(|h| is_callable(h, &f)) => Some(f),
        _ => None,
    }
}

/// Drive an iterator object (one with a `.next()` returning `{value, done}`) to
/// exhaustion.
fn drain_iterator(iterator: &Value) -> Result<Vec<Value>, String> {
    let mut out = Vec::new();
    loop {
        let step = call_method(iterator, "next", Vec::new())?;
        let done = get_prop_chain(&step, "done")?;
        if with_host(|h| h.truthy(&done)) {
            break;
        }
        out.push(get_prop_chain(&step, "value")?);
    }
    Ok(out)
}

/// Property read that walks the prototype chain (used by iteration helpers).
pub fn get_prop_chain(recv: &Value, name: &str) -> Result<Value, String> {
    crate::builtins::get_property(recv, name)
}

/// Whether `v` is an ECMAScript primitive, i.e. `ToPrimitive` is the identity
/// on it. `undefined`, `null`, booleans, numbers, strings, symbols and bigints
/// qualify; every other heap cell (objects, arrays, functions, `Map`/`Set`,
/// native-tagged instances) is an object and must be converted.
pub fn is_primitive(h: &JsHost, v: &Value) -> bool {
    match v {
        Value::Obj(_) => matches!(
            h.get(v),
            None | Some(JsObj::Null)
                | Some(JsObj::Str(_))
                | Some(JsObj::Symbol { .. })
                | Some(JsObj::BigInt(_))
        ),
        _ => true,
    }
}

/// `ToPrimitive(v, hint)` — ECMA-262 7.1.1. `hint` is `"default"`, `"number"`
/// or `"string"`.
///
/// An object carrying a `Symbol.toPrimitive` method (internal key
/// `@@toPrimitive`) has it called with the hint and must return a primitive.
/// Otherwise `OrdinaryToPrimitive` (7.1.1.1) tries `valueOf` then `toString` —
/// the order reversed for the string hint — and takes the FIRST call whose
/// result is a primitive. An object that yields no primitive (a null-prototype
/// object has neither method) throws V8's
/// `TypeError: Cannot convert object to primitive value`.
///
/// This is the conversion behind `+`, `-`/`*`/`/`/`%`/`**`, the relational
/// operators, `==` against a primitive, and `ToPropertyKey` — all of which used
/// to read `str_of` directly and so never invoked a user `valueOf`.
pub fn to_primitive(v: &Value, hint: &str) -> Result<Value, String> {
    if with_host(|h| is_primitive(h, v)) {
        return Ok(v.clone());
    }
    if let Some(f) = with_host(|h| lookup_chain(h, v, "@@toPrimitive")) {
        if with_host(|h| is_callable(h, &f)) {
            let hv = with_host(|h| h.new_str(hint.to_string()));
            let r = invoke(&f, vec![hv], Some(v.clone()))?;
            if with_host(|h| is_primitive(h, &r)) {
                return Ok(r);
            }
            return Err(type_error("Cannot convert object to primitive value"));
        }
    }
    // `Date.prototype[@@toPrimitive]` (21.4.4.45) treats the DEFAULT hint as
    // `"string"`, which is why `new Date() + 1` concatenates while
    // `new Date() - 1` is arithmetic.
    let hint = if hint == "default" && crate::stdlib::native_tag(v).as_deref() == Some("Date") {
        "string"
    } else {
        hint
    };
    let order = if hint == "string" {
        ["toString", "valueOf"]
    } else {
        ["valueOf", "toString"]
    };
    for m in order {
        let f = crate::builtins::get_property(v, m).unwrap_or(Value::Undef);
        if !with_host(|h| is_callable(h, &f)) {
            continue;
        }
        let r = invoke(&f, Vec::new(), Some(v.clone()))?;
        if with_host(|h| is_primitive(h, &r)) {
            return Ok(r);
        }
    }
    // Every object except a null-prototype one inherits `Object.prototype
    // .toString`, which always returns a string — so the exhausted-methods
    // TypeError is reachable only there. The exotics whose property funnel has
    // no `toString` entry of its own (`Map`, `Set`, `Promise`, …) land here and
    // get the same `[object Tag]` brand V8 gives them.
    if !with_host(|h| h.has_null_proto(v)) {
        return crate::builtins::proto_method(v, "Object:toString", Vec::new());
    }
    Err(type_error("Cannot convert object to primitive value"))
}

/// `ToString(v)` with `ToPrimitive` method dispatch: an object is converted
/// with the string hint (so a user `toString` — or `valueOf`, if `toString`
/// is absent or returns an object — is invoked), then rendered by `str_of`.
/// Returns a heap string value.
pub fn to_string_value(v: &Value) -> Result<Value, String> {
    let p = to_primitive(v, "string")?;
    Ok(with_host(|h| {
        let s = h.str_of(&p);
        h.new_str(s)
    }))
}

/// `ToNumber(v)` — ECMA-262 7.1.4 — with the object case going through
/// `ToPrimitive(v, number)` first, so `+{ valueOf() { return 7 } }` is `7` and
/// `+new Date(0)` is `0`. `JsHost::to_number` alone cannot do this: it runs
/// under the host borrow and so can never invoke a JS `valueOf`.
pub fn to_number_value(v: &Value) -> Result<f64, String> {
    if let Some(n) = with_host(|h| is_primitive(h, v).then(|| h.to_number(v))) {
        return Ok(n);
    }
    let p = to_primitive(v, "number")?;
    Ok(with_host(|h| h.to_number(&p)))
}

/// `ToPropertyKey(v)` — ECMA-262 7.1.19. A symbol keeps its stable internal
/// key; anything else is `ToPrimitive(v, string)` then `ToString`, so
/// `obj[{ toString() { return 'k' } }]` really reads `obj.k`.
pub fn to_property_key(v: &Value) -> Result<String, String> {
    // One borrow for the overwhelmingly common primitive key (`a[i]`, `o[s]`,
    // `o[sym]`); only an object key pays for the conversion.
    if let Some(k) = with_host(|h| is_primitive(h, v).then(|| h.property_key(v))) {
        return Ok(k);
    }
    let p = to_primitive(v, "string")?;
    Ok(with_host(|h| h.str_of(&p)))
}

/// Whether `h.get(v)` is any callable kind.
pub fn is_callable(h: &JsHost, v: &Value) -> bool {
    matches!(
        h.get(v),
        Some(JsObj::Func(_))
            | Some(JsObj::Builtin(_))
            | Some(JsObj::BoundMethod { .. })
            | Some(JsObj::BoundFunc { .. })
            | Some(JsObj::Class(_))
    )
}

/// Walk `recv`'s own props then its prototype chain for `key`, returning the
/// stored value (methods, inherited data props). Does NOT invoke accessors.
pub fn lookup_chain(h: &JsHost, recv: &Value, key: &str) -> Option<Value> {
    if let Some(JsObj::Object(p)) = h.get(recv) {
        if let Some(v) = p.get(key) {
            return Some(v.clone());
        }
    }
    let mut cur = h.proto_of(recv);
    while let Some(p) = cur {
        // A chain link may be a plain object OR a function/class (the `router`
        // package sets `Router.prototype = function(){}` and hangs its methods off
        // that function, so the methods live in the fn-prop side table).
        match h.get(&p) {
            Some(JsObj::Object(props)) => {
                if let Some(v) = props.get(key) {
                    return Some(v.clone());
                }
            }
            Some(JsObj::Func(_)) | Some(JsObj::Class(_)) => {
                if let Some(v) = h.fn_prop(&p, key) {
                    return Some(v);
                }
            }
            _ => {}
        }
        cur = h.proto_of(&p);
    }
    None
}

/// Find a getter/setter accessor for `key` on `recv` or up its prototype chain.
pub fn lookup_accessor(
    h: &JsHost,
    recv: &Value,
    key: &str,
) -> Option<(Option<Value>, Option<Value>)> {
    if let Some(a) = h.own_accessor(recv, key) {
        return Some(a);
    }
    let mut cur = h.proto_of(recv);
    while let Some(p) = cur {
        if let Some(a) = h.own_accessor(&p, key) {
            return Some(a);
        }
        cur = h.proto_of(&p);
    }
    None
}

/// Register a builtin error prototype (for `instanceof Error` etc.).
pub fn set_error_proto(name: &str, proto: Value) {
    with_host(|h| {
        h.error_protos.insert(name.to_string(), proto);
    });
}
pub fn error_proto(name: &str) -> Option<Value> {
    with_host(|h| h.error_protos.get(name).cloned())
}
/// Error prototype lookup with a borrowed host (used inside a `with_host` block).
pub fn error_proto_of(h: &JsHost, name: &str) -> Option<Value> {
    h.error_protos.get(name).cloned()
}

impl JsHost {
    /// `Error.prototype.toString` for an object whose prototype chain reaches
    /// `Error.prototype`: `"Name"` with an empty message, else `"Name: message"`.
    /// `None` for anything that is not an error, so the caller keeps its own
    /// stringification.
    pub(crate) fn error_to_string(&self, v: &Value) -> Option<String> {
        let base = self.error_protos.get("Error")?;
        let mut cur = self.proto_of(v);
        let mut is_error = false;
        while let Some(p) = cur {
            if self.strict_eq(&p, base) {
                is_error = true;
                break;
            }
            cur = self.proto_of(&p);
        }
        if !is_error {
            return None;
        }
        let name = lookup_chain(self, v, "name")
            .map(|n| self.str_of(&n))
            .unwrap_or_else(|| "Error".into());
        let message = lookup_chain(self, v, "message")
            .map(|m| self.str_of(&m))
            .unwrap_or_default();
        // Node's internal coded errors override `toString` as
        // `${name} [${code}]: ${message}` (internal/errors.js NodeError). The
        // `@@nodeError` tag marks the errors `synth_error` built from a
        // `Name [ERR_CODE]: …` string, so a user error that merely has a `.code`
        // property still stringifies plainly.
        if let Some(JsObj::Object(p)) = self.get(v) {
            if p.contains_key("@@nodeError") {
                if let Some(code) = p.get("code").map(|c| self.str_of(c)) {
                    return Some(format!("{name} [{code}]: {message}"));
                }
            }
        }
        Some(match (name.is_empty(), message.is_empty()) {
            (true, _) => message,
            (false, true) => name,
            (false, false) => format!("{name}: {message}"),
        })
    }
}

/// The set of builtin error constructor names forming the error hierarchy.
pub const ERROR_NAMES: &[&str] = &[
    "Error",
    "TypeError",
    "RangeError",
    "SyntaxError",
    "ReferenceError",
    "EvalError",
    "URIError",
    "AggregateError",
];

impl JsHost {
    /// Lazily build the builtin error prototype chain: `Error.prototype →
    /// Object.prototype`, and every specific error's prototype → `Error.prototype`.
    /// Populated once; instances link to these so `e instanceof TypeError` and
    /// `e instanceof Error` both hold.
    /// The real `Buffer.prototype` object, building the
    /// `Buffer.prototype → Uint8Array.prototype → Object.prototype` chain on
    /// first use.
    ///
    /// A `Buffer` used to be a bare tagged object with no `[[Prototype]]` at
    /// all, so `Object.getPrototypeOf(buf) === Buffer.prototype` read false and
    /// `instanceof` had to be special-cased around it. Each prototype is a
    /// genuine object carrying `@proto:<Ctor>:<method>` thunks for its instance
    /// methods, so `Buffer.prototype.slice.call(buf, 1)` still dispatches the
    /// way it did when `Buffer.prototype` was a `Builtin` namespace.
    pub fn ensure_native_protos(&mut self) {
        if self.native_protos.contains_key("Buffer") {
            return;
        }
        let obj_proto = self.object_proto();
        // `Object.prototype` is the one builtin prototype that already existed as
        // a real object (it is the chain root). Register it so `Object.prototype`
        // reads resolve to THAT object rather than a fresh `Builtin` namespace —
        // otherwise `Object.getPrototypeOf(C.prototype) === Object.prototype`
        // compares a real object against a thunk and reads false.
        self.native_protos
            .insert("Object".to_string(), obj_proto.clone());
        for m in crate::builtins::OBJECT_PROTO_METHODS {
            let thunk = self.alloc(JsObj::Builtin(format!("@proto:Object:{m}")));
            if let Some(JsObj::Object(p)) = self.get_mut(&obj_proto) {
                p.insert((*m).to_string(), thunk);
            }
            self.hide_prop(&obj_proto, m);
        }
        // `Buffer.prototype → Uint8Array.prototype → %TypedArray%.prototype →
        // Object.prototype`, which is the chain node v26.7.0 really has. The
        // shared iteration methods (`every`, `map`, `filter`, …) live on the
        // `%TypedArray%.prototype` intermediate, NOT on `Uint8Array.prototype`:
        // measured, `Uint8Array.prototype.hasOwnProperty('every')` is false in
        // Node while the intermediate owns it. `%TypedArray%` is not a global,
        // so it is reachable only by walking the chain — exactly as in Node.
        // Every element kind gets its own prototype hanging off the shared
        // intermediate, so `Object.getPrototypeOf(new Int32Array(1))` is
        // `Int32Array.prototype` rather than some other kind's. Linking them all
        // to `Uint8Array.prototype` would have been the easy version and would
        // have made an `Int32Array` claim the wrong prototype.
        let mut chain: Vec<(&str, Value)> = vec![("TypedArray", obj_proto)];
        for kind in crate::stdlib::typedarray::ELEMENT_KINDS {
            chain.push((kind, Value::Undef)); // parent: %TypedArray%.prototype
        }
        // `Buffer.prototype`'s parent is `Uint8Array.prototype` specifically.
        chain.push(("Buffer", Value::Undef));
        let mut prev: Option<Value> = None;
        for (ctor, parent) in chain.drain(..) {
            let proto = self.new_object(IndexMap::new());
            // Each kind hangs off the shared intermediate; `Buffer` hangs off
            // `Uint8Array.prototype`; the intermediate itself off
            // `Object.prototype`.
            let parent = match ctor {
                "TypedArray" => parent,
                "Buffer" => self
                    .native_protos
                    .get("Uint8Array")
                    .cloned()
                    .unwrap_or_else(|| prev.clone().expect("intermediate built first")),
                _ => self
                    .native_protos
                    .get("TypedArray")
                    .cloned()
                    .unwrap_or_else(|| prev.clone().expect("intermediate built first")),
            };
            self.set_proto(&proto, parent);
            // `%TypedArray%.prototype` has no reachable constructor global, so
            // it gets no `constructor` slot (Node's is the anonymous
            // `%TypedArray%` intrinsic).
            if ctor != "TypedArray" {
                let ctor_val = self.alloc(JsObj::Builtin(ctor.to_string()));
                if let Some(JsObj::Object(p)) = self.get_mut(&proto) {
                    p.insert("constructor".into(), ctor_val);
                }
                self.hide_prop(&proto, "constructor");
            }
            let methods: &[&str] = match ctor {
                "Buffer" => crate::stdlib::buffer::INSTANCE_METHODS,
                "TypedArray" => crate::stdlib::typedarray::PROTOTYPE_METHODS,
                // A kind's prototype owns no methods; it inherits them from the
                // intermediate above. It does own `BYTES_PER_ELEMENT`, which is
                // per-kind and which Node really keeps there (measured:
                // `Uint8Array.prototype.hasOwnProperty('BYTES_PER_ELEMENT')`).
                _ => &[],
            };
            if crate::stdlib::typedarray::ELEMENT_KINDS.contains(&ctor) {
                let bpe = Value::Float(crate::stdlib::typedarray::bytes_per_element(ctor) as f64);
                if let Some(JsObj::Object(p)) = self.get_mut(&proto) {
                    p.insert("BYTES_PER_ELEMENT".into(), bpe);
                }
                self.hide_prop(&proto, "BYTES_PER_ELEMENT");
            }
            for m in methods {
                let thunk = self.alloc(JsObj::Builtin(format!("@proto:{ctor}:{m}")));
                if let Some(JsObj::Object(p)) = self.get_mut(&proto) {
                    p.insert((*m).to_string(), thunk);
                }
                self.hide_prop(&proto, m);
            }
            self.native_protos.insert(ctor.to_string(), proto.clone());
            prev = Some(proto);
        }
    }

    /// The real prototype object for a builtin exotic, if it has one.
    pub fn native_proto(&self, ctor: &str) -> Option<Value> {
        self.native_protos.get(ctor).cloned()
    }

    /// The real `.prototype` object for a native stdlib constructor (`StringDecoder`,
    /// `Hash`, `URLSearchParams`, …), built on first read and cached.
    ///
    /// `Ctor.prototype` used to read `undefined` for every native class outside the
    /// hand-written `is_builtin_ctor` list, which broke the ES5 subclassing pattern
    /// that libraries still use. `iconv-lite`'s internal codec — reached from
    /// `raw-body` on every `express.json()` request — does exactly this:
    ///
    /// ```text
    /// var StringDecoder = require('string_decoder').StringDecoder;
    /// if (!StringDecoder.prototype.end) StringDecoder.prototype.end = function () {};
    /// function InternalDecoder(options, codec) { StringDecoder.call(this, codec.enc); }
    /// InternalDecoder.prototype = StringDecoder.prototype;
    /// ```
    ///
    /// The first line threw `Cannot read properties of undefined (reading 'end')`.
    ///
    /// Methods come from `stdlib::instance_method_lists`, the same table a method
    /// READ consults, so the prototype can never advertise a name the dispatcher
    /// does not implement. Each is the `@proto:<Ctor>:<method>` thunk that
    /// dispatches against its invoke-time `this`, so a subclass instance whose
    /// prototype IS this object gets the native implementation. Returns `None` for
    /// a tag with no instance methods, leaving those constructors as they were.
    pub fn ensure_ctor_proto(&mut self, ctor: &str) -> Option<Value> {
        if let Some(p) = self.native_protos.get(ctor) {
            return Some(p.clone());
        }
        let (own, emitter) = crate::stdlib::instance_method_lists(ctor);
        if own.is_empty() && emitter.is_empty() {
            return None;
        }
        let obj_proto = self.object_proto();
        let proto = self.new_object(IndexMap::new());
        self.set_proto(&proto, obj_proto);
        let ctor_val = self.alloc(JsObj::Builtin(ctor.to_string()));
        if let Some(JsObj::Object(p)) = self.get_mut(&proto) {
            p.insert("constructor".into(), ctor_val);
        }
        self.hide_prop(&proto, "constructor");
        for m in own.iter().chain(emitter.iter()) {
            let thunk = self.alloc(JsObj::Builtin(format!("@proto:{ctor}:{m}")));
            if let Some(JsObj::Object(p)) = self.get_mut(&proto) {
                p.insert((*m).to_string(), thunk);
            }
            self.hide_prop(&proto, m);
        }
        self.native_protos.insert(ctor.to_string(), proto.clone());
        Some(proto)
    }

    pub fn ensure_error_protos(&mut self) {
        if !self.error_protos.is_empty() {
            return;
        }
        let obj_proto = self.object_proto();
        // Error.prototype first (the shared base).
        let err_proto = self.new_object(IndexMap::new());
        self.set_proto(&err_proto, obj_proto);
        let nm = self.new_str("Error");
        let empty = self.new_str("");
        let ctor = self.alloc(JsObj::Builtin("Error".into()));
        if let Some(JsObj::Object(p)) = self.get_mut(&err_proto) {
            p.insert("name".into(), nm);
            p.insert("message".into(), empty);
            p.insert("constructor".into(), ctor);
        }
        // Everything on `Error.prototype` is non-enumerable in V8.
        for k in ["name", "message", "constructor"] {
            self.hide_prop(&err_proto, k);
        }
        self.error_protos.insert("Error".into(), err_proto.clone());
        for name in &ERROR_NAMES[1..] {
            let p = self.new_object(IndexMap::new());
            self.set_proto(&p, err_proto.clone());
            let nm = self.new_str(*name);
            let ctor = self.alloc(JsObj::Builtin((*name).to_string()));
            if let Some(JsObj::Object(o)) = self.get_mut(&p) {
                o.insert("name".into(), nm);
                o.insert("constructor".into(), ctor);
            }
            self.hide_prop(&p, "name");
            self.hide_prop(&p, "constructor");
            self.error_protos.insert((*name).to_string(), p);
        }
    }
}

// ── Map/Set element access (used by builtins) ────────────────────────────────

impl JsHost {
    /// A function's `.length`: the count of leading params before the first one
    /// with a default or the rest element.
    pub fn func_arity(&self, v: &Value) -> usize {
        let def_id = match self.get(v) {
            Some(JsObj::Func(f)) => Some(f.def_id),
            Some(JsObj::Class(c)) => match c.ctor.as_ref().and_then(|cf| self.get(cf)) {
                Some(JsObj::Func(f)) => Some(f.def_id),
                _ => None,
            },
            _ => None,
        };
        match def_id.and_then(|id| self.funcs.get(id)) {
            Some(def) => def
                .params
                .iter()
                .take_while(|p| !p.rest && !p.has_default)
                .count(),
            None => 0,
        }
    }

    pub fn is_map(&self, v: &Value) -> bool {
        matches!(self.get(v), Some(JsObj::Map { .. }))
    }
    pub fn is_set(&self, v: &Value) -> bool {
        matches!(self.get(v), Some(JsObj::Set { .. }))
    }
}

// ── promises & the event loop ────────────────────────────────────────────────

impl JsHost {
    /// Allocate a fresh pending promise, returning its heap value.
    pub fn new_promise(&mut self) -> Value {
        let id = self.promises.len() as u32;
        self.promises.push(PromiseCell {
            state: PromiseState::Pending,
            value: Value::Undef,
            reactions: Vec::new(),
            handled: false,
        });
        self.alloc(JsObj::Promise { id })
    }
    pub fn promise_id(&self, v: &Value) -> Option<u32> {
        match self.get(v) {
            Some(JsObj::Promise { id }) => Some(*id),
            _ => None,
        }
    }
    pub fn promise_state(&self, id: u32) -> PromiseState {
        self.promises[id as usize].state
    }
    pub fn promise_value(&self, id: u32) -> Value {
        self.promises[id as usize].value.clone()
    }
    pub fn promise_mark_handled(&mut self, id: u32) {
        self.promises[id as usize].handled = true;
    }
    /// Take the pending reactions of a promise (called on settle).
    pub fn take_reactions(&mut self, id: u32) -> Vec<PromiseReaction> {
        std::mem::take(&mut self.promises[id as usize].reactions)
    }
    pub fn add_reaction(&mut self, id: u32, r: PromiseReaction) {
        self.promises[id as usize].reactions.push(r);
    }
    pub fn settle_promise(&mut self, id: u32, state: PromiseState, value: Value) {
        let c = &mut self.promises[id as usize];
        if c.state != PromiseState::Pending {
            return; // already settled — resolve/reject are one-shot
        }
        c.state = state;
        c.value = value;
    }
    pub fn queue_micro(&mut self, cb: Value, args: Vec<Value>) {
        self.microtasks.push_back(Task::Js { cb, args });
    }
    pub fn queue_nexttick(&mut self, cb: Value, args: Vec<Value>) {
        self.nextticks.push_back(Task::Js { cb, args });
    }
    /// Schedule a native (Rust) microtask — used by Promise reactions and async
    /// resumption.
    pub fn queue_micro_native(&mut self, f: Box<dyn FnOnce() -> Result<(), String>>) {
        self.microtasks.push_back(Task::Native(f));
    }
    /// Schedule a macrotask. `interval` is the repeat period for `setInterval`
    /// (`None` for the one-shot `setTimeout`/`setImmediate`). Returns the timer
    /// id, which the `Timeout`/`Immediate` handle object carries so `clear*`,
    /// `ref`/`unref` and `refresh` can find this entry again.
    pub fn add_timer(
        &mut self,
        delay: f64,
        callback: Value,
        args: Vec<Value>,
        interval: Option<f64>,
    ) -> u64 {
        let id = self.next_timer;
        self.next_timer += 1;
        // Real deadline for the real-clock path; `setImmediate` (delay < 0) is
        // clamped to "now". Virtual-clock ordering still uses `delay`/`seq`.
        let deadline = Instant::now() + Duration::from_millis(delay.max(0.0) as u64);
        self.macrotasks.push(Timer {
            id,
            delay,
            seq: id,
            callback,
            args,
            cancelled: false,
            interval,
            refed: true,
            deadline,
        });
        id
    }
    /// Re-arm a repeating timer that is about to fire, keeping its id (so a
    /// `clearInterval` from *inside* the callback cancels this very entry) and
    /// taking a fresh `seq` so same-delay peers still round-robin.
    ///
    /// Called BEFORE the callback runs: if it were called after, the entry would
    /// be absent while the callback executed and a `clearInterval(t)` there would
    /// cancel nothing, resurrecting an interval the program had stopped.
    fn rearm_timer(&mut self, t: &Timer, period: f64) {
        let seq = self.next_timer;
        self.next_timer += 1;
        let deadline = Instant::now() + Duration::from_millis(period.max(0.0) as u64);
        self.macrotasks.push(Timer {
            id: t.id,
            delay: t.delay,
            seq,
            callback: t.callback.clone(),
            args: t.args.clone(),
            cancelled: false,
            interval: Some(period),
            refed: t.refed,
            deadline,
        });
    }
    /// `timeout.ref()` / `timeout.unref()` — set the handle bit on a pending
    /// timer. A no-op once the timer has fired or been cleared (Node likewise
    /// treats `ref`/`unref` on a dead timer as inert).
    pub fn set_timer_refed(&mut self, id: u64, refed: bool) {
        for t in &mut self.macrotasks {
            if t.id == id && !t.cancelled {
                t.refed = refed;
            }
        }
    }
    /// `timeout.hasRef()` — whether a still-pending timer holds the loop open.
    /// A fired or cleared timer reports `false`, matching Node.
    pub fn timer_has_ref(&self, id: u64) -> bool {
        self.macrotasks
            .iter()
            .any(|t| t.id == id && !t.cancelled && t.refed)
    }
    /// `timeout.refresh()` — restart the countdown from now, as if the timer had
    /// just been scheduled.
    pub fn refresh_timer(&mut self, id: u64) {
        let now = Instant::now();
        for t in &mut self.macrotasks {
            if t.id == id && !t.cancelled {
                t.deadline = now + Duration::from_millis(t.delay.max(0.0) as u64);
            }
        }
    }
    /// Clone the I/O sender for a background I/O thread.
    pub fn io_sender(&self) -> Sender<IoTask> {
        self.io_tx.clone()
    }
    /// Register a live handle (listener/socket/ref'd resource) keeping the loop
    /// alive.
    pub fn incr_handle(&mut self) {
        self.open_handles += 1;
    }
    /// Release a handle; the loop exits once this reaches `0` with empty queues.
    pub fn decr_handle(&mut self) {
        self.open_handles = self.open_handles.saturating_sub(1);
    }
    pub fn open_handles(&self) -> usize {
        self.open_handles
    }
    /// Pop the earliest timer whose real deadline is at or before `now` (I/O
    /// path). Ties break by `seq`.
    fn pop_due_timer(&mut self, now: Instant) -> Option<Timer> {
        let idx = self
            .macrotasks
            .iter()
            .enumerate()
            .filter(|(_, t)| !t.cancelled && t.deadline <= now)
            .min_by(|(_, a), (_, b)| a.deadline.cmp(&b.deadline).then(a.seq.cmp(&b.seq)))
            .map(|(i, _)| i);
        idx.map(|i| self.macrotasks.remove(i))
    }
    /// Time until the earliest pending timer's deadline (I/O path blocking bound),
    /// or `None` if no timers are pending. Clamped to `0` for already-due timers.
    fn next_timer_timeout(&self, now: Instant) -> Option<Duration> {
        self.macrotasks
            .iter()
            .filter(|t| !t.cancelled)
            .map(|t| t.deadline)
            .min()
            .map(|d| d.saturating_duration_since(now))
    }
    pub fn cancel_timer(&mut self, id: u64) {
        for t in &mut self.macrotasks {
            if t.id == id {
                t.cancelled = true;
            }
        }
    }
    fn pop_next_timer(&mut self) -> Option<Timer> {
        // Earliest (delay, seq) fires first — a deterministic virtual clock.
        let idx = self
            .macrotasks
            .iter()
            .enumerate()
            .filter(|(_, t)| !t.cancelled)
            .min_by(|(_, a), (_, b)| {
                a.delay
                    .partial_cmp(&b.delay)
                    .unwrap_or(std::cmp::Ordering::Equal)
                    .then(a.seq.cmp(&b.seq))
            })
            .map(|(i, _)| i);
        idx.map(|i| self.macrotasks.remove(i))
    }
    fn next_microtask(&mut self) -> Option<Task> {
        // nextTick drains before promise microtasks (Node ordering).
        self.nextticks
            .pop_front()
            .or_else(|| self.microtasks.pop_front())
    }
    fn has_microtasks(&self) -> bool {
        !self.nextticks.is_empty() || !self.microtasks.is_empty()
    }
    /// Whether any pending timer is *referenced* — the timer half of Node's
    /// handle count. Only these keep the loop alive; unref'd timers still fire
    /// while something else holds the loop open, but never hold it themselves.
    fn has_refed_macrotasks(&self) -> bool {
        self.macrotasks.iter().any(|t| !t.cancelled && t.refed)
    }
    /// Whether any pending timer repeats. A repeating timer cannot run on the
    /// virtual clock: virtual time never advances, so the interval would re-arm
    /// at the same instant forever, spinning a core and starving every
    /// longer-delay timer behind it. Its presence forces the real clock.
    fn has_pending_interval(&self) -> bool {
        self.macrotasks
            .iter()
            .any(|t| !t.cancelled && t.interval.is_some())
    }
}

/// Drive the event loop to quiescence.
///
/// **Liveness** is Node's handle count: the loop runs while a microtask is
/// pending, an open handle is registered (a listening server, a live socket, an
/// in-flight async op), or a *referenced* timer is still pending. That last term
/// is what makes `setInterval(fn, 1000)` hold the process open forever, as it
/// does in Node — the interval re-arms itself, so a ref'd timer is always
/// pending and the loop never reaches its exit condition.
///
/// Two **clock regimes**, selected per iteration:
///
/// - **Virtual clock** (no open handles and no repeating timer): the original
///   deterministic path — fire the earliest `(delay, seq)` timer immediately, no
///   real waiting. Parity output and test speed for ordinary `setTimeout`
///   scripts are unchanged.
/// - **Real clock** (an open handle, or any pending interval): fire every timer
///   whose wall-clock deadline has passed, then BLOCK on the I/O channel
///   (`recv_timeout` bounded by the next deadline, or unbounded `recv` if no
///   timers) and run the received `IoTask` on the main thread. The host keeps
///   its own `Sender`, so `recv` never disconnects while the process should stay
///   alive.
///
///   A repeating timer *must* take this path: virtual time never advances, so an
///   interval on the virtual clock would re-fire at the same instant forever,
///   spinning a core and starving every longer-delay timer behind it.
///
/// Errors thrown by a task/timer/I/O dispatch abort the loop (uncaught → surfaced).
pub fn run_event_loop() -> Result<(), String> {
    // Own the receiver for the loop's duration (blocking `recv` cannot hold a
    // host borrow); restore it afterward so a re-entrant run reuses the channel.
    let rx = with_host(|h| h.io_rx.take());
    let result = drive_event_loop(rx.as_ref());
    with_host(|h| h.io_rx = rx);
    result
}

fn drive_event_loop(rx: Option<&Receiver<IoTask>>) -> Result<(), String> {
    loop {
        // 1) Exhaust the microtask queue (nextTick before promise reactions),
        //    then report anything that rejected with nobody watching.
        while let Some(task) = with_host(|h| h.next_microtask()) {
            task.run()?;
        }
        check_unhandled_rejections()?;

        // 2) Liveness (Node's handle count). Nothing referenced left to do ⇒ the
        //    process exits, dropping any unref'd timers still pending — which is
        //    why `setTimeout(fn, 1000).unref()` never fires, while an unref'd
        //    timer behind a ref'd one does.
        let alive =
            with_host(|h| h.has_microtasks() || h.open_handles() > 0 || h.has_refed_macrotasks());
        if !alive {
            break;
        }

        // 3) Pick the clock regime for this turn.
        let virtual_clock = with_host(|h| h.open_handles() == 0 && !h.has_pending_interval());
        if virtual_clock {
            // ── virtual-clock regime (unchanged for one-shot timers) ─────────
            match with_host(|h| h.pop_next_timer()) {
                Some(t) => fire_timer(t)?,
                // Unreachable while `alive` holds (a ref'd timer must exist),
                // but exiting is the safe reading of "nothing left to run".
                None => break,
            }
            continue;
        }

        // ── real-clock / blocking-I/O regime ─────────────────────────────────
        let now = Instant::now();
        if let Some(t) = with_host(|h| h.pop_due_timer(now)) {
            fire_timer(t)?;
            continue; // re-drain microtasks, re-check deadlines
        }
        // Nothing due and no pending microtasks: block for the next I/O event,
        // bounded by the soonest timer deadline so due timers still fire on time.
        let rx = rx.expect("blocking-I/O regime requires the I/O receiver");
        let timeout = with_host(|h| h.next_timer_timeout(now));
        let recv = match timeout {
            Some(d) => rx.recv_timeout(d),
            None => rx
                .recv()
                .map_err(|_| std::sync::mpsc::RecvTimeoutError::Disconnected),
        };
        match recv {
            Ok(task) => task()?,
            Err(std::sync::mpsc::RecvTimeoutError::Timeout) => {} // a timer is now due
            Err(std::sync::mpsc::RecvTimeoutError::Disconnected) => break, // no senders left
        }
    }
    Ok(())
}

/// Run one due timer's callback, first re-arming it if it repeats.
///
/// The re-arm happens BEFORE the callback runs so that a `clearInterval(t)`
/// issued from inside that callback cancels the next occurrence. Re-arming
/// afterwards would leave the interval absent from the queue for the duration of
/// its own callback, so the `clear` would match nothing and the freshly pushed
/// entry would resurrect an interval the program had just stopped.
fn fire_timer(t: Timer) -> Result<(), String> {
    if let Some(period) = t.interval {
        with_host(|h| h.rearm_timer(&t, period));
    }
    invoke(&t.callback, t.args, None)?;
    Ok(())
}

// ── async functions & promise resolution (native) ────────────────────────────

/// Drive a freshly-built async coroutine and return its result promise.
fn run_async(gen: Value) -> Value {
    let result = with_host(|h| h.new_promise());
    let rid = with_host(|h| h.promise_id(&result).unwrap());
    drive_async(gen, rid, Value::Undef);
    result
}

/// Resume an async coroutine one step, wiring `await` continuations to promise
/// settlement.
fn drive_async(gen: Value, rid: u32, send: Value) {
    match gen_resume(&gen, send) {
        Ok(GenStep::Yield(awaited)) => {
            let ap = promise_of(&awaited);
            let aid = with_host(|h| h.promise_id(&ap).unwrap());
            let gen2 = gen.clone();
            subscribe_native(
                aid,
                Box::new(move |state, val| {
                    // Resume the coroutine with a `[tag, value]` packet the AWAIT
                    // op unwraps (tag 1 ⇒ the awaited promise rejected → throw).
                    let tag = if state == PromiseState::Rejected {
                        1.0
                    } else {
                        0.0
                    };
                    let packet = with_host(|h| h.new_array(vec![Value::Float(tag), val]));
                    drive_async(gen2, rid, packet);
                    Ok(())
                }),
            );
        }
        Ok(GenStep::Done(v)) => resolve_promise_val(rid, v),
        Err(e) => {
            let ev = take_exc_or_error(&e);
            reject_promise_val(rid, ev);
        }
    }
}

/// The AWAIT op body (runs inside the async coroutine): suspend, yielding the
/// awaited value; on resume, unwrap the settlement packet (throwing on reject).
pub fn await_value(awaited: Value) -> Result<Value, String> {
    // Inside an `async function*`, `await` and `yield` share one coroutine
    // yielder, so an awaited value has to be tagged or the driver would hand it
    // to the consumer as if the body had yielded it.
    let awaited = match CUR_GEN.with(|c| c.get()) {
        Some(id) if with_host(|h| h.generators[id as usize].async_gen) => with_host(|h| {
            let mut m = IndexMap::new();
            m.insert(AWAIT_MARKER.to_string(), awaited);
            h.new_object(m)
        }),
        _ => awaited,
    };
    let packet = gen_yield(awaited)?;
    let items = with_host(|h| h.iter_vec(&packet)).unwrap_or_default();
    let tag = items
        .first()
        .map(|v| with_host(|h| h.to_number(v)))
        .unwrap_or(0.0);
    let val = items.get(1).cloned().unwrap_or(Value::Undef);
    if tag == 1.0 {
        with_host(|h| h.exc = Some(val.clone()));
        Err(with_host(|h| crate::builtins::error_string(h, &val)))
    } else {
        Ok(val)
    }
}

/// Hidden key marking an `await` suspension inside an async generator.
const AWAIT_MARKER: &str = "@@await";

/// The operand of an `await` suspension, or `None` for a real `yield`.
fn await_marker(v: &Value) -> Option<Value> {
    with_host(|h| match h.get(v) {
        Some(JsObj::Object(props)) if props.len() == 1 => props.get(AWAIT_MARKER).cloned(),
        _ => None,
    })
}

/// `AsyncGeneratorEnqueue` — queue one request against an `async function*` and
/// hand back the promise its `{value, done}` record (or rejection) will settle.
///
/// All three of `.next`, `.return` and `.throw` come through here, so a request
/// never resumes the body while an earlier one is still suspended on an
/// internal `await`.
pub fn async_gen_enqueue(gen: &Value, req: GenReq) -> Value {
    let step = with_host(|h| h.new_promise());
    let sid = with_host(|h| h.promise_id(&step).unwrap());
    let id = match with_host(|h| match h.get(gen) {
        Some(JsObj::Generator { id }) => Some(*id),
        _ => None,
    }) {
        Some(id) => id,
        None => return step,
    };
    with_host(|h| h.generators[id as usize].queue.push_back((req, sid)));
    pump_async_gen(gen.clone(), id);
    step
}

/// One `.next(v)` of an `async function*`.
pub fn async_gen_step(gen: &Value, send: Value) -> Value {
    async_gen_enqueue(gen, GenReq::Next(send))
}

/// `AsyncGeneratorResumeNext`: start the oldest queued request, unless one is
/// already in flight (the body may only be resumed by one request at a time).
fn pump_async_gen(gen: Value, id: u32) {
    if with_host(|h| h.generators[id as usize].running) {
        return;
    }
    let Some((req, sid)) = with_host(|h| h.generators[id as usize].queue.pop_front()) else {
        return;
    };
    with_host(|h| h.generators[id as usize].running = true);
    start_async_gen_req(gen, sid, req);
}

/// Begin one queued request: resume the body with the completion it carries,
/// then hand the outcome to the shared continuation.
///
/// A RETURN completion always Awaits its value before the body sees it — via
/// `AsyncGeneratorUnwrapYieldResumption` (ECMA-262 27.6.3.7) when the generator
/// is suspended at a `yield`, and via `AsyncGeneratorAwaitReturn` (27.6.3.9)
/// when it is not yet started or already completed. So a `.return()` settles one
/// microtask after a `.next()` or `.throw()` issued in its place would, and the
/// `finally` it unwinds through runs a tick later too. Skipping that tick lets a
/// `.return()` overtake the reactions of the `.next()` it followed.
fn start_async_gen_req(gen: Value, sid: u32, req: GenReq) {
    if matches!(req, GenReq::Return(_)) {
        with_host(|h| {
            h.queue_micro_native(Box::new(move || {
                resume_async_gen_req(gen, sid, req);
                Ok(())
            }))
        });
        return;
    }
    resume_async_gen_req(gen, sid, req);
}

/// Deliver a queued completion to the body and settle its step promise.
fn resume_async_gen_req(gen: Value, sid: u32, req: GenReq) {
    let step = match req {
        GenReq::Next(v) => gen_resume(&gen, v),
        GenReq::Return(v) => gen_return(&gen, v),
        GenReq::Throw(e) => gen_throw(&gen, e),
    };
    settle_async_gen_step(gen, sid, step);
}

/// One request has settled: release the body and start the next queued request.
fn finish_async_gen_step(gen: Value, id: u32) {
    with_host(|h| h.generators[id as usize].running = false);
    pump_async_gen(gen, id);
}

/// Whether `v` is an `async function*` object (its `.next()` yields promises).
pub fn is_async_generator(v: &Value) -> bool {
    let id = match with_host(|h| match h.get(v) {
        Some(JsObj::Generator { id }) => Some(*id),
        _ => None,
    }) {
        Some(id) => id,
        None => return false,
    };
    with_host(|h| h.generators[id as usize].async_gen)
}

/// A `{ value, done }` iterator-result object.
fn iter_record(value: Value, done: bool) -> Value {
    with_host(|h| {
        let mut m = IndexMap::new();
        m.insert("value".to_string(), value);
        m.insert("done".to_string(), Value::Bool(done));
        h.new_object(m)
    })
}

/// Resume a request that was suspended on an internal `await` (always a normal
/// completion — the awaited promise's outcome rides in `packet`).
fn drive_async_gen(gen: Value, sid: u32, packet: Value) {
    let step = gen_resume(&gen, packet);
    settle_async_gen_step(gen, sid, step);
}

/// Turn one body resumption into a settled step promise: transparently re-drive
/// internal `await` suspensions, and settle on the first REAL yield or on the
/// body's completion. Shared by the initial resume of a queued request and by
/// every await-resumption of it.
fn settle_async_gen_step(gen: Value, sid: u32, step: Result<GenStep, String>) {
    let id = match with_host(|h| match h.get(&gen) {
        Some(JsObj::Generator { id }) => Some(*id),
        _ => None,
    }) {
        Some(id) => id,
        None => return,
    };
    match step {
        Ok(GenStep::Yield(v)) => match await_marker(&v) {
            Some(awaited) => {
                // An internal `await`: settle it, then resume the body. The
                // request stays in flight across the suspension.
                let ap = promise_of(&awaited);
                let aid = with_host(|h| h.promise_id(&ap).unwrap());
                subscribe_native(
                    aid,
                    Box::new(move |state, val| {
                        let tag = if state == PromiseState::Rejected {
                            1.0
                        } else {
                            0.0
                        };
                        let packet = with_host(|h| h.new_array(vec![Value::Float(tag), val]));
                        drive_async_gen(gen.clone(), sid, packet);
                        Ok(())
                    }),
                );
            }
            // ECMA-262 27.6.3.8 AsyncGeneratorYield step 5: the yielded value is
            // AWAITED before the step promise settles, so `yield somePromise`
            // hands the consumer the RESOLVED value (and costs its microtask).
            None => {
                let yp = promise_of(&v);
                let yid = with_host(|h| h.promise_id(&yp).unwrap());
                subscribe_native(
                    yid,
                    Box::new(move |state, val| {
                        if state == PromiseState::Rejected {
                            reject_promise_val(sid, val);
                        } else {
                            resolve_promise_val(sid, iter_record(val, false));
                        }
                        finish_async_gen_step(gen.clone(), id);
                        Ok(())
                    }),
                );
            }
        },
        Ok(GenStep::Done(v)) => {
            resolve_promise_val(sid, iter_record(v, true));
            finish_async_gen_step(gen, id);
        }
        Err(e) => {
            let ev = take_exc_or_error(&e);
            reject_promise_val(sid, ev);
            finish_async_gen_step(gen, id);
        }
    }
}

/// A promise for `v`: `v` itself if it is already a promise, else a promise
/// resolved with `v`.
pub fn promise_of(v: &Value) -> Value {
    if with_host(|h| h.promise_id(v)).is_some() {
        return v.clone();
    }
    let p = with_host(|h| h.new_promise());
    let id = with_host(|h| h.promise_id(&p).unwrap());
    resolve_promise_val(id, v.clone());
    p
}

/// Register a native reaction on promise `id` (schedules immediately if already
/// settled).
pub fn subscribe_native(id: u32, f: Box<dyn FnOnce(PromiseState, Value) -> Result<(), String>>) {
    // A native continuation (`await`, promise adoption, `for await`) observes a
    // rejection exactly as a `.catch` does, so it is not "unhandled".
    with_host(|h| h.promise_mark_handled(id));
    let state = with_host(|h| h.promise_state(id));
    if state == PromiseState::Pending {
        with_host(|h| h.add_reaction(id, PromiseReaction::Native(f)));
    } else {
        let val = with_host(|h| h.promise_value(id));
        with_host(|h| h.queue_micro_native(Box::new(move || f(state, val))));
    }
}

/// The Promise "resolve" operation: adopt `value`'s state if it is a promise,
/// else fulfill with it.
pub fn resolve_promise_val(id: u32, value: Value) {
    if with_host(|h| h.promise_state(id)) != PromiseState::Pending {
        return;
    }
    if let Some(vid) = with_host(|h| h.promise_id(&value)) {
        if vid == id {
            // Resolving a promise with itself → reject with a TypeError.
            let e = with_host(|h| {
                crate::builtins::synth_error(h, "TypeError: Chaining cycle detected")
            });
            reject_promise_val(id, e);
            return;
        }
        // A native promise is still a thenable, so the spec routes it through
        // `NewPromiseResolveThenableJob` too — one microtask before the adoption
        // is even registered. (`await` does NOT pay this: V8's await optimization
        // subscribes to a native promise directly, which `await_value` mirrors.)
        with_host(|h| {
            h.queue_micro_native(Box::new(move || {
                subscribe_native(
                    vid,
                    Box::new(move |state, val| {
                        with_host(|h| h.settle_promise(id, state, val.clone()));
                        schedule_reactions(id);
                        Ok(())
                    }),
                );
                Ok(())
            }))
        });
        return;
    }
    // ECMA-262 27.2.1.3.2: any OBJECT carrying a callable `then` is assimilated
    // through a dedicated job — the promise adopts what `then` reports, it is
    // never fulfilled WITH the thenable itself.
    if let Some(then) = thenable_then(&value) {
        with_host(|h| {
            h.queue_micro_native(Box::new(move || resolve_thenable_job(id, value, then)))
        });
        return;
    }
    with_host(|h| h.settle_promise(id, PromiseState::Fulfilled, value));
    schedule_reactions(id);
}

/// `value.then` if `value` is an object with a callable `then` — the test that
/// makes a value a *thenable*. Primitives (and objects without one) are `None`.
fn thenable_then(value: &Value) -> Option<Value> {
    if !with_host(|h| matches!(h.get(value), Some(JsObj::Object(_)))) {
        return None;
    }
    let then = with_host(|h| lookup_chain(h, value, "then"))?;
    with_host(|h| is_callable(h, &then)).then_some(then)
}

/// `NewPromiseResolveThenableJob`: hand the thenable this promise's own resolve /
/// reject continuations and let it settle us. A throw out of `then` rejects.
fn resolve_thenable_job(id: u32, thenable: Value, then: Value) -> Result<(), String> {
    let res = with_host(|h| h.alloc(JsObj::Builtin(format!("@@presolve:{id}"))));
    let rej = with_host(|h| h.alloc(JsObj::Builtin(format!("@@preject:{id}"))));
    if let Err(e) = invoke(&then, vec![res, rej], Some(thenable)) {
        let ev = take_exc_or_error(&e);
        reject_promise_val(id, ev);
    }
    Ok(())
}

pub fn reject_promise_val(id: u32, value: Value) {
    if with_host(|h| h.promise_state(id)) != PromiseState::Pending {
        return;
    }
    with_host(|h| {
        h.settle_promise(id, PromiseState::Rejected, value);
        h.pending_rejections.push(id);
    });
    schedule_reactions(id);
}

/// Report every promise that settled rejected since the last checkpoint and
/// still has no handler. Node's default is `--unhandled-rejections=throw`: the
/// rejection becomes an uncaught exception (stderr + exit 1) unless a
/// `process.on('unhandledRejection')` listener takes it.
fn check_unhandled_rejections() -> Result<(), String> {
    loop {
        let ids: Vec<u32> = with_host(|h| std::mem::take(&mut h.pending_rejections));
        if ids.is_empty() {
            return Ok(());
        }
        for id in ids {
            let unhandled = with_host(|h| {
                h.promise_state(id) == PromiseState::Rejected && !h.promises[id as usize].handled
            });
            if !unhandled {
                continue;
            }
            // Report each promise at most once, however many checkpoints pass.
            with_host(|h| h.promise_mark_handled(id));
            let val = with_host(|h| h.promise_value(id));
            let listeners = with_host(|h| {
                h.process_listeners
                    .get("unhandledRejection")
                    .cloned()
                    .unwrap_or_default()
            });
            if listeners.is_empty() {
                let msg = with_host(|h| crate::builtins::error_string(h, &val));
                with_host(|h| h.exc = Some(val));
                return Err(msg);
            }
            let promise = with_host(|h| h.alloc(JsObj::Promise { id }));
            for f in listeners {
                invoke(&f, vec![val.clone(), promise.clone()], None)?;
            }
        }
    }
}

/// Drain a settled promise's reactions into microtasks.
fn schedule_reactions(id: u32) {
    let reactions = with_host(|h| h.take_reactions(id));
    let state = with_host(|h| h.promise_state(id));
    let value = with_host(|h| h.promise_value(id));
    for r in reactions {
        let value = value.clone();
        match r {
            PromiseReaction::Native(f) => {
                with_host(|h| h.queue_micro_native(Box::new(move || f(state, value))));
            }
            PromiseReaction::Js {
                on_ful,
                on_rej,
                result,
            } => {
                with_host(|h| {
                    h.queue_micro_native(Box::new(move || {
                        run_js_reaction(state, value, on_ful, on_rej, result)
                    }))
                });
            }
        }
    }
}

/// Run a `.then` reaction: call the appropriate handler and settle the result
/// promise with its outcome (or pass through if there is no handler).
fn run_js_reaction(
    state: PromiseState,
    value: Value,
    on_ful: Value,
    on_rej: Value,
    result: Value,
) -> Result<(), String> {
    let rid = match with_host(|h| h.promise_id(&result)) {
        Some(i) => i,
        None => return Ok(()),
    };
    let handler = if state == PromiseState::Rejected {
        on_rej
    } else {
        on_ful
    };
    if with_host(|h| is_callable(h, &handler)) {
        match invoke(&handler, vec![value], None) {
            Ok(r) => resolve_promise_val(rid, r),
            Err(e) => reject_promise_val(rid, take_exc_or_error(&e)),
        }
    } else if state == PromiseState::Rejected {
        reject_promise_val(rid, value);
    } else {
        resolve_promise_val(rid, value);
    }
    Ok(())
}

/// The JS value of a just-caught error: the live `exc` (a real thrown value) or a
/// synthesized `Error` from the internal message.
pub fn take_exc_or_error(e: &str) -> Value {
    with_host(|h| {
        h.error.take();
        h.exc
            .take()
            .unwrap_or_else(|| crate::builtins::synth_error(h, e))
    })
}

/// Register a user `.then` reaction (JS handlers + result promise).
pub fn promise_then(p: &Value, on_ful: Value, on_rej: Value) -> Value {
    let id = match with_host(|h| h.promise_id(p)) {
        Some(i) => i,
        None => return Value::Undef,
    };
    with_host(|h| h.promise_mark_handled(id));
    let result = with_host(|h| h.new_promise());
    let reaction = PromiseReaction::Js {
        on_ful,
        on_rej,
        result: result.clone(),
    };
    let state = with_host(|h| h.promise_state(id));
    if state == PromiseState::Pending {
        with_host(|h| h.add_reaction(id, reaction));
    } else {
        let value = with_host(|h| h.promise_value(id));
        if let PromiseReaction::Js {
            on_ful,
            on_rej,
            result,
        } = reaction
        {
            with_host(|h| {
                h.queue_micro_native(Box::new(move || {
                    run_js_reaction(state, value, on_ful, on_rej, result)
                }))
            });
        }
    }
    result
}
