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
use std::rc::Rc;

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
#[derive(Clone)]
pub struct FuncDef {
    pub name: String,
    /// Parameter binding templates (destructuring lowered by the compiler into
    /// the body prologue; here we only track the simple arg slots).
    pub params: Vec<ParamSlot>,
    pub chunk: Chunk,
    pub is_arrow: bool,
}

/// One parameter slot. `name` is the simple bound name; a destructuring pattern
/// is lowered to a synthetic `.arg{i}` name plus body prologue code.
#[derive(Clone)]
pub struct ParamSlot {
    pub name: String,
    /// True for the `...rest` collector.
    pub rest: bool,
    /// True if this slot has a default expression (applied in the body prologue).
    pub has_default: bool,
}

/// A compiled `try`/`catch`/`finally` block. Bodies are bare chunks run in the
/// current scope.
#[derive(Clone)]
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
}

// ── environments ─────────────────────────────────────────────────────────────

/// A local-variable environment, shared (by `Rc`) between a frame and any nested
/// function that captures it.
pub struct EnvData {
    pub vars: IndexMap<String, Value>,
    pub parent: Option<Env>,
}
pub type Env = Rc<RefCell<EnvData>>;

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
}

impl Default for JsHost {
    fn default() -> Self {
        Self::new()
    }
}

impl JsHost {
    pub fn new() -> JsHost {
        let module_env = new_env(None);
        let mut h = JsHost {
            heap: Vec::new(),
            funcs: Vec::new(),
            tries: Vec::new(),
            globals: IndexMap::new(),
            frames: vec![Frame {
                env: module_env,
                this_obj: None,
            }],
            error: None,
            exc: None,
            signal: None,
            null_val: Value::Undef,
        };
        h.null_val = h.alloc(JsObj::Null);
        h
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

// ── the fusevm run plumbing ──────────────────────────────────────────────────

/// Register every node-js builtin + the numeric hook on a VM, then run it.
pub fn run_chunk_on(chunk: Chunk) -> Result<Value, String> {
    let mut vm = VM::new(chunk);
    crate::builtins::install(&mut vm);
    vm.set_numeric_hook(std::sync::Arc::new(|op, a, b| {
        crate::builtins::numeric_hook(op, a, b)
    }));
    vm.enable_tracing_jit();
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

/// Run the top-level program chunk.
pub fn run_main(chunk: Chunk) -> Result<Value, String> {
    let r = run_chunk_on(chunk);
    with_host(|h| h.signal = None);
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
                Some(JsObj::Func(_)) | Some(JsObj::Builtin(_)) | Some(JsObj::BoundMethod { .. }) => {
                    "function"
                }
                _ => "object", // arrays, objects, null
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
                Some(JsObj::BoundMethod { .. }) => "function () { [native code] }".into(),
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
                Some(JsObj::Array(items)) => {
                    if items.is_empty() {
                        return "[]".into();
                    }
                    let inner: Vec<String> = items.iter().map(|x| self.inspect(x)).collect();
                    format!("[ {} ]", inner.join(", "))
                }
                Some(JsObj::Object(props)) => {
                    if props.is_empty() {
                        return "{}".into();
                    }
                    let inner: Vec<String> = props
                        .iter()
                        .map(|(k, val)| format!("{}: {}", fmt_key(k), self.inspect(val)))
                        .collect();
                    format!("{{ {} }}", inner.join(", "))
                }
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
                _ => "undefined".into(),
            },
            _ => "undefined".into(),
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
                    let s = format!("{}{}", self.str_of(a), self.str_of(b));
                    Ok(self.new_str(s))
                } else {
                    Ok(Value::Float(self.to_number(a) + self.to_number(b)))
                }
            }
            Sub => Ok(Value::Float(self.to_number(a) - self.to_number(b))),
            Mul => Ok(Value::Float(self.to_number(a) * self.to_number(b))),
            Div => Ok(Value::Float(self.to_number(a) / self.to_number(b))),
            Mod => Ok(Value::Float(js_mod(self.to_number(a), self.to_number(b)))),
            Pow => Ok(Value::Float(self.to_number(a).powf(self.to_number(b)))),
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
            Value::Obj(_) => !matches!(self.get(v), Some(JsObj::Null) | None),
            _ => false,
        }
    }

    /// Relational comparison (`< <= > >=`) with JS coercion: string/string is
    /// lexicographic, otherwise numeric (NaN yields false).
    fn relational(&self, op: NumOp, a: &Value, b: &Value) -> bool {
        use std::cmp::Ordering;
        let ord = if let (Some(x), Some(y)) = (self.as_str(a), self.as_str(b)) {
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

    /// Bitwise/shift ops with JS ToInt32/ToUint32 semantics.
    pub fn bitwise(&self, tag: i64, a: &Value, b: &Value) -> Value {
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
        Value::Float(r as f64)
    }
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
    /// Collect an iterable into a vector of values (arrays and strings).
    pub fn iter_vec(&mut self, v: &Value) -> Result<Vec<Value>, String> {
        match self.get(v) {
            Some(JsObj::Array(items)) => Ok(items.clone()),
            Some(JsObj::Str(s)) => {
                let chars: Vec<String> = s.chars().map(|c| c.to_string()).collect();
                Ok(chars.into_iter().map(|c| self.new_str(c)).collect())
            }
            Some(JsObj::Iter { items, idx }) => Ok(items[*idx..].to_vec()),
            _ => Err(type_error(&format!(
                "{} is not iterable",
                self.type_of(v)
            ))),
        }
    }

    /// Enumerable keys of an object/array (for `for-in`).
    pub fn enum_keys(&mut self, v: &Value) -> Vec<Value> {
        let keys: Vec<String> = match self.get(v) {
            Some(JsObj::Object(props)) => props.keys().cloned().collect(),
            Some(JsObj::Array(items)) => (0..items.len()).map(|i| i.to_string()).collect(),
            _ => Vec::new(),
        };
        keys.into_iter().map(|k| self.new_str(k)).collect()
    }
}

// ── function invocation ──────────────────────────────────────────────────────

/// Resolve a bare name and call it (`f(args)`, `parseInt(args)`).
pub fn call_named(name: &str, args: Vec<Value>) -> Result<Value, String> {
    if let Some(v) = with_host(|h| h.read_name(name)) {
        return invoke(&v, args, None);
    }
    if crate::builtins::is_known_builtin(name) {
        return crate::builtins::call_builtin_function(name, args);
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
    // A method stored as a property (object method, `this` = recv).
    if let Some(JsObj::Object(props)) = with_host(|h| h.get(recv).cloned()) {
        if let Some(f) = props.get(name).cloned() {
            return invoke(&f, args, Some(recv.clone()));
        }
    }
    // Type methods (array/string/number methods).
    crate::builtins::call_type_method(recv, name, args)
}

/// Call any callable value.
pub fn invoke(callable: &Value, args: Vec<Value>, this: Option<Value>) -> Result<Value, String> {
    let obj = with_host(|h| h.get(callable).cloned());
    match obj {
        Some(JsObj::Builtin(name)) => crate::builtins::call_builtin_function(&name, args),
        Some(JsObj::Func(fv)) => run_user_func(&fv, args, this),
        Some(JsObj::BoundMethod { recv, name }) => call_method(&recv, &name, args),
        _ => Err(type_error(&format!(
            "{} is not a function",
            with_host(|h| h.str_of(callable))
        ))),
    }
}

/// Execute a user function/closure body on a fresh frame.
pub fn run_user_func(fv: &FuncVal, args: Vec<Value>, this: Option<Value>) -> Result<Value, String> {
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
    with_host(|h| {
        h.frames.push(Frame {
            env,
            this_obj: this_val,
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
    let inst = with_host(|h| h.new_object(IndexMap::new()));
    let obj = with_host(|h| h.get(ctor).cloned());
    match obj {
        Some(JsObj::Func(fv)) => {
            let r = run_user_func(&fv, args, Some(inst.clone()))?;
            // If the constructor returned an object/array, use it; else the instance.
            if matches!(
                with_host(|h| h.get(&r).cloned()),
                Some(JsObj::Object(_)) | Some(JsObj::Array(_))
            ) {
                Ok(r)
            } else {
                Ok(inst)
            }
        }
        Some(JsObj::Builtin(name)) => crate::builtins::construct_builtin(&name, args),
        _ => Err(type_error("not a constructor")),
    }
}
