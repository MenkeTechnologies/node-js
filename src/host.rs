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
    BoundMethod { recv: Value, name: String },
    /// The single canonical `null`.
    Null,
    /// A live iterator over a sequence, with a cursor.
    Iter { items: Vec<Value>, idx: usize },
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
    Symbol { desc: Option<String>, id: u64 },
    /// A `Map` (or `WeakMap` when `weak`): insertion-ordered key→value entries.
    Map { entries: IndexMap<MapKey, (Value, Value)>, weak: bool },
    /// A `Set` (or `WeakSet` when `weak`): insertion-ordered unique values.
    Set { entries: IndexMap<MapKey, Value>, weak: bool },
    /// A live generator, backed by a stackful `corosensei` coroutine in
    /// `JsHost.generators`.
    Generator { id: u32 },
    /// A Promise, backed by a `PromiseCell` in `JsHost.promises`.
    Promise { id: u32 },
    /// An arbitrary-precision `BigInt` (`typeof === "bigint"`).
    BigInt(num_bigint::BigInt),
    /// A compiled regular expression (`/pat/flags` or `new RegExp(...)`).
    RegExp(Box<RegExpObj>),
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
    /// `lastIndex`, in UTF-16 code units-approximated-as-chars; advanced by
    /// `exec`/`test` under the `g`/`y` flags.
    pub last_index: usize,
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
    /// Instance field initializers: `(name, thunk_fn)`, run per-instance after
    /// `super()` (or at construction start for a base class).
    pub fields: Vec<(String, Value)>,
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

fn new_env(parent: Option<Env>) -> Env {
    Rc::new(RefCell::new(EnvData {
        vars: IndexMap::new(),
        parent,
    }))
}

/// One function activation.
pub struct Frame {
    pub env: Env,
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
}

/// A non-local control signal.
#[derive(Clone)]
pub enum Signal {
    Return(Value),
    Break,
    Continue,
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
    pub error: Option<String>,
    /// The in-flight thrown value, if any (JS `throw`).
    pub exc: Option<Value>,
    pub signal: Option<Signal>,
    /// The canonical `null` handle (allocated once).
    null_val: Value,
    /// `[[Prototype]]` link per heap object, by heap index. Absent = default
    /// (`Object.prototype` for objects, `null` for the root).
    protos: HashMap<u32, Value>,
    /// Own properties of function objects (functions are objects in JS): a live
    /// closure's `name`/`prototype`/static-ish members. Keyed by heap index.
    fn_props: HashMap<u32, IndexMap<String, Value>>,
    /// Accessor (getter/setter) properties per owning object, by heap index then
    /// key: `(get, set)`. Class `get x()`/`set x()` install here on the prototype.
    accessors: HashMap<u32, IndexMap<String, Accessor>>,
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
    /// `Symbol.for` registry: description → symbol value.
    symbol_registry: HashMap<String, Value>,
    /// Monotonic id source for fresh `Symbol()` values.
    next_symbol: u64,
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

/// A scheduled macrotask (`setTimeout`/`setImmediate`). Ordering is by `(delay,
/// seq)` — a deterministic virtual clock, never wall time.
pub struct Timer {
    pub id: u64,
    pub delay: f64,
    pub seq: u64,
    pub callback: Value,
    pub args: Vec<Value>,
    pub cancelled: bool,
    /// Real wall-clock deadline (`now + delay`), used only on the blocking I/O
    /// path (`open_handles > 0`). With no open handles the loop stays on the
    /// deterministic virtual clock and ignores this.
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
}

/// A forced completion pushed into a suspended generator by `.return()`/`.throw()`.
enum GenInject {
    Return(Value),
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
        let module_env = new_env(None);
        let (io_tx, io_rx) = std::sync::mpsc::channel();
        let mut h = JsHost {
            heap: Vec::new(),
            funcs: Vec::new(),
            tries: Vec::new(),
            globals: IndexMap::new(),
            frames: vec![Frame {
                env: module_env,
                this_obj: None,
                new_target: None,
                home_class: None,
                line: 0,
                owner: None,
            }],
            error: None,
            exc: None,
            signal: None,
            null_val: Value::Undef,
            protos: HashMap::new(),
            fn_props: HashMap::new(),
            accessors: HashMap::new(),
            builtin_statics: HashMap::new(),
            object_proto: Value::Undef,
            proto_class: HashMap::new(),
            class_registry: HashMap::new(),
            error_protos: HashMap::new(),
            symbol_registry: HashMap::new(),
            next_symbol: 1,
            generators: Vec::new(),
            promises: Vec::new(),
            microtasks: std::collections::VecDeque::new(),
            nextticks: std::collections::VecDeque::new(),
            macrotasks: Vec::new(),
            next_timer: 1,
            io_tx,
            io_rx: Some(io_rx),
            open_handles: 0,
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
    /// Set `v`'s `[[Prototype]]` to `proto` (or clear it if `proto` is null).
    pub fn set_proto(&mut self, v: &Value, proto: Value) {
        if let Value::Obj(i) = v {
            if self.is_null(&proto) || matches!(proto, Value::Undef) {
                self.protos.remove(i);
            } else {
                self.protos.insert(*i, proto);
            }
        }
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
        match self.class_of(obj) {
            Some(c) => match self.get(&c) {
                Some(JsObj::Class(cv)) => cv.name.clone(),
                _ => String::new(),
            },
            None => String::new(),
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
            self.fn_props.entry(*i).or_default().insert(name.to_string(), val);
        }
    }
    /// A user-assigned static on a builtin namespace (`Error.prepareStackTrace`).
    pub fn builtin_static(&self, ns: &str, name: &str) -> Option<Value> {
        self.builtin_statics.get(ns).and_then(|m| m.get(name).cloned())
    }
    /// Assign a static on a builtin namespace (persists across fresh `Builtin`
    /// handles for the same namespace).
    pub fn set_builtin_static(&mut self, ns: &str, name: &str, val: Value) {
        self.builtin_statics.entry(ns.to_string()).or_default().insert(name.to_string(), val);
    }
    pub fn fn_prop_keys(&self, v: &Value) -> Vec<String> {
        if let Value::Obj(i) = v {
            self.fn_props.get(i).map(|m| m.keys().cloned().collect()).unwrap_or_default()
        } else {
            Vec::new()
        }
    }

    /// Install an accessor `(get, set)` for `key` on the object `owner`.
    pub fn set_accessor(&mut self, owner: &Value, key: &str, get: Option<Value>, set: Option<Value>) {
        if let Value::Obj(i) = owner {
            let slot = self.accessors.entry(*i).or_default().entry(key.to_string()).or_insert((None, None));
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

    /// A fresh unique `Symbol(desc)` value.
    pub fn new_symbol(&mut self, desc: Option<String>) -> Value {
        let id = self.next_symbol;
        self.next_symbol += 1;
        self.alloc(JsObj::Symbol { desc, id })
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
    /// The internal property-key string for a value used as a key. A `Symbol`
    /// maps to a stable per-symbol string so symbol-keyed props round-trip;
    /// `Symbol.iterator` maps to the sentinel `@@iterator`.
    pub fn property_key(&self, v: &Value) -> String {
        if let Some(JsObj::Symbol { desc, id }) = self.get(v) {
            if desc.as_deref() == Some("@@Symbol.iterator") {
                return "@@iterator".to_string();
            }
            if desc.as_deref() == Some("@@Symbol.asyncIterator") {
                return "@@asyncIterator".to_string();
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
    pub fn new_str(&mut self, s: impl Into<String>) -> Value {
        self.alloc(JsObj::Str(s.into()))
    }
    pub fn new_array(&mut self, items: Vec<Value>) -> Value {
        self.alloc(JsObj::Array(items))
    }
    pub fn new_object(&mut self, props: IndexMap<String, Value>) -> Value {
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

    /// Declare a new binding in the current scope (`let`/`const`/`var`).
    pub fn declare_name(&mut self, name: &str, val: Value) {
        if self.frames.len() == 1 {
            self.globals.insert(name.to_string(), val);
        } else {
            self.cur_env().borrow_mut().vars.insert(name.to_string(), val);
        }
    }
    pub fn set_global(&mut self, name: &str, val: Value) {
        self.globals.insert(name.to_string(), val);
    }
    pub fn del_name(&mut self, name: &str) {
        if self.cur_env().borrow_mut().vars.shift_remove(name).is_some() {
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
    pub fn super_context(&self) -> (Option<Value>, Vec<(String, Value)>) {
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
        let parent = match self.current_home_class().and_then(|cv| match self.get(&cv) {
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
                | Some(JsObj::Builtin(_))
                | Some(JsObj::BoundMethod { .. })
                | Some(JsObj::BoundFunc { .. })
                | Some(JsObj::Class(_)) => "function",
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
                Some(JsObj::Object(_)) => "[object Object]".into(),
                Some(JsObj::Func(f)) => {
                    let name = self.funcs.get(f.def_id).map(|d| d.name.clone()).unwrap_or_default();
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
                    if items.is_empty() {
                        return "[]".into();
                    }
                    let inner: Vec<String> =
                        items.iter().map(|x| self.inspect_lvl(x, indent + 2)).collect();
                    self.render_array(&inner, items, indent)
                }
                Some(JsObj::Object(props)) => {
                    // Instances print with their constructor name as a prefix
                    // (`C { x: 1 }`); plain objects have none.
                    let prefix = match self.ctor_name(v) {
                        n if n.is_empty() || n == "Object" => String::new(),
                        n => format!("{n} "),
                    };
                    // Skip internal symbol-keyed props (`@@…`) in the display.
                    let shown: Vec<(&String, &Value)> =
                        props.iter().filter(|(k, _)| !k.starts_with("@@") && !k.starts_with('#')).collect();
                    if shown.is_empty() {
                        return format!("{prefix}{{}}");
                    }
                    let inner: Vec<String> = shown
                        .iter()
                        .map(|(k, val)| format!("{}: {}", fmt_key(k), self.inspect_lvl(val, indent + 2)))
                        .collect();
                    format!("{prefix}{{ {} }}", inner.join(", "))
                }
                Some(JsObj::Symbol { desc, .. }) => match desc {
                    Some(d) => format!("Symbol({d})"),
                    None => "Symbol()".into(),
                },
                Some(JsObj::Class(c)) => {
                    if c.parent.is_some() {
                        let pname = c
                            .parent
                            .as_ref()
                            .map(|p| self.callable_name(p))
                            .unwrap_or_default();
                        format!("[class {} extends {}]", c.name, pname)
                    } else {
                        format!("[class {}]", c.name)
                    }
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
                        PromiseState::Fulfilled => format!("Promise {{ {} }}", self.inspect(&c.value)),
                        PromiseState::Rejected => {
                            format!("Promise {{ <rejected> {} }}", self.inspect(&c.value))
                        }
                    },
                    None => "Promise { <pending> }".into(),
                },
                Some(JsObj::Func(f)) => {
                    let name = self.funcs.get(f.def_id).map(|d| d.name.clone()).unwrap_or_default();
                    if name.is_empty() {
                        "[Function (anonymous)]".into()
                    } else {
                        format!("[Function: {name}]")
                    }
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

    /// Render a non-empty array's already-formatted element strings, applying
    /// Node's `util.inspect` layout: a single line when it fits, else a multi-line
    /// grid via `groupArrayElements` (for >6 entries), else one element per line.
    /// `values` is the raw element list (drives numeric right-alignment); `indent`
    /// is the array's own indentation level.
    fn render_array(&self, output: &[String], values: &[Value], indent: usize) -> String {
        // Group array elements together if the array has more than six entries.
        let entries = output.len();
        let (lines, grouped) = if entries > 6 {
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

    /// The `.name` of any callable (function/class/builtin/bound).
    pub fn callable_name(&self, v: &Value) -> String {
        // A user-set `.name` own property wins.
        if let Some(n) = self.fn_prop(v, "name") {
            return self.str_of(&n);
        }
        match self.get(v) {
            Some(JsObj::Func(f)) => self.funcs.get(f.def_id).map(|d| d.name.clone()).unwrap_or_default(),
            Some(JsObj::Class(c)) => c.name.clone(),
            Some(JsObj::Builtin(n)) => n.rsplit('.').next().unwrap_or(n).to_string(),
            Some(JsObj::BoundFunc { target, .. }) => format!("bound {}", self.callable_name(target)),
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
            Value::Obj(_) => !matches!(self.get(v), Some(JsObj::Null) | Some(JsObj::BigInt(_)) | None),
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
            _ => return Err(type_error(
                "Cannot mix BigInt and other types, use explicit conversions",
            )),
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
            _ => return Err(type_error(
                "Cannot mix BigInt and other types, use explicit conversions",
            )),
        };
        let r = match tag {
            binop::BITAND => x & y,
            binop::BITOR => x | y,
            binop::BITXOR => x ^ y,
            binop::SHL => {
                let n = num_traits::ToPrimitive::to_i64(&y).unwrap_or(0);
                if n >= 0 { x << (n as usize) } else { x >> ((-n) as usize) }
            }
            binop::SHR => {
                let n = num_traits::ToPrimitive::to_i64(&y).unwrap_or(0);
                if n >= 0 { x >> (n as usize) } else { x << ((-n) as usize) }
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
        return i64::from_str_radix(hex, 16).map(|n| n as f64).unwrap_or(f64::NAN);
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
        ((approx_char_heights * biased_max * output_length as f64).sqrt() / biased_max).round() as i64,
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

/// Quote a string the way `util.inspect` does (single quotes, escaped).
fn quote_str(s: &str) -> String {
    let mut out = String::from("'");
    for c in s.chars() {
        match c {
            '\'' => out.push_str("\\'"),
            '\\' => out.push_str("\\\\"),
            '\n' => out.push_str("\\n"),
            '\t' => out.push_str("\\t"),
            '\r' => out.push_str("\\r"),
            _ => out.push(c),
        }
    }
    out.push('\'');
    out
}

/// Render an object key: bare if it is a valid identifier, quoted otherwise.
fn fmt_key(k: &str) -> String {
    let ok = !k.is_empty()
        && k.chars().next().map(|c| c.is_ascii_alphabetic() || c == '_' || c == '$').unwrap_or(false)
        && k.chars().all(|c| c.is_ascii_alphanumeric() || c == '_' || c == '$');
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
                Ok(pairs.into_iter().map(|(k, v)| self.new_array(vec![k, v])).collect())
            }
            _ => Err(type_error(&format!("{} is not iterable", self.type_of(v)))),
        }
    }

    /// Enumerable string keys of an object/array (for `for-in`). Internal
    /// symbol-keyed props (`@@…`) are not enumerable.
    pub fn enum_keys(&mut self, v: &Value) -> Vec<Value> {
        let keys: Vec<String> = match self.get(v) {
            Some(JsObj::Object(props)) => props.keys().filter(|k| !k.starts_with("@@") && !k.starts_with('#')).cloned().collect(),
            Some(JsObj::Array(items)) => (0..items.len()).map(|i| i.to_string()).collect(),
            _ => Vec::new(),
        };
        keys.into_iter().map(|k| self.new_str(k)).collect()
    }
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
    if let Some(JsObj::Builtin(ns)) = with_host(|h| h.get(recv).cloned()) {
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
    if matches!(with_host(|h| h.get(recv).cloned()), Some(JsObj::Object(_))) {
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
        return Err(type_error(&format!("{name} is not a function")));
    }
    // Function value methods: call / apply / bind, then any static method stored
    // on the function object.
    if matches!(
        with_host(|h| h.get(recv).cloned()),
        Some(JsObj::Func(_))
            | Some(JsObj::Class(_))
            | Some(JsObj::BoundFunc { .. })
            | Some(JsObj::BoundMethod { .. })
            | Some(JsObj::Builtin(_))
    ) {
        if let Some(r) = crate::builtins::function_builtin_method(recv, name, &args)? {
            return Ok(r);
        }
        // A static method (own or inherited): `this` is the constructor (`recv`).
        let stat = if matches!(with_host(|h| h.get(recv).cloned()), Some(JsObj::Class(_))) {
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
        if matches!(with_host(|h| h.get(recv).cloned()), Some(JsObj::Builtin(_)))
            && crate::builtins::is_object_builtin_method(name)
        {
            return crate::builtins::object_builtin_method(recv, name, args);
        }
    }
    // Type methods (array/string/number, Map/Set/Symbol/generator methods).
    crate::builtins::call_type_method(recv, name, args)
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
        Some(JsObj::Builtin(name)) => crate::builtins::call_builtin_function(&name, args),
        Some(JsObj::Func(fv)) => run_user_func(&fv, args, this),
        Some(JsObj::BoundMethod { recv, name }) => call_method(&recv, &name, args),
        Some(JsObj::BoundFunc { target, this: bthis, args: pre }) => {
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
    let this_val = if fv.is_arrow {
        fv.this.clone()
    } else {
        this
    };
    // A generator function does not run its body on call — it returns a suspended
    // generator over the already-bound frame.
    if def.is_generator {
        return Ok(make_generator(def.chunk.clone(), env, this_val, fv.home_class.clone()));
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
            env,
            this_obj: this_val,
            new_target,
            home_class: home,
            line: 0,
            owner: Some(def.name.clone()),
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
    // `arguments` array (simple approximation).
    let args_arr = with_host(|h| h.new_array(args));
    vars.entry("arguments".to_string()).or_insert(args_arr);
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
        Some(JsObj::BoundFunc { target, args: pre, .. }) => {
            let mut all = pre;
            all.extend(args);
            construct_nt(&target, all, new_target)
        }
        _ => Err(type_error(&format!("{} is not a constructor", with_host(|h| h.str_of(ctor))))),
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
fn construct_class(class_val: &Value, args: Vec<Value>, new_target: Value) -> Result<Value, String> {
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
    for (name, thunk) in &cv.fields {
        // The thunk is an arrow capturing the class scope; run it with `this`=inst
        // so `this.other`-referencing initializers work.
        let val = invoke(thunk, Vec::new(), Some(inst.clone()))?;
        with_host(|h| {
            if let Some(JsObj::Object(props)) = h.get_mut(inst) {
                props.insert(name.clone(), val);
            }
        });
    }
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
                if let Some(JsObj::Object(props)) = h.get_mut(inst) {
                    for (k, v) in entries {
                        props.insert(k, v);
                    }
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
        let parent_opt = if matches!(parent, Value::Undef) { None } else { Some(parent.clone()) };
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
                _ => h.fn_prop(p, "prototype").unwrap_or_else(|| h.object_proto()),
            },
            None => h.object_proto(),
        };
        let proto = h.new_object(IndexMap::new());
        h.set_proto(&proto, parent_proto);
        let ctor_opt = if matches!(ctor, Value::Undef) { None } else { Some(ctor.clone()) };
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
    });
}

/// Register an instance-field initializer thunk on a class (`DEF_FIELD`).
pub fn define_field(class_val: &Value, name: &str, thunk: Value) {
    with_host(|h| {
        if let Some(JsObj::Class(c)) = h.get_mut(class_val) {
            c.fields.push((name.to_string(), thunk));
        }
    });
}

/// The `[[Prototype]]` object a constructor value hands to its instances
/// (`Ctor.prototype`), for `instanceof`.
fn ctor_prototype(h: &JsHost, ctor: &Value) -> Option<Value> {
    match h.get(ctor) {
        Some(JsObj::Class(c)) => Some(c.proto.clone()),
        Some(JsObj::Func(_)) => h.fn_prop(ctor, "prototype"),
        Some(JsObj::Builtin(name)) => h.error_protos.get(name).cloned(),
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
            Some(JsObj::Func(_)) | Some(JsObj::Class(_)) | Some(JsObj::Builtin(_)) | Some(JsObj::BoundFunc { .. })
        )
    });
    if !ctor_callable {
        return Err(type_error("Right-hand side of 'instanceof' is not callable"));
    }
    // Builtin constructors whose instances aren't prototype-linked in our model
    // (arrays/plain objects/functions) get a structural instanceof.
    if let Some(JsObj::Builtin(name)) = with_host(|h| h.get(ctor).cloned()) {
        let kind = with_host(|h| h.get(obj).cloned());
        match name.as_str() {
            "Array" => return Ok(matches!(kind, Some(JsObj::Array(_)))),
            "Function" => return Ok(with_host(|h| is_callable(h, obj))),
            "Object" => {
                // Everything object-typed except a null-prototype object is an
                // Object instance.
                let is_obj = matches!(
                    kind,
                    Some(JsObj::Object(_)) | Some(JsObj::Array(_)) | Some(JsObj::Func(_))
                        | Some(JsObj::Class(_)) | Some(JsObj::Map { .. }) | Some(JsObj::Set { .. })
                        | Some(JsObj::Promise { .. }) | Some(JsObj::Generator { .. })
                );
                if is_obj {
                    // A null-prototype object created via Object.create(null) is
                    // NOT an Object instance.
                    if matches!(kind, Some(JsObj::Object(_))) && with_host(|h| h.proto_of(obj)).is_none() {
                        // Distinguish an ordinary object (no explicit proto but
                        // conceptually Object.prototype) from Object.create(null):
                        // we can't, so treat a bare object as an Object instance.
                    }
                    return Ok(true);
                }
                return Ok(false);
            }
            _ => {}
        }
    }
    with_host(|h| h.ensure_error_protos());
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
    pub fn gen_done(&self, id: u32) -> bool {
        self.generators.get(id as usize).map(|g| g.done).unwrap_or(true)
    }
    fn gen_started(&self, id: u32) -> bool {
        self.generators.get(id as usize).map(|g| g.started).unwrap_or(false)
    }
}

/// Build a suspended generator whose body is `chunk`, run in a frame with the
/// already-bound `env`. Nothing executes until the first `gen_resume`.
fn make_generator(chunk: Chunk, env: Env, this_val: Option<Value>, home_class: Option<String>) -> Value {
    let home = home_class
        .as_ref()
        .and_then(|n| with_host(|h| h.class_registry.get(n).cloned()));
    let frame = Frame {
        env,
        this_obj: this_val,
        new_target: None,
        home_class: home,
        line: 0,
        owner: None,
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
    let gen_ctx = with_host(|h| h.install_gen_ctx(caller_ctx));
    with_host(|h| {
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
    // Sync-fallback iterator: drive it here, awaiting each yielded value.
    if let Some(JsObj::Iter { items, idx }) = with_host(|h| h.get(iterator).cloned()) {
        if idx >= items.len() {
            let rec = with_host(|h| {
                let mut m = IndexMap::new();
                m.insert("value".to_string(), Value::Undef);
                m.insert("done".to_string(), Value::Bool(true));
                h.new_object(m)
            });
            return Ok(promise_of(&rec));
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
                    let rec = with_host(|h| {
                        let mut m = IndexMap::new();
                        m.insert("value".to_string(), val.clone());
                        m.insert("done".to_string(), Value::Bool(false));
                        h.new_object(m)
                    });
                    resolve_promise_val(sid, rec);
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

/// `ToString(v)` with `ToPrimitive` method dispatch: an object with a user
/// `toString` (or `valueOf`) on its prototype chain has it invoked; everything
/// else uses the raw `str_of`. Returns a heap string value.
pub fn to_string_value(v: &Value) -> Result<Value, String> {
    if with_host(|h| matches!(h.get(v), Some(JsObj::Object(_)))) {
        // Prefer a user toString; fall back to valueOf if it returns a primitive.
        for m in ["toString", "valueOf"] {
            if let Some(f) = with_host(|h| lookup_chain(h, v, m)) {
                if with_host(|h| is_callable(h, &f)) {
                    let r = invoke(&f, Vec::new(), Some(v.clone()))?;
                    // A primitive result is used directly; an object result from
                    // toString is still stringified (matches V8's OrdinaryToPrimitive
                    // fallthrough closely enough for our surface).
                    if !matches!(with_host(|h| h.get(&r).cloned()), Some(JsObj::Object(_))) {
                        return Ok(with_host(|h| {
                            let s = h.str_of(&r);
                            h.new_str(s)
                        }));
                    }
                }
            }
        }
    }
    Ok(with_host(|h| {
        let s = h.str_of(v);
        h.new_str(s)
    }))
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
pub fn lookup_accessor(h: &JsHost, recv: &Value, key: &str) -> Option<(Option<Value>, Option<Value>)> {
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

/// The set of builtin error constructor names forming the error hierarchy.
pub const ERROR_NAMES: &[&str] = &[
    "Error",
    "TypeError",
    "RangeError",
    "SyntaxError",
    "ReferenceError",
    "EvalError",
    "URIError",
];

impl JsHost {
    /// Lazily build the builtin error prototype chain: `Error.prototype →
    /// Object.prototype`, and every specific error's prototype → `Error.prototype`.
    /// Populated once; instances link to these so `e instanceof TypeError` and
    /// `e instanceof Error` both hold.
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
            Some(def) => def.params.iter().take_while(|p| !p.rest && !p.has_default).count(),
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
    pub fn add_timer(&mut self, delay: f64, callback: Value, args: Vec<Value>) -> u64 {
        let id = self.next_timer;
        self.next_timer += 1;
        // Real deadline for the blocking I/O path; `setImmediate` (delay < 0) is
        // clamped to "now". Virtual-clock ordering still uses `delay`/`seq`.
        let deadline = Instant::now() + Duration::from_millis(delay.max(0.0) as u64);
        self.macrotasks.push(Timer {
            id,
            delay,
            seq: id,
            callback,
            args,
            cancelled: false,
            deadline,
        });
        id
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
                a.delay.partial_cmp(&b.delay).unwrap_or(std::cmp::Ordering::Equal).then(a.seq.cmp(&b.seq))
            })
            .map(|(i, _)| i);
        idx.map(|i| self.macrotasks.remove(i))
    }
    fn next_microtask(&mut self) -> Option<Task> {
        // nextTick drains before promise microtasks (Node ordering).
        self.nextticks.pop_front().or_else(|| self.microtasks.pop_front())
    }
    fn has_microtasks(&self) -> bool {
        !self.nextticks.is_empty() || !self.microtasks.is_empty()
    }
    fn has_macrotasks(&self) -> bool {
        self.macrotasks.iter().any(|t| !t.cancelled)
    }
}

/// Drive the event loop to quiescence.
///
/// Two regimes, selected per iteration by `open_handles`:
///
/// - **No open handles (pure script / timers only):** the original deterministic
///   virtual clock — drain microtasks, fire the earliest `(delay, seq)` timer
///   immediately (no real waiting), repeat until both queues empty, then EXIT.
///   Parity output and test speed are unchanged.
/// - **Open handles (a server is listening / sockets are live):** drain
///   microtasks, fire every timer whose real deadline has passed, then BLOCK on
///   the I/O channel (`recv_timeout` bounded by the next timer's real deadline,
///   or unbounded `recv` if no timers) and run the received `IoTask` on the main
///   thread. The host keeps its own `Sender`, so `recv` never disconnects while
///   the process should stay alive.
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
        // 1) Exhaust the microtask queue (nextTick before promise reactions).
        while let Some(task) = with_host(|h| h.next_microtask()) {
            task.run()?;
        }

        if with_host(|h| h.open_handles()) == 0 {
            // ── virtual-clock regime (unchanged behavior) ────────────────────
            match with_host(|h| h.pop_next_timer()) {
                Some(t) => {
                    invoke(&t.callback, t.args, None)?;
                }
                None => {
                    if !with_host(|h| h.has_microtasks()) {
                        break;
                    }
                }
            }
            if !with_host(|h| h.has_microtasks() || h.has_macrotasks()) {
                break;
            }
            continue;
        }

        // ── real-clock / blocking-I/O regime ─────────────────────────────────
        let now = Instant::now();
        if let Some(t) = with_host(|h| h.pop_due_timer(now)) {
            invoke(&t.callback, t.args, None)?;
            continue; // re-drain microtasks, re-check deadlines
        }
        // Nothing due and no pending microtasks: block for the next I/O event,
        // bounded by the soonest timer deadline so due timers still fire on time.
        let rx = rx.expect("blocking-I/O regime requires the I/O receiver");
        let timeout = with_host(|h| h.next_timer_timeout(now));
        let recv = match timeout {
            Some(d) => rx.recv_timeout(d),
            None => rx.recv().map_err(|_| std::sync::mpsc::RecvTimeoutError::Disconnected),
        };
        match recv {
            Ok(task) => task()?,
            Err(std::sync::mpsc::RecvTimeoutError::Timeout) => {} // a timer is now due
            Err(std::sync::mpsc::RecvTimeoutError::Disconnected) => break, // no senders left
        }
    }
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
                    let tag = if state == PromiseState::Rejected { 1.0 } else { 0.0 };
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
    let packet = gen_yield(awaited)?;
    let items = with_host(|h| h.iter_vec(&packet)).unwrap_or_default();
    let tag = items.first().map(|v| with_host(|h| h.to_number(v))).unwrap_or(0.0);
    let val = items.get(1).cloned().unwrap_or(Value::Undef);
    if tag == 1.0 {
        with_host(|h| h.exc = Some(val.clone()));
        Err(with_host(|h| crate::builtins::error_string(h, &val)))
    } else {
        Ok(val)
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
            let e = with_host(|h| crate::builtins::synth_error(h, "TypeError: Chaining cycle detected"));
            reject_promise_val(id, e);
            return;
        }
        subscribe_native(
            vid,
            Box::new(move |state, val| {
                with_host(|h| h.settle_promise(id, state, val.clone()));
                schedule_reactions(id);
                Ok(())
            }),
        );
        return;
    }
    with_host(|h| h.settle_promise(id, PromiseState::Fulfilled, value));
    schedule_reactions(id);
}

pub fn reject_promise_val(id: u32, value: Value) {
    if with_host(|h| h.promise_state(id)) != PromiseState::Pending {
        return;
    }
    with_host(|h| h.settle_promise(id, PromiseState::Rejected, value));
    schedule_reactions(id);
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
            PromiseReaction::Js { on_ful, on_rej, result } => {
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
fn run_js_reaction(state: PromiseState, value: Value, on_ful: Value, on_rej: Value, result: Value) -> Result<(), String> {
    let rid = match with_host(|h| h.promise_id(&result)) {
        Some(i) => i,
        None => return Ok(()),
    };
    let handler = if state == PromiseState::Rejected { on_rej } else { on_ful };
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
        h.exc.take().unwrap_or_else(|| crate::builtins::synth_error(h, e))
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
        if let PromiseReaction::Js { on_ful, on_rej, result } = reaction {
            with_host(|h| {
                h.queue_micro_native(Box::new(move || {
                    run_js_reaction(state, value, on_ful, on_rej, result)
                }))
            });
        }
    }
    result
}
