//! Builtin op handlers (compiler-emitted `CallBuiltin` ids) plus the JS standard
//! library (`console`, `Math`, `JSON`, `Object`, array/string methods) reachable
//! from the host. Handlers pop their arguments off the VM operand stack and
//! return the result value, which the VM pushes back.

use crate::host::{self, ops, with_host, FuncVal, JsObj, ObjKind};
use fusevm::{NumOp, Value, VM};
use indexmap::IndexMap;

/// Register every node-js builtin id on a VM.
pub fn install(vm: &mut VM) {
    vm.register_builtin(ops::GETLOCAL, b_getlocal);
    vm.register_builtin(ops::SETLOCAL, b_setlocal);
    vm.register_builtin(ops::SETLOCAL_STRICT, b_setlocal_strict);
    vm.register_builtin(ops::DECLARE, b_declare);
    vm.register_builtin(ops::DECLARE_CONST, b_declare_const);
    vm.register_builtin(ops::MARK_HOLE, b_mark_hole);
    vm.register_builtin(ops::DELNAME, b_delname);
    vm.register_builtin(ops::GETATTR, b_getattr);
    vm.register_builtin(ops::SETATTR, b_setattr);
    vm.register_builtin(ops::GETITEM, b_getitem);
    vm.register_builtin(ops::SETITEM, b_setitem);
    vm.register_builtin(ops::DELITEM, b_delitem);
    vm.register_builtin(ops::MKSTR, b_mkstr);
    vm.register_builtin(ops::MKARR, b_mkarr);
    vm.register_builtin(ops::MKOBJ, b_mkobj);
    vm.register_builtin(ops::CALL, b_call);
    vm.register_builtin(ops::CALL_METHOD, b_call_method);
    vm.register_builtin(ops::CALL_VALUE, b_call_value);
    vm.register_builtin(ops::NEW, b_new);
    vm.register_builtin(ops::TRUTHY, b_truthy);
    vm.register_builtin(ops::TOSTR, b_tostr);
    vm.register_builtin(ops::MKFUNC, b_mkfunc);
    vm.register_builtin(ops::GETITER, b_getiter);
    vm.register_builtin(ops::FORITER, b_foriter);
    vm.register_builtin(ops::FORIN_KEYS, b_forin_keys);
    vm.register_builtin(ops::CONTAINS, b_contains);
    vm.register_builtin(ops::SIG_RETURN, b_sig_return);
    vm.register_builtin(ops::BINOP, b_binop);
    vm.register_builtin(ops::UNARY, b_unary);
    vm.register_builtin(ops::STRICT_EQ, b_strict_eq);
    vm.register_builtin(ops::LOOSE_EQ, b_loose_eq);
    vm.register_builtin(ops::TYPEOF, b_typeof);
    vm.register_builtin(ops::LOAD_NULL, b_load_null);
    vm.register_builtin(ops::THROW, b_throw);
    vm.register_builtin(ops::TRY, b_try);
    vm.register_builtin(ops::NULLISH, b_nullish);
    vm.register_builtin(ops::UNPACK, b_unpack);
    vm.register_builtin(ops::BUILD_ARGS, b_build_args);
    vm.register_builtin(ops::THIS, b_this);
    vm.register_builtin(ops::INSTANCEOF, b_instanceof);
    vm.register_builtin(ops::DELPROP_NAME, b_delprop_name);
    vm.register_builtin(ops::APPLY, b_apply);
    vm.register_builtin(ops::APPLY_METHOD, b_apply_method);
    vm.register_builtin(ops::OBJ_REST, b_obj_rest);
    vm.register_builtin(ops::DIV, b_div);
    vm.register_builtin(ops::POW, b_pow);
    vm.register_builtin(ops::MKCLASS, b_mkclass);
    vm.register_builtin(ops::DEF_MEMBER, b_def_member);
    vm.register_builtin(ops::DEF_FIELD, b_def_field);
    vm.register_builtin(ops::SUPER_CALL, b_super_call);
    vm.register_builtin(ops::SUPER_GET, b_super_get);
    vm.register_builtin(ops::YIELD, b_yield);
    vm.register_builtin(ops::PROPKEY, b_propkey);
    vm.register_builtin(ops::NEW_TARGET, b_new_target);
    vm.register_builtin(ops::AWAIT, b_await);
    vm.register_builtin(ops::DEF_ACCESSOR, b_def_accessor);
    vm.register_builtin(ops::DBG_LINE, b_dbg_line);
    vm.register_builtin(ops::MKBIGINT, b_mkbigint);
    vm.register_builtin(ops::MKREGEX, b_mkregex);
    vm.register_builtin(ops::TAG_TMPL, b_tag_tmpl);
    vm.register_builtin(ops::GET_ASYNC_ITER, b_get_async_iter);
    vm.register_builtin(ops::ASYNC_STEP, b_async_step);
    vm.register_builtin(ops::NUM_STEP, b_num_step);
    vm.register_builtin(ops::ITER_CLOSE, b_iter_close);
    vm.register_builtin(ops::TYPEOF_NAME, b_typeof_name);
    vm.register_builtin(ops::SIG_BREAK, b_sig_break);
    vm.register_builtin(ops::SIG_CONTINUE, b_sig_continue);
    vm.register_builtin(ops::SIG_UNWIND, b_sig_unwind);
    vm.register_builtin(ops::PUSH_SCOPE, b_push_scope);
    vm.register_builtin(ops::POP_SCOPE, b_pop_scope);
    vm.register_builtin(ops::COPY_SCOPE, b_copy_scope);
    vm.register_builtin(ops::DECLARE_VAR, b_declare_var);
    vm.register_builtin(ops::NAMED_EVAL, b_named_eval);
}

/// `ITER_CLOSE`: close the iterator on the stack (a for-of `break`). A generator
/// runs its pending `finally`; a user iterator object gets its `.return()` called
/// if present; a plain materialized iterator just drops. Returns `undefined`.
/// `IteratorClose` (7.4.9): resume a generator with a forced return so its
/// pending `finally` runs, or invoke a user iterator's `.return()`. A value that
/// is neither is left alone.
pub(crate) fn close_iterator(it: &Value) -> Result<(), String> {
    if with_host(|h| h.is_generator_val(it)) {
        host::gen_return(it, Value::Undef)?;
        return Ok(());
    }
    if matches!(with_host(|h| h.get(it).cloned()), Some(JsObj::Object(_))) {
        if let Some(f) = with_host(|h| host::lookup_chain(h, it, "return")) {
            if with_host(|h| host::is_callable(h, &f)) {
                host::invoke(&f, Vec::new(), Some(it.clone()))?;
            }
        }
    }
    Ok(())
}

fn b_iter_close(vm: &mut VM, _: u8) -> Value {
    let it = vm.pop();
    // A `finally` may print or yield, but the loop is done either way; an error
    // it raises still propagates.
    match close_iterator(&it) {
        Ok(()) => Value::Undef,
        Err(e) => abort(vm, e),
    }
}

/// `NUM_STEP`: the `++`/`--` core. Pops `old` and the step `tag` (`+1`/`-1`),
/// pushes `ToNumeric(old)` (a BigInt stays a BigInt, else a Number), and returns
/// `old ± 1` in the SAME numeric type — so `x++` on a BigInt neither coerces to
/// Number nor throws the mix error.
fn b_num_step(vm: &mut VM, _: u8) -> Value {
    let old = vm.pop();
    let tag = match vm.pop() {
        Value::Int(n) => n,
        Value::Float(f) => f as i64,
        _ => 1,
    };
    if with_host(|h| h.is_bigint_val(&old)) {
        let b = with_host(|h| h.as_bigint(&old)).unwrap();
        let old_n = with_host(|h| h.new_bigint(b.clone()));
        let new = with_host(|h| h.new_bigint(b + num_bigint::BigInt::from(tag)));
        vm.push(old_n);
        new
    } else {
        let n = with_host(|h| h.to_number(&old));
        vm.push(Value::Float(n));
        Value::Float(n + tag as f64)
    }
}

/// `ASYNC_STEP`: one step of a `for await` loop — returns a Promise of the
/// `{value, done}` record (see `host::async_step`).
fn b_async_step(vm: &mut VM, _: u8) -> Value {
    let iter = vm.pop();
    let r = host::async_step(&iter);
    finish(vm, r)
}

/// `MKBIGINT`: pop the canonical decimal digit string constant, allocate the heap
/// BigInt. The lexer already validated the digits, so parsing cannot fail here.
fn b_mkbigint(vm: &mut VM, _: u8) -> Value {
    let digits = sval(&vm.pop());
    match digits.parse::<num_bigint::BigInt>() {
        Ok(b) => with_host(|h| h.new_bigint(b)),
        Err(_) => abort(vm, host::type_error("invalid BigInt literal")),
    }
}

/// `TAG_TMPL`: invoke a tagged template. The compiler emits the operands as
/// `[tag, n, m, cooked×n, raw×n, values×m]` (see `compile_tagged_template`).
/// Builds the `strings` array (carrying its `.raw` array) and calls
/// `tag(strings, ...values)`.
fn b_tag_tmpl(vm: &mut VM, argc: u8) -> Value {
    let mut all = pop_n(vm, argc as usize);
    let int_of = |v: &Value| match v {
        Value::Int(n) => *n as usize,
        Value::Float(f) => *f as usize,
        _ => 0,
    };
    let tag = all.remove(0);
    let n = int_of(&all.remove(0));
    let mcount = int_of(&all.remove(0));
    let cooked: Vec<Value> = all.drain(0..n.min(all.len())).collect();
    let raw: Vec<Value> = all.drain(0..n.min(all.len())).collect();
    let values: Vec<Value> = all.drain(0..mcount.min(all.len())).collect();
    // strings = cooked array; strings.raw = raw array (frozen in JS; nothing here
    // mutates it).
    let strings = with_host(|h| h.new_array(cooked));
    let raw_arr = with_host(|h| h.new_array(raw));
    // `GetTemplateObject` (13.2.8.4) defines `raw` as an own property that is
    // neither writable, enumerable, nor configurable, then integrity-seals the
    // template object. So `raw` stays out of `Object.keys(strings)` while
    // `getOwnPropertyNames` still reports it.
    with_host(|h| {
        h.set_fn_prop(&strings, "raw", raw_arr);
        h.set_prop_attrs(
            &strings,
            "raw",
            host::PropAttrs {
                writable: false,
                enumerable: false,
                configurable: false,
            },
        );
    });
    let mut call_args = vec![strings];
    call_args.extend(values);
    let r = host::invoke(&tag, call_args, None);
    finish(vm, r)
}

/// `GET_ASYNC_ITER`: obtain an async iterator for `for await (… of …)`. If the
/// value has a `Symbol.asyncIterator`, use it; otherwise fall back to its sync
/// iterator (each yielded value is awaited). Returns the iterator object/handle.
fn b_get_async_iter(vm: &mut VM, _: u8) -> Value {
    let src = vm.pop();
    let r = host::get_async_iterator(&src);
    finish(vm, r)
}

/// `MKREGEX`: pop `(pattern, flags)`, translate the JS pattern to a Rust `regex`,
/// and allocate a `RegExp`. A pattern using a JS feature Rust `regex` cannot
/// express (backreference/lookaround) throws a `SyntaxError` here.
fn b_mkregex(vm: &mut VM, _: u8) -> Value {
    let flags = sval(&vm.pop());
    let pattern = sval(&vm.pop());
    match crate::regexp::build_regexp(&pattern, &flags) {
        Ok(v) => v,
        Err(e) => abort(vm, e),
    }
}

/// DAP per-statement marker (`node --dap` only; the compiler emits this before
/// each statement under `debug`). Pops the source line pushed by the preceding
/// `LoadInt` and fires the debugger line hook, which pauses at breakpoints/step
/// targets. Returns `undefined` (the compiler pops it). A no-op unless a debug
/// session is active.
fn b_dbg_line(vm: &mut VM, _: u8) -> Value {
    let line = match vm.pop() {
        Value::Int(n) => n as u32,
        _ => 0,
    };
    crate::dap::on_debug_line(line);
    Value::Undef
}

/// Install an object-literal getter/setter on an object (`kind` is `member::GET`
/// or `member::SET`). Keeps the object on the stack.
fn b_def_accessor(vm: &mut VM, _: u8) -> Value {
    let func = vm.pop();
    let kind = match vm.pop() {
        Value::Int(n) => n,
        _ => 0,
    };
    let name = sval(&vm.pop());
    let obj = vm.pop();
    with_host(|h| {
        if kind == host::member::SET {
            h.set_accessor(&obj, &name, None, Some(func));
        } else {
            h.set_accessor(&obj, &name, Some(func), None);
        }
    });
    obj
}

fn b_await(vm: &mut VM, _: u8) -> Value {
    let v = vm.pop();
    match host::await_value(v) {
        Ok(r) => r,
        Err(e) => abort(vm, e),
    }
}

// ── classes / super / generators / property keys (compiler-emitted ops) ──────

fn b_mkclass(vm: &mut VM, _: u8) -> Value {
    let ctor = vm.pop();
    let parent = vm.pop();
    let name = sval(&vm.pop());
    host::build_class(&name, parent, ctor)
}

fn b_def_member(vm: &mut VM, _: u8) -> Value {
    let func = vm.pop();
    let is_static = matches!(vm.pop(), Value::Bool(true));
    let kind = match vm.pop() {
        Value::Int(n) => n,
        _ => 0,
    };
    let name = sval(&vm.pop());
    let class_val = vm.pop();
    host::define_member(&class_val, &name, kind, is_static, func);
    class_val
}

fn b_def_field(vm: &mut VM, _: u8) -> Value {
    // `name_anon`: the initializer was an anonymous function definition, so
    // 15.7.10 NamedEvaluation names its result after the field. Syntactic —
    // decided by the compiler, not re-derived from the produced value.
    let name_anon = matches!(vm.pop(), Value::Bool(true));
    let thunk = vm.pop();
    let name = sval(&vm.pop());
    let class_val = vm.pop();
    host::define_field(&class_val, &name, thunk, name_anon);
    class_val
}

/// `super(...args)` in a derived constructor: run the parent constructor on the
/// current `this`, then this class's field initializers.
fn b_super_call(vm: &mut VM, argc: u8) -> Value {
    let args = pop_n(vm, argc as usize);
    let this = with_host(|h| h.current_this());
    let this = match this {
        Some(t) => t,
        None => return abort(vm, host::type_error("'super' keyword unexpected here")),
    };
    // The class whose constructor is running = the running method's home class.
    let (parent, fields) = with_host(|h| h.super_context());
    let (parent, fields) = match parent {
        Some(p) => (p, fields),
        None => return abort(vm, host::type_error("'super' keyword unexpected here")),
    };
    let nt = with_host(|h| h.current_new_target()).unwrap_or_else(|| this.clone());
    let r = host::super_construct(&parent, args, &this, &nt);
    if let Err(e) = r {
        return abort(vm, e);
    }
    // Run this (derived) class's own instance-field initializers after super.
    for (name, thunk, name_anon) in fields {
        if let Err(e) = host::init_one_field(&this, &name, &thunk, name_anon) {
            return abort(vm, e);
        }
    }
    Value::Undef
}

/// `super.name` — a method from the parent's prototype, or a getter's result.
fn b_super_get(vm: &mut VM, _: u8) -> Value {
    let name = sval(&vm.pop());
    match with_host(|h| h.super_resolve(&name)) {
        host::SuperRef::Data(v) => v,
        host::SuperRef::Getter(getter) => {
            let this = with_host(|h| h.current_this());
            match host::invoke(&getter, Vec::new(), this) {
                Ok(v) => v,
                Err(e) => abort(vm, e),
            }
        }
    }
}

/// Close every loop iterator parked on `vm`'s stack at the op now executing,
/// innermost first. Called where a chunk is about to be halted abruptly, since
/// the code that would ordinarily close them is being jumped over.
///
/// A close runs user code (a generator's `finally`), which can itself throw; the
/// error is deliberately dropped, because it must not replace the completion
/// that caused the unwind.
fn close_parked_iters(vm: &mut VM) {
    let n = host::parked_iters(vm);
    if n == 0 {
        return;
    }
    // The completion that caused the unwind is already pending on the host.
    // Closing an iterator resumes ANOTHER generator, which settles its own
    // signal/error state, so the pending one is saved across the close and put
    // back — otherwise the outer `.return()` would be lost.
    let saved = with_host(|h| (h.signal.take(), h.error.take()));
    for _ in 0..n {
        let it = vm.pop();
        let _ = close_iterator(&it);
    }
    with_host(|h| {
        h.signal = saved.0;
        h.error = saved.1;
    });
}

fn b_yield(vm: &mut VM, _: u8) -> Value {
    let v = vm.pop();
    match host::gen_yield(v) {
        Ok(sent) => {
            // A `.return()`/`.throw()` injected on resume sets a pending Return
            // signal (or error); halt the chunk so the body unwinds through any
            // enclosing `try/finally`, exactly like a source `return`/`throw`.
            if with_host(|h| h.error.is_some() || h.signal.is_some()) {
                // Halting jumps past the loop exits, so the `for…of` / `yield*`
                // iterators parked on this chunk's stack would be abandoned
                // still-suspended. They sit directly beneath the yielded value
                // (innermost last), and the compiler recorded how many are
                // there for this exact op.
                close_parked_iters(vm);
                vm.ip = vm.chunk.ops.len();
            }
            sent
        }
        // An injected `.throw()` comes back as an error rather than a signal,
        // and abandons the parked iterators the same way. The thrown value is
        // already on the host as `exc`; `close_parked_iters` puts back whatever
        // it saves, so the close cannot swallow it.
        Err(e) => {
            close_parked_iters(vm);
            abort(vm, e)
        }
    }
}

/// `PROPKEY` — ToPropertyKey (7.1.19) for an object literal's COMPUTED key.
///
/// It called `JsHost::property_key` directly, which is the primitive-only half
/// of the conversion, so an object key never ran `ToPrimitive`:
/// `{ [{toString(){return "TS"}}]: 1 }` keyed on `"[object Object]"` while the
/// member form `a[o] = 1` — which does go through `host::to_property_key` —
/// keyed on `"TS"`. The two forms are the same abstract operation and now share
/// the same implementation.
fn b_propkey(vm: &mut VM, _: u8) -> Value {
    let v = vm.pop();
    match host::to_property_key(&v) {
        Ok(k) => with_host(|h| h.new_str(k)),
        Err(e) => abort(vm, e),
    }
}

fn b_new_target(_vm: &mut VM, _: u8) -> Value {
    with_host(|h| h.current_new_target().unwrap_or(Value::Undef))
}

/// `a / b` with JS/IEEE-754 semantics. fusevm's native `Op::Div` returns `Undef`
/// for a zero divisor (so a frontend whose `/` differs must lower to a builtin —
/// its own documented guidance), but JavaScript requires `x/0 === ±Infinity` and
/// `0/0 === NaN`, so `/` is lowered here instead.
///
/// Being a builtin rather than a native op means it does NOT reach the numeric
/// hook, so `/` was the one arithmetic operator that never ran `ToPrimitive`:
/// `({valueOf(){return 7}}) / 2` was `NaN` where every other operator gave
/// `3.5`, and `new Date(2) / 1` was `NaN` instead of `2`. It goes through the
/// hook now, so `/` coerces exactly as `*` and `-` do.
fn b_div(vm: &mut VM, _: u8) -> Value {
    let b = vm.pop();
    let a = vm.pop();
    let r = numeric_hook(NumOp::Div, &a, &b);
    finish(vm, r)
}

/// `a ** b`. Same reason `/` is a builtin: fusevm's native `Op::Pow` is IEEE-754
/// `pow`, which returns 1 for `(-1) ** Infinity` and for `1 ** NaN` where the
/// spec says NaN. Routing through the numeric hook also keeps BigInt `**` on the
/// one code path that already handles it.
fn b_pow(vm: &mut VM, _: u8) -> Value {
    let b = vm.pop();
    let a = vm.pop();
    let r = numeric_hook(NumOp::Pow, &a, &b);
    finish(vm, r)
}

/// `{ ...rest } = obj`: a new object of `obj`'s own keys minus the excluded set.
fn b_obj_rest(vm: &mut VM, _: u8) -> Value {
    let excluded = vm.pop();
    let obj = vm.pop();
    let excl: Vec<String> = with_host(|h| h.iter_vec(&excluded))
        .unwrap_or_default()
        .iter()
        .map(|v| with_host(|h| h.str_of(v)))
        .collect();
    with_host(|h| {
        let props: IndexMap<String, Value> = match h.get(&obj) {
            Some(JsObj::Object(m)) => m
                .iter()
                .filter(|(k, _)| !excl.contains(k))
                .map(|(k, v)| (k.clone(), v.clone()))
                .collect(),
            _ => IndexMap::new(),
        };
        h.new_object(props)
    })
}

// ── helpers ──────────────────────────────────────────────────────────────────

fn pop_n(vm: &mut VM, n: usize) -> Vec<Value> {
    let mut v = Vec::with_capacity(n);
    for _ in 0..n {
        v.push(vm.pop());
    }
    v.reverse();
    v
}

/// Read a compiler-internal name string (native `Value::Str` or heap `str`).
fn sval(v: &Value) -> String {
    if let Value::Str(s) = v {
        return (**s).clone();
    }
    with_host(|h| h.as_str(v)).unwrap_or_default()
}

/// The same string, without `sval`'s deep copy. Every identifier the compiler
/// emits is a `Value::Str` constant, so a variable read or write that went
/// through `sval` heap-allocated and memcpy'd the NAME once per access — on the
/// hot path of every loop. `Value::Str` is an `Arc<String>`, so cloning the
/// handle is a refcount bump instead.
fn sname(v: &Value) -> std::sync::Arc<String> {
    match v {
        Value::Str(s) => s.clone(),
        _ => std::sync::Arc::new(sval(v)),
    }
}

fn abort(vm: &mut VM, e: String) -> Value {
    with_host(|h| h.error = Some(e));
    vm.ip = vm.chunk.ops.len();
    Value::Undef
}

/// Halt the chunk if a call left an error or non-local signal pending.
fn finish(vm: &mut VM, r: Result<Value, String>) -> Value {
    match r {
        Ok(v) => {
            if with_host(|h| h.error.is_some() || h.signal.is_some()) {
                vm.ip = vm.chunk.ops.len();
            }
            v
        }
        Err(e) => abort(vm, e),
    }
}

// ── name handlers ─────────────────────────────────────────────────────────────

/// The value a bare global identifier resolves to, or `None` if unbound.
///
/// Shared by `b_getlocal` (the `x` form) and the `globalThis.x` property read,
/// which must agree: a name reachable one way and not the other is exactly the
/// discrepancy that left `globalThis.process` undefined while `process` worked.
pub(crate) fn global_binding(name: &str) -> Option<Value> {
    if let Some(v) = with_host(|h| h.read_name(name)) {
        return Some(v);
    }
    // Globals bound lazily: numeric sentinels + builtin namespaces.
    match name {
        "undefined" => return Some(Value::Undef),
        "NaN" => return Some(Value::Float(f64::NAN)),
        "Infinity" => return Some(Value::Float(f64::INFINITY)),
        // One object, not a fresh one per read: `globalThis === globalThis` is
        // `true` in JS, and `globalThis.x = 1` is readable back as
        // `globalThis.x`. Both were false while each read minted a new object.
        // `global` is Node's alias for the same object.
        "globalThis" | "global" => return Some(with_host(|h| h.global_object())),
        _ => {}
    }
    if is_namespace(name) || is_known_builtin(name) {
        return Some(with_host(|h| h.alloc(JsObj::Builtin(name.to_string()))));
    }
    None
}

fn b_getlocal(vm: &mut VM, _: u8) -> Value {
    let name = sname(&vm.pop());
    match global_binding(&name) {
        Some(v) => v,
        None => abort(vm, host::ref_error(&name)),
    }
}

/// The three global VALUE properties that are `{writable: false}` (19.1.1-19.1.3).
/// Assigning to one is a silent no-op in sloppy code and a `TypeError` in strict
/// code — and, either way, never rebinds the name.
const READONLY_GLOBALS: [&str; 3] = ["undefined", "NaN", "Infinity"];

fn readonly_global_error(name: &str) -> String {
    host::type_error(&format!(
        "Cannot assign to read only property '{name}' of object '#<Object>'"
    ))
}

fn b_setlocal(vm: &mut VM, _: u8) -> Value {
    let val = vm.pop();
    let name = sname(&vm.pop());
    // Sloppy assignment to a non-writable global is DISCARDED, not applied:
    // `undefined = 1` used to rebind the name and make every later `undefined`
    // read back as `1`.
    if READONLY_GLOBALS.contains(&name.as_str()) && !with_host(|h| h.has_name(&name)) {
        return val;
    }
    // An assignment to a `const` binding throws (8.5.2 SetMutableBinding on an
    // immutable binding). This used to succeed silently.
    if !with_host(|h| h.set_name(&name, val.clone())) {
        return abort(vm, host::type_error("Assignment to constant variable."));
    }
    val
}

/// Strict-mode `x = v` (6.2.5.6 `PutValue` with an unresolvable reference):
/// where sloppy code silently creates a global, strict code throws
/// `ReferenceError: x is not defined`.
///
/// A separate opcode rather than a runtime flag: strictness is a static property
/// of the code, so the compiler already knows which of the two an assignment is
/// and sloppy code — everything in a CommonJS module without the directive —
/// keeps the exact instruction it had.
fn b_setlocal_strict(vm: &mut VM, _: u8) -> Value {
    let val = vm.pop();
    let name = sname(&vm.pop());
    if !binding_exists(&name) {
        return abort(vm, host::ref_error(&name));
    }
    if READONLY_GLOBALS.contains(&name.as_str()) && !with_host(|h| h.has_name(&name)) {
        return abort(vm, readonly_global_error(&name));
    }
    if !with_host(|h| h.set_name(&name, val.clone())) {
        return abort(vm, host::type_error("Assignment to constant variable."));
    }
    val
}

/// Whether `name` resolves to anything — a scope binding, a global, or a lazily
/// materialised builtin namespace. `global_binding` answers the same question
/// but ALLOCATES the namespace object to do it, which an assignment then throws
/// away.
fn binding_exists(name: &str) -> bool {
    if with_host(|h| h.has_name(name)) {
        return true;
    }
    matches!(
        name,
        "undefined" | "NaN" | "Infinity" | "globalThis" | "global"
    ) || is_namespace(name)
        || is_known_builtin(name)
}

fn b_declare(vm: &mut VM, _: u8) -> Value {
    let val = vm.pop();
    let name = sname(&vm.pop());
    with_host(|h| h.declare_name(&name, val.clone()));
    val
}

/// `const x = …`: like `DECLARE`, but the binding is immutable, so a later
/// assignment to the name throws instead of overwriting it.
fn b_declare_const(vm: &mut VM, _: u8) -> Value {
    let val = vm.pop();
    let name = sname(&vm.pop());
    with_host(|h| h.declare_const_name(&name, val.clone()));
    val
}

/// `var x = …` / a hoisted `function f(){}`: bind at function scope, skipping any
/// open block scopes, so the name outlives the block it was written in.
fn b_declare_var(vm: &mut VM, _: u8) -> Value {
    let val = vm.pop();
    let name = sname(&vm.pop());
    with_host(|h| h.declare_var_name(&name, val.clone()));
    val
}

fn b_push_scope(_: &mut VM, _: u8) -> Value {
    with_host(|h| h.push_scope());
    Value::Undef
}

fn b_pop_scope(_: &mut VM, _: u8) -> Value {
    with_host(|h| h.pop_scope());
    Value::Undef
}

fn b_copy_scope(_: &mut VM, _: u8) -> Value {
    with_host(|h| h.copy_scope());
    Value::Undef
}

fn b_delname(vm: &mut VM, _: u8) -> Value {
    let name = sval(&vm.pop());
    with_host(|h| h.del_name(&name));
    Value::Bool(true)
}

fn b_this(_vm: &mut VM, _: u8) -> Value {
    with_host(|h| h.current_this().unwrap_or(Value::Undef))
}

fn b_load_null(_vm: &mut VM, _: u8) -> Value {
    with_host(|h| h.null())
}

// ── attribute / item handlers ─────────────────────────────────────────────────

fn b_getattr(vm: &mut VM, _: u8) -> Value {
    let name = sval(&vm.pop());
    let recv = vm.pop();
    match get_property(&recv, &name) {
        Ok(v) => v,
        Err(e) => abort(vm, e),
    }
}

/// Read `recv.name` (also the computed-key path for string keys). Walks own
/// properties, accessors, and the prototype chain (class methods / getters).
/// Read one small piece out of `recv`'s heap cell under a short borrow.
///
/// The closure must not call back into the host (`with_host` is a `RefCell`
/// borrow and re-entering panics) — which is exactly why it hands back only the
/// value needed: the caller re-enters freely afterwards. This replaces the old
/// `h.get(recv).cloned()` habit, which deep-copied a whole `Vec`/`IndexMap`/
/// `String` just to look at it.
fn peek<R>(recv: &Value, f: impl FnOnce(&JsObj) -> Option<R>) -> Option<R> {
    with_host(|h| h.get(recv).and_then(f))
}

/// The nearest `[[Prototype]]` link of `recv` that is a Proxy, when the chain
/// reaches it without a closer link already owning `name`.
///
/// A proxy prototype answers only from the position it occupies in the chain: a
/// nearer prototype that owns the key (as a data property or an accessor) still
/// wins, exactly as `OrdinaryGet` walks one link at a time.
pub(crate) fn proxy_proto_link(recv: &Value, name: &str) -> Option<Value> {
    with_host(|h| {
        let mut cur = h.proto_of(recv);
        for _ in 0..100 {
            let p = cur?;
            match h.get(&p) {
                Some(JsObj::Proxy { .. }) => return Some(p),
                Some(JsObj::Object(props)) if props.contains_key(name) => return None,
                _ => {}
            }
            if h.own_accessor(&p, name).is_some() {
                return None;
            }
            cur = h.proto_of(&p);
        }
        None
    })
}

pub fn get_property(recv: &Value, name: &str) -> Result<Value, String> {
    // A `#`-prefixed key is a PRIVATE name. `[[PrivateGet]]` (7.3.31) throws
    // when the receiver carries no such private element — it does NOT read back
    // as `undefined`, which is what `C.prototype.method.call({})` used to do.
    if name.starts_with('#') && !with_host(|h| h.has_private(recv, name)) {
        return Err(private_brand_message(name, false));
    }
    get_property_recv(recv, name, recv)
}

/// The `TypeError` a failed private brand check raises. Node words it two ways:
/// a private METHOD or accessor names the class the receiver should have been an
/// instance of, while a private FIELD names the member.
pub fn private_brand_message(name: &str, writing: bool) -> String {
    if with_host(|h| h.is_private_method(name)) {
        if let Some(class) = with_host(|h| h.current_home_class_name()) {
            return host::type_error(&format!("Receiver must be an instance of class {class}"));
        }
    }
    let verb = if writing { "write" } else { "read" };
    let prep = if writing { "to" } else { "from" };
    host::type_error(&format!(
        "Cannot {verb} private member {name} {prep} an object whose class did not declare it"
    ))
}

/// `[[Get]](name, receiver)` — 10.1.8. `receiver` is the object the read STARTED
/// from and is what a getter sees as `this`; it differs from `recv` only when the
/// read was forwarded down a prototype chain, which is why `Reflect.get(t, k, r)`
/// and a Proxy `get` trap's third argument both need it. Every ordinary read
/// passes `recv` itself.
pub fn get_property_recv(recv: &Value, name: &str, receiver: &Value) -> Result<Value, String> {
    // `[[Get]]` on a Proxy: the handler's `get` trap, or a forward to the
    // target. Checked before anything else so no ordinary-object shortcut can
    // read past the handler.
    if let Some(v) = crate::proxy::get(recv, name, receiver)? {
        return Ok(v);
    }
    if with_host(|h| h.is_nullish(recv)) {
        return Err(host::type_error(&format!(
            "Cannot read properties of {} (reading '{name}')",
            with_host(|h| h.str_of(recv))
        )));
    }
    // A read off `globalThis` for a name the object does not own falls back to
    // the same lazy global binding the bare identifier gets. Without it the
    // global object was an empty bag: `globalThis.process`, `.console`, `.Math`
    // and `.JSON` were all `undefined`, so `process === globalThis.process` was
    // `false` and any `globalThis.X` feature probe reported the feature missing.
    if with_host(|h| h.is_global_object(recv)) {
        let own = with_host(|h| match h.get(recv) {
            Some(JsObj::Object(p)) => p.contains_key(name),
            _ => false,
        });
        // The CommonJS wrapper's parameters are function locals in Node, not
        // global-object properties: `typeof globalThis.require` is `undefined`
        // there even though the bare `require` works.
        const CJS_WRAPPER_LOCALS: &[&str] = &[
            "require",
            "module",
            "exports",
            "__filename",
            "__dirname",
            "__cjs_require",
            "__cjs_resolve",
        ];
        if !own && !CJS_WRAPPER_LOCALS.contains(&name) {
            if let Some(v) = global_binding(name) {
                return Ok(v);
            }
        }
    }
    // Accessor (own or inherited getter) takes precedence over the chain walk.
    // The getter runs with the RECEIVER as `this`, not the object that owns it.
    if let Some((getter, _)) = with_host(|h| host::lookup_accessor(h, recv, name)) {
        return match getter {
            Some(g) => host::invoke(&g, Vec::new(), Some(receiver.clone())),
            None => Ok(Value::Undef), // set-only property reads as undefined
        };
    }
    // `Symbol.toStringTag` read as an ordinary property. The builtins that carry
    // one expose it to a plain read, not just to `Object.prototype.toString` —
    // `new Uint8Array(1)[Symbol.toStringTag]` is `'Uint8Array'`, and a `Buffer`
    // inherits `'Uint8Array'` from the typed-array prototype it now really has.
    // Anything the receiver's own chain provides wins (a class may define its
    // own getter), so this is only the fallback.
    if name == "@@toStringTag" && with_host(|h| host::lookup_chain(h, recv, name)).is_none() {
        if let Some(tag) = with_host(|h| well_known_tag(h, recv)) {
            return Ok(with_host(|h| h.new_str(tag)));
        }
    }
    // `constructor`: a user class/function sets it on the prototype chain, and
    // that wins; otherwise every builtin instance reports its native
    // constructor (so `[].constructor`, `new Map().constructor`,
    // `Promise.resolve(1).constructor`, `(5).constructor` match Node).
    if name == "constructor" {
        if let Some(v) = with_host(|h| {
            match h.get(recv) {
                Some(JsObj::Object(p)) => p.get("constructor").cloned(),
                _ => None,
            }
            .or_else(|| host::lookup_chain(h, recv, "constructor"))
        }) {
            return Ok(v);
        }
        if let Some(cn) = with_host(|h| default_ctor_name(h, recv)) {
            return Ok(with_host(|h| h.alloc(JsObj::Builtin(cn.to_string()))));
        }
    }
    // `__proto__` (Annex B B.2.2.1) is an accessor on `Object.prototype`, so it
    // answers for EVERY object that inherits from it, not only plain ones —
    // `[].__proto__` is `Array.prototype`. Only the plain-object arm handled it,
    // so an array, function or builtin instance read `undefined`. An object with
    // a null prototype inherits no such accessor and reads `undefined`, which is
    // why this is skipped there rather than answering `null`.
    if name == "__proto__"
        && !with_host(|h| h.has_null_proto(recv))
        && peek(recv, |o| match o {
            JsObj::Object(p) => Some(p.contains_key("__proto__")),
            _ => Some(false),
        }) != Some(true)
    {
        return Ok(prototype_of(recv));
    }
    let kind = with_host(|h| h.kind_of(recv));
    Ok(match kind {
        Some(ObjKind::Object) => {
            let numeric = !name.is_empty() && name.bytes().all(|b| b.is_ascii_digit());
            // Typed-array element read (`ta[i]`): elements live in a hidden
            // `@@elems`, not as own numeric props, so intercept integer keys.
            if numeric && crate::stdlib::native_tag(recv).as_deref() == Some("TypedArray") {
                if let Some(v) = crate::stdlib::typedarray::elem_get(recv, name) {
                    return Ok(v);
                }
            }
            // `buf[i]`: a Buffer's bytes live in a hidden `@@bytes` array, not as
            // own numeric props, so integer keys read through to it.
            if numeric
                && peek(recv, |o| match o {
                    JsObj::Object(p) => Some(p.contains_key("@@bytes")),
                    _ => None,
                })
                .unwrap_or(false)
            {
                return Ok(crate::stdlib::buffer::byte_get(recv, name));
            }
            if let Some(v) = peek(recv, |o| match o {
                JsObj::Object(p) => p.get(name).cloned(),
                _ => None,
            }) {
                v
            } else if let Some(link) = proxy_proto_link(recv, name) {
                // A Proxy sitting in the prototype chain. `OrdinaryGet` (10.1.8.1
                // step 4) forwards to the parent's `[[Get]]` with the ORIGINAL
                // receiver, so the trap sees the child as `receiver` and `this`
                // inside a trap-served getter resolves to the child, not the
                // proxy. `lookup_chain` cannot do this: it reads property maps,
                // and a proxy has none.
                return Ok(crate::proxy::get(&link, name, recv)?.expect("link is a proxy"));
            } else if let Some(v) = with_host(|h| host::lookup_chain(h, recv, name)) {
                // A method / data property inherited from the prototype chain.
                v
            } else if crate::stdlib::native_tag(recv)
                .map(|tag| crate::stdlib::instance_has_method(&tag, name))
                .unwrap_or(false)
            {
                // A native instance method read as a property (`server.listen`) →
                // a bound method, dispatched via `instance_call` when invoked.
                bound_method(recv, name)
            } else if is_object_method(name) && !with_host(|h| h.has_null_proto(recv)) {
                // `Object.create(null)` inherits nothing, so `toString`/`valueOf`
                // read as `undefined` there — which is also what makes
                // `Object.create(null) + 1` the spec `TypeError` instead of a
                // silent `"[object Object]1"`.
                bound_method(recv, name)
            } else {
                Value::Undef
            }
        }
        Some(ObjKind::Class) | Some(ObjKind::Func) | Some(ObjKind::BoundFunc) => {
            function_property(recv, name)
        }
        Some(ObjKind::Symbol) => match name {
            "description" => {
                match peek(recv, |o| match o {
                    JsObj::Symbol { desc, .. } => desc.clone(),
                    _ => None,
                }) {
                    Some(d) => with_host(|h| h.new_str(d)),
                    None => Value::Undef,
                }
            }
            "toString" => bound_method(recv, name),
            _ => Value::Undef,
        },
        Some(ObjKind::BigInt) => {
            if matches!(
                name,
                "toString" | "valueOf" | "toLocaleString" | "constructor"
            ) {
                bound_method(recv, name)
            } else {
                Value::Undef
            }
        }
        Some(ObjKind::RegExp) => {
            // A RegExp holds no collection, so cloning the compiled pattern here
            // does not scale with any input size; `regexp_property` re-enters the
            // host to allocate `source`/`flags`, so it cannot run under a borrow.
            let r = peek(recv, |o| match o {
                JsObj::RegExp(r) => Some(r.clone()),
                _ => None,
            });
            match r {
                Some(r) => crate::regexp::regexp_property(&r, name).unwrap_or_else(|| {
                    if crate::regexp::is_regexp_method(name) {
                        bound_method(recv, name)
                    } else {
                        Value::Undef
                    }
                }),
                None => Value::Undef,
            }
        }
        // A WeakMap/WeakSet has NO `size` (its contents are not observable), so
        // the read must be `undefined` rather than a live count.
        Some(ObjKind::Map) => {
            let (len, weak) = peek(recv, |o| match o {
                JsObj::Map { entries, weak } => Some((entries.len(), *weak)),
                _ => None,
            })
            .unwrap_or((0, false));
            match name {
                "size" if !weak => Value::Float(len as f64),
                "@@iterator" => bound_method(recv, name),
                _ if is_map_method(name) => bound_method(recv, name),
                _ => Value::Undef,
            }
        }
        Some(ObjKind::Set) => {
            let (len, weak) = peek(recv, |o| match o {
                JsObj::Set { entries, weak } => Some((entries.len(), *weak)),
                _ => None,
            })
            .unwrap_or((0, false));
            match name {
                "size" if !weak => Value::Float(len as f64),
                "@@iterator" => bound_method(recv, name),
                _ if is_set_method(name) => bound_method(recv, name),
                _ => Value::Undef,
            }
        }
        Some(ObjKind::Generator) => {
            if is_generator_method(name) {
                bound_method(recv, name)
            } else {
                Value::Undef
            }
        }
        Some(ObjKind::Promise) => {
            if matches!(name, "then" | "catch" | "finally") {
                bound_method(recv, name)
            } else {
                Value::Undef
            }
        }
        Some(ObjKind::Iter) => {
            if matches!(name, "next" | "return" | "@@iterator") {
                bound_method(recv, name)
            } else {
                Value::Undef
            }
        }
        Some(ObjKind::Array) => {
            if name == "length" {
                let n = peek(recv, |o| match o {
                    JsObj::Array(items) => Some(items.len()),
                    _ => None,
                })
                .unwrap_or(0);
                Value::Float(n as f64)
            } else if let Ok(i) = name.parse::<usize>() {
                peek(recv, |o| match o {
                    JsObj::Array(items) => items.get(i).cloned(),
                    _ => None,
                })
                .unwrap_or(Value::Undef)
            } else if name == "@@iterator" || is_array_method(name) || is_object_method(name) {
                bound_method(recv, name)
            } else if let Some(v) = with_host(|h| h.fn_prop(recv, name)) {
                // Extra own props attached to an array (e.g. `RegExp.exec` result's
                // `.index`/`.input`/`.groups`).
                v
            } else {
                Value::Undef
            }
        }
        Some(ObjKind::Str) => {
            // `.length` and `s[i]` count UTF-16 code units, not code points.
            if name == "length" {
                let n = peek(recv, |o| match o {
                    JsObj::Str(s) => Some(crate::utf16::len(s)),
                    _ => None,
                })
                .unwrap_or(0);
                Value::Float(n as f64)
            } else if let Ok(i) = name.parse::<usize>() {
                match peek(recv, |o| match o {
                    JsObj::Str(s) => crate::utf16::Units::of(s).unit_str(i),
                    _ => None,
                }) {
                    Some(c) => with_host(|h| h.new_str(c)),
                    None => Value::Undef,
                }
            } else if name == "@@iterator" || is_string_method(name) {
                bound_method(recv, name)
            } else {
                Value::Undef
            }
        }
        Some(ObjKind::Builtin) => {
            let ns = peek(recv, |o| match o {
                JsObj::Builtin(ns) => Some(ns.clone()),
                _ => None,
            })
            .unwrap_or_default();
            namespace_property(&ns, name)
        }
        _ => {
            // Primitive numbers/booleans: method access -> bound method.
            if matches!(recv, Value::Float(_) | Value::Int(_)) && is_number_method(name) {
                bound_method(recv, name)
            } else {
                Value::Undef
            }
        }
    })
}

/// The namespace name of the `require.cache` view. A `Builtin` rather than an
/// object literal because the module cache is the single source of truth: a
/// populated copy would answer reads correctly and silently ignore a `delete`,
/// which is the operation the property exists for.
pub const REQUIRE_CACHE: &str = "__cjs_cache";

/// The builtin constructor name for a value with no own/inherited `constructor`
/// property, so `x.constructor` (and thus `x.constructor.name`) matches Node for
/// arrays, plain objects, Map/Set, promises, iterators, functions, and boxed
/// primitives. `None` ⇒ leave `.constructor` as `undefined` (e.g. generators,
/// whose `.constructor.name` is `""` in Node — not worth modelling).
fn default_ctor_name(h: &host::JsHost, recv: &Value) -> Option<&'static str> {
    match h.get(recv) {
        Some(JsObj::Array(_)) => Some("Array"),
        Some(JsObj::Object(props)) => {
            // A native instance reports its own constructor, not Object — e.g.
            // `qs` does `buf.constructor.isBuffer(buf)`, so a Buffer's
            // `.constructor` must be `Buffer` (which carries `isBuffer`). Read
            // the `@@native` tag off the already-borrowed host (calling
            // `native_tag`, which re-enters `with_host`, would double-borrow).
            match props.get("@@native").map(|t| h.str_of(t)).as_deref() {
                Some("Buffer") => Some("Buffer"),
                Some("URL") => Some("URL"),
                Some("Date") => Some("Date"),
                Some("WeakRef") => Some("WeakRef"),
                Some("FinalizationRegistry") => Some("FinalizationRegistry"),
                Some("TextEncoder") => Some("TextEncoder"),
                Some("TextDecoder") => Some("TextDecoder"),
                Some("EventEmitter") => Some("EventEmitter"),
                Some("Timeout") => Some("Timeout"),
                Some("Immediate") => Some("Immediate"),
                _ => Some("Object"),
            }
        }
        Some(JsObj::Map { weak, .. }) => Some(if *weak { "WeakMap" } else { "Map" }),
        Some(JsObj::Set { weak, .. }) => Some(if *weak { "WeakSet" } else { "Set" }),
        Some(JsObj::Promise { .. }) => Some("Promise"),
        Some(JsObj::Str(_)) => Some("String"),
        Some(JsObj::Symbol { .. }) => Some("Symbol"),
        Some(JsObj::BigInt(_)) => Some("BigInt"),
        Some(JsObj::RegExp(_)) => Some("RegExp"),
        Some(JsObj::Iter { .. }) => Some("Iterator"),
        Some(JsObj::Func(_)) | Some(JsObj::Class(_)) | Some(JsObj::BoundFunc { .. }) => {
            Some("Function")
        }
        _ => match recv {
            Value::Float(_) | Value::Int(_) => Some("Number"),
            Value::Bool(_) => Some("Boolean"),
            _ => None,
        },
    }
}

/// The builtin constructor *functions*, so `Ctor.name` is the constructor name.
/// Excludes the non-callable namespaces (`Math`, `JSON`, `console`, `Reflect`,
/// `process`), whose `.name` is `undefined` in Node.
///
/// Most are also globals, but not all: `Timeout`/`Immediate` are unexposed in
/// Node (`typeof Timeout === 'undefined'`) yet still name themselves through a
/// handle's `.constructor.name`, so they belong here and not in `GLOBALS`.
fn is_builtin_ctor(name: &str) -> bool {
    matches!(
        name,
        "Array"
            | "Object"
            | "Number"
            | "String"
            | "Boolean"
            | "Symbol"
            | "Function"
            | "Map"
            | "Set"
            | "WeakMap"
            | "WeakSet"
            | "Promise"
            | "BigInt"
            | "Iterator"
            | "RegExp"
            | "Date"
            | "ArrayBuffer"
            | "Uint8Array"
            | "Int8Array"
            | "Uint8ClampedArray"
            | "Int16Array"
            | "Uint16Array"
            | "Int32Array"
            | "Uint32Array"
            | "Float32Array"
            | "Float64Array"
            | "BigInt64Array"
            | "BigUint64Array"
            | "WeakRef"
            | "FinalizationRegistry"
            | "TextEncoder"
            | "TextDecoder"
            | "IncomingMessage"
            | "ServerResponse"
            | "EventEmitter"
            | "Buffer"
            | "URL"
            | "URLSearchParams"
            | "Timeout"
            | "Immediate"
    ) || host::ERROR_NAMES.contains(&name)
}

fn bound_method(recv: &Value, name: &str) -> Value {
    with_host(|h| {
        h.alloc(JsObj::BoundMethod {
            recv: recv.clone(),
            name: name.to_string(),
        })
    })
}

/// `Object.prototype` methods reachable on any object.
fn is_object_method(name: &str) -> bool {
    matches!(
        name,
        "hasOwnProperty"
            | "isPrototypeOf"
            | "propertyIsEnumerable"
            | "toString"
            | "toLocaleString"
            | "valueOf"
            | "constructor"
    )
}

/// The `Object.prototype` methods installed as thunks on the real
/// `Object.prototype` object, so `Object.prototype.toString.call(x)` and a class
/// prototype's inherited `hasOwnProperty` both resolve through the chain.
pub const OBJECT_PROTO_METHODS: &[&str] = &[
    "hasOwnProperty",
    "isPrototypeOf",
    "propertyIsEnumerable",
    "toString",
    "toLocaleString",
    "valueOf",
];

pub fn is_object_builtin_method(name: &str) -> bool {
    matches!(
        name,
        "hasOwnProperty"
            | "isPrototypeOf"
            | "propertyIsEnumerable"
            | "toString"
            | "toLocaleString"
            | "valueOf"
    )
}

/// Dispatch an `Object.prototype` builtin method on an object/instance.
pub fn object_builtin_method(recv: &Value, name: &str, args: Vec<Value>) -> Result<Value, String> {
    match name {
        "hasOwnProperty" => {
            let k = with_host(|h| h.property_key(&arg0(&args)));
            // A builtin namespace/prototype receiver (`Map.prototype`) reports
            // ownership via `has_property` (its methods resolve as thunks).
            if with_host(|h| h.kind_of(recv)) == Some(ObjKind::Builtin) {
                return Ok(Value::Bool(has_property(recv, &k)?));
            }
            // `HasOwnProperty` (7.3.12) is `[[GetOwnProperty]]`, so on a Proxy it
            // is the `getOwnPropertyDescriptor` trap — NOT the `has` trap and not
            // the target's property map.
            if with_host(|h| h.kind_of(recv)) == Some(ObjKind::Proxy) {
                let d = crate::proxy::get_own_descriptor(recv, &k)?.unwrap_or(Value::Undef);
                return Ok(Value::Bool(!matches!(d, Value::Undef)));
            }
            // A Buffer's / typed array's own keys are its element indices: the
            // `length`/`byteLength` slots are internal bookkeeping, and V8
            // reports `hasOwnProperty('length')` as false for a typed array.
            // Shared with the `in` operator so the two cannot drift apart.
            if let Some(hit) = crate::stdlib::typedarray::has_index(recv, &k) {
                return Ok(Value::Bool(hit));
            }
            let has = with_host(|h| match h.get(recv) {
                Some(JsObj::Object(p)) => p.contains_key(&k) || h.own_accessor(recv, &k).is_some(),
                Some(JsObj::Array(items)) => {
                    k == "length"
                        || k.parse::<usize>()
                            .map(|i| i < items.len() && !h.is_hole(recv, i))
                            .unwrap_or(false)
                }
                _ => false,
            });
            Ok(Value::Bool(has))
        }
        "isPrototypeOf" => {
            let target = arg0(&args);
            // The ARGUMENT is what gets walked, so a proxy there needs its
            // `getPrototypeOf` trap for the FIRST hop: `proto_of` reads a link a
            // proxy does not hold, which reported `false` for every proxy. From
            // the second hop on the chain is ordinary objects again, walked by
            // the recorded link exactly as before.
            let mut cur = match crate::proxy::get_prototype_of(&target)? {
                Some(p) => Some(p).filter(|p| !with_host(|h| h.is_null(p))),
                None => with_host(|h| h.proto_of(&target)),
            };
            while let Some(p) = cur {
                if with_host(|h| h.strict_eq(&p, recv)) {
                    return Ok(Value::Bool(true));
                }
                cur = with_host(|h| h.proto_of(&p));
            }
            Ok(Value::Bool(false))
        }
        "propertyIsEnumerable" => {
            let k = with_host(|h| h.str_of(&arg0(&args)));
            // Own *and* enumerable — a non-enumerable own slot reads false. On a
            // Proxy that question is `[[GetOwnProperty]]`, i.e. the descriptor
            // trap, since there is no property map to enumerate.
            if with_host(|h| h.kind_of(recv)) == Some(ObjKind::Proxy) {
                let has = crate::proxy::own_enum_string_keys(recv)?.contains(&k);
                return Ok(Value::Bool(has));
            }
            let has = with_host(|h| h.own_enum_key_names(recv).contains(&k));
            Ok(Value::Bool(has))
        }
        "toString" => Ok(with_host(|h| {
            // An instance with a custom `toString` up the chain is handled by
            // call_method before reaching here; this is the default.
            let s = h.str_of(recv);
            h.new_str(s)
        })),
        // `Object.prototype.toLocaleString` (20.1.3.5) is defined as
        // `Invoke(this, "toString")` — no locale behavior of its own. It was
        // installed as a thunk on `Object.prototype` but had no dispatch arm, so
        // calling it threw `is not a function` on every plain object.
        "toLocaleString" => {
            let v = host::call_method(recv, "toString", Vec::new())?;
            Ok(v)
        }
        "valueOf" => Ok(recv.clone()),
        _ => Err(host::type_error(&format!("{name} is not a function"))),
    }
}

/// `Function.prototype` methods (`call`/`apply`/`bind`) plus `Symbol.prototype`/
/// generator handling done elsewhere. Returns `Ok(None)` if `name` is not one of
/// these (so the caller can try statics).
pub fn function_builtin_method(
    recv: &Value,
    name: &str,
    args: &[Value],
) -> Result<Option<Value>, String> {
    match name {
        "call" => {
            let this = args.first().cloned();
            let rest = args.get(1..).map(|s| s.to_vec()).unwrap_or_default();
            Ok(Some(host::invoke(recv, rest, this)?))
        }
        "apply" => {
            let this = args.first().cloned();
            let arr = args.get(1).cloned().unwrap_or(Value::Undef);
            let call_args = if matches!(arr, Value::Undef) || with_host(|h| h.is_null(&arr)) {
                Vec::new()
            } else {
                with_host(|h| h.iter_vec(&arr)).unwrap_or_default()
            };
            Ok(Some(host::invoke(recv, call_args, this)?))
        }
        "bind" => {
            let this = args.first().cloned().unwrap_or(Value::Undef);
            let pre = args.get(1..).map(|s| s.to_vec()).unwrap_or_default();
            Ok(Some(with_host(|h| {
                h.alloc(JsObj::BoundFunc {
                    target: recv.clone(),
                    this,
                    args: pre,
                })
            })))
        }
        "toString" => Ok(Some(with_host(|h| {
            let s = h.str_of(recv);
            h.new_str(s)
        }))),
        _ => Ok(None),
    }
}

fn is_function_method(name: &str) -> bool {
    matches!(name, "call" | "apply" | "bind" | "toString")
}
fn is_map_method(name: &str) -> bool {
    matches!(
        name,
        "get" | "set" | "has" | "delete" | "clear" | "forEach" | "keys" | "values" | "entries"
    )
}
fn is_set_method(name: &str) -> bool {
    matches!(
        name,
        "add" | "has" | "delete" | "clear" | "forEach" | "keys" | "values" | "entries"
    )
}
fn is_generator_method(name: &str) -> bool {
    matches!(name, "next" | "return" | "throw")
}

/// A property read on a function/class value: own fn-props (statics, name,
/// prototype, length) plus inherited statics and `call`/`apply`/`bind`.
fn function_property(recv: &Value, name: &str) -> Value {
    // A class static, inherited down the constructor chain.
    if with_host(|h| h.kind_of(recv)) == Some(ObjKind::Class) {
        if let Some(v) = with_host(|h| h.class_static(recv, name)) {
            return v;
        }
        // The chain may bottom out in a BUILTIN constructor (`class D extends
        // Array {}`), whose statics `class_static` cannot see — it only walks
        // `ClassVal.parent` links between user classes. Finish the lookup with an
        // ordinary read on that ancestor so `D.from` inherits `Array.from`.
        if let Some(anc) = with_host(|h| h.class_builtin_ancestor(recv)) {
            if let Ok(v) = get_property(&anc, name) {
                if !matches!(v, Value::Undef) {
                    return v;
                }
            }
        }
    } else if let Some(v) = with_host(|h| h.fn_prop(recv, name)) {
        return v;
    }
    // A method inherited via the function's [[Prototype]] chain (set with
    // `Object.setPrototypeOf(fn, proto)` — the `router` package makes each router
    // *function* inherit `route`/`use`/`get`/… from `Router.prototype` this way).
    if let Some(v) = with_host(|h| host::lookup_chain(h, recv, name)) {
        return v;
    }
    match name {
        "name" => with_host(|h| {
            let n = h.callable_name(recv);
            h.new_str(n)
        }),
        "length" => Value::Float(with_host(|h| h.func_arity(recv)) as f64),
        "prototype" => ensure_fn_prototype(recv),
        _ if is_function_method(name) => bound_method(recv, name),
        _ => Value::Undef,
    }
}

/// The `.prototype` of a function value, auto-created on first access (as Node
/// does for every non-arrow function) with `.constructor` linking back. Arrow
/// functions have no `prototype`.
fn ensure_fn_prototype(recv: &Value) -> Value {
    if let Some(p) = with_host(|h| h.fn_prop(recv, "prototype")) {
        return p;
    }
    // Only a constructor gets one: an arrow, a method definition and an async
    // function are not constructors, and a class sets its own (10.2.5).
    if with_host(|h| h.kind_of(recv)) != Some(ObjKind::Func) {
        return Value::Undef;
    }
    if !with_host(|h| h.owns_prototype(recv)) {
        return Value::Undef;
    }
    with_host(|h| {
        let proto = h.new_object(IndexMap::new());
        if let Some(JsObj::Object(p)) = h.get_mut(&proto) {
            p.insert("constructor".to_string(), recv.clone());
        }
        h.hide_prop(&proto, "constructor");
        h.set_fn_prop(recv, "prototype", proto.clone());
        proto
    })
}

/// A property on a builtin namespace object (`Math.PI`, `Number.MAX_SAFE_INTEGER`,
/// `console.log`).
pub fn namespace_property(ns: &str, name: &str) -> Value {
    // `require.cache[id]` — a LIVE view of the module cache, not a copy, so a
    // read sees whatever is loaded now and `delete` (see `delete_property`)
    // actually invalidates.
    if ns == REQUIRE_CACHE {
        return crate::module::cache_get(name).unwrap_or(Value::Undef);
    }
    // The ENTRY script's `require` is this builtin rather than the per-module
    // closure, so its `cache` has to be handed out here too.
    if ns == "require" && name == "cache" {
        return with_host(|h| h.alloc(JsObj::Builtin(REQUIRE_CACHE.to_string())));
    }
    // Numeric constants.
    let konst = match (ns, name) {
        ("Math", "PI") => Some(std::f64::consts::PI),
        ("Math", "E") => Some(std::f64::consts::E),
        ("Math", "LN2") => Some(std::f64::consts::LN_2),
        ("Math", "LN10") => Some(std::f64::consts::LN_10),
        ("Math", "LOG2E") => Some(std::f64::consts::LOG2_E),
        ("Math", "LOG10E") => Some(std::f64::consts::LOG10_E),
        ("Math", "SQRT2") => Some(std::f64::consts::SQRT_2),
        ("Math", "SQRT1_2") => Some(std::f64::consts::FRAC_1_SQRT_2),
        ("Number", "MAX_SAFE_INTEGER") => Some(9007199254740991.0),
        ("Number", "MIN_SAFE_INTEGER") => Some(-9007199254740991.0),
        ("Number", "MAX_VALUE") => Some(f64::MAX),
        // The smallest positive value a Number can hold, which is the smallest
        // SUBNORMAL double (`5e-324`), not Rust's `f64::MIN_POSITIVE` — that is
        // the smallest *normal* double, `2.2250738585072014e-308`, ~256 binary
        // orders of magnitude too large.
        ("Number", "MIN_VALUE") => Some(f64::from_bits(1)),
        ("Number", "EPSILON") => Some(f64::EPSILON),
        ("Number", "POSITIVE_INFINITY") => Some(f64::INFINITY),
        ("Number", "NEGATIVE_INFINITY") => Some(f64::NEG_INFINITY),
        ("Number", "NaN") => Some(f64::NAN),
        _ => None,
    };
    if let Some(k) = konst {
        return Value::Float(k);
    }
    // `Ctor.name` on a builtin constructor is the constructor name (`Array.name`
    // === "Array"); non-callable namespaces (`Math`/`JSON`) fall through to
    // `undefined`.
    if name == "name" && is_builtin_ctor(ns) {
        return with_host(|h| h.new_str(ns.to_string()));
    }
    // A well-known symbol (`Symbol.iterator`, `Symbol.toPrimitive`, …) used as a
    // computed property/method key.
    if ns == "Symbol" && host::WELL_KNOWN_SYMBOLS.contains(&name) {
        return with_host(|h| h.well_known_symbol(name));
    }
    // Non-function constants on a stdlib namespace (`path.sep`, `os.EOL`,
    // `buffer.Buffer`, `url.URL`).
    if let Some(v) = crate::stdlib::constant(ns, name) {
        return v;
    }
    // `Ctor.prototype` on a builtin constructor (`Object.prototype`,
    // `Array.prototype`, …): a prototype namespace whose methods are callable
    // thunks (`Object.prototype.toString.call(x)` is a load-time idiom in the
    // `get-intrinsic`/`function-bind` family).
    if name == "prototype" && is_builtin_ctor(ns) {
        // Same reasoning as the native prototypes below, for the error
        // hierarchy: `new Error(...)` links its `[[Prototype]]` to the REAL
        // `error_protos` object, so `Error.prototype` has to read back that same
        // object. It resolved to a fresh `Builtin("Error.prototype")` thunk
        // instead, which is a FUNCTION — so `Object.getPrototypeOf(new
        // Error("x")) === Error.prototype` was false, and `typeof
        // Error.prototype` was `"function"` where node says `"object"`.
        if host::ERROR_NAMES.contains(&ns) {
            if let Some(p) = with_host(|h| {
                h.ensure_error_protos();
                host::error_proto_of(h, ns)
            }) {
                return p;
            }
        }
        // `Buffer`/`Uint8Array` have real prototype *objects* — a Buffer's
        // `[[Prototype]]` points at one, so `Object.getPrototypeOf(buf) ===
        // Buffer.prototype` must compare equal, which a freshly-allocated
        // `Builtin` handle never can.
        if let Some(p) = with_host(|h| {
            h.ensure_native_protos();
            h.native_proto(ns)
        }) {
            return p;
        }
        let _ = ns;
        return with_host(|h| h.alloc(JsObj::Builtin(format!("{ns}.prototype"))));
    }
    // A NATIVE stdlib constructor's `.prototype` (`StringDecoder`, `Hash`,
    // `URLSearchParams`, …). These are absent from `is_builtin_ctor`, so the arm
    // above never fired and the read produced `undefined` — which broke the ES5
    // subclassing pattern libraries still ship. `iconv-lite`'s internal codec
    // reads `StringDecoder.prototype.end` at load, and threw
    // `Cannot read properties of undefined (reading 'end')`. Built from the same
    // instance-method table a method read consults, so the two cannot disagree.
    if name == "prototype" {
        if let Some(p) = with_host(|h| h.ensure_ctor_proto(ns)) {
            return p;
        }
    }
    // A method read off a builtin prototype namespace (`Array.prototype.slice`):
    // a `@proto:<Ctor>:<method>` thunk that, when invoked (typically via
    // `.call`/`.apply`), dispatches `method` against the invoke-time `this`.
    if let Some(ctor) = ns.strip_suffix(".prototype") {
        return with_host(|h| h.alloc(JsObj::Builtin(format!("@proto:{ctor}:{name}"))));
    }
    let qualified = format!("{ns}.{name}");
    if is_known_builtin(&qualified) {
        return with_host(|h| h.alloc(JsObj::Builtin(qualified)));
    }
    // A property the user stuck on this builtin namespace (`Error.prepareStackTrace`).
    if let Some(v) = with_host(|h| h.builtin_static(ns, name)) {
        return v;
    }
    Value::Undef
}

/// Dispatch a `@proto:<Ctor>:<method>` thunk (a method read off a builtin
/// prototype, e.g. `Object.prototype.toString`) against `recv` (its invoke-time
/// `this`). `Object.prototype.toString` yields the `[object Tag]` brand string
/// libraries type-check on; every other method routes through normal method
/// dispatch on `recv`.
pub fn proto_method(recv: &Value, ctor_method: &str, args: Vec<Value>) -> Result<Value, String> {
    let (ctor, method) = ctor_method.split_once(':').unwrap_or(("", ctor_method));
    // `Error.prototype.toString` (20.5.3.4): `name`, `message`, or `name:
    // message`, read off the chain so a subclass's `this.name = 'E'` is honored.
    if ctor == "Error" && method == "toString" {
        let s = with_host(|h| h.error_to_string(recv)).unwrap_or_else(|| {
            with_host(|h| {
                let name = host::lookup_chain(h, recv, "name")
                    .map(|n| h.str_of(&n))
                    .unwrap_or_else(|| "Error".into());
                let msg = host::lookup_chain(h, recv, "message")
                    .map(|m| h.str_of(&m))
                    .unwrap_or_default();
                if msg.is_empty() {
                    name
                } else {
                    format!("{name}: {msg}")
                }
            })
        });
        return Ok(with_host(|h| h.new_str(s)));
    }
    if ctor == "Object" && method == "toString" {
        // Steps 16-17 of 20.1.3.6: a `Symbol.toStringTag` STRING on the receiver
        // (own or inherited, data property or getter) replaces the builtin brand,
        // which is how a class advertises its own (`class C { get
        // [Symbol.toStringTag]() { return 'Cee' } }` → `[object Cee]`). The read
        // runs outside the host borrow so an accessor can be invoked.
        // A Proxy has no chain to probe: 20.1.3.6 step 15 is an unconditional
        // `Get(O, @@toStringTag)`, so the `get` trap decides. Probing first (as
        // the ordinary receiver does, to keep the read off objects that have no
        // tag) would always miss and brand every tagged proxy `[object Object]`.
        let tagged = with_host(|h| h.kind_of(recv)) == Some(ObjKind::Proxy)
            || with_host(|h| {
                host::lookup_chain(h, recv, "@@toStringTag").is_some()
                    || host::lookup_accessor(h, recv, "@@toStringTag").is_some()
            });
        if tagged {
            let t = get_property(recv, "@@toStringTag")?;
            if let Some(s) = with_host(|h| h.as_str(&t)) {
                return Ok(with_host(|h| h.new_str(format!("[object {s}]"))));
            }
        }
        return Ok(with_host(|h| h.new_str(object_tag(h, recv))));
    }
    // These thunks now live on the real `Object.prototype` object, i.e. on the
    // receiver's own chain — routing back through `call_method` would re-resolve
    // this very thunk and recurse.
    if ctor == "Object" && is_object_builtin_method(method) {
        return object_builtin_method(recv, method, args);
    }
    // `EventEmitter.prototype.<m>` mixed onto a receiver (express's `app`): run the
    // emitter method directly against `recv` (routing back through `call_method`
    // would re-resolve the mixed-in thunk and recurse).
    if ctor == "EventEmitter" {
        return crate::stdlib::events::instance_call(recv, method, args);
    }
    // Same recursion hazard for the exotics with a real prototype object: the
    // thunk now lives ON the receiver's prototype chain, so `call_method` would
    // re-resolve this very thunk. Dispatch straight to the native instance
    // implementation when the receiver is in fact an instance of `ctor`.
    if ctor == "Buffer" && crate::stdlib::native_tag(recv).as_deref() == Some("Buffer") {
        return crate::stdlib::buffer::instance_call(recv, method, &args);
    }
    // The shared typed-array methods now live on the `%TypedArray%.prototype`
    // intermediate, so their thunks are tagged `TypedArray`; `Uint8Array` still
    // appears for anything read directly off `Uint8Array.prototype`. Both
    // dispatch the same way, and both must bypass `call_method` or the thunk
    // would re-resolve itself off the receiver's chain and recurse.
    if ctor == "Uint8Array" || ctor == "TypedArray" {
        match crate::stdlib::native_tag(recv).as_deref() {
            Some("Buffer") => return crate::stdlib::buffer::instance_call(recv, method, &args),
            Some("TypedArray") => {
                return crate::stdlib::typedarray::instance_call(recv, method, &args)
            }
            _ => {}
        }
    }
    // `Array.prototype.<m>.call(arrayLike)` — every `Array.prototype` method is
    // GENERIC over `this` (23.1.3: each starts with `ToObject(this)` and
    // `LengthOfArrayLike`), which is what makes
    // `Array.prototype.slice.call(arguments)` the idiom it is. The receiver here
    // is not an Array, so `call_method` would report the method missing.
    if ctor == "Array" && with_host(|h| h.kind_of(recv)) != Some(ObjKind::Array) {
        return array_generic(recv, method, args);
    }
    // The general form of the two special cases above: a thunk taken off a native
    // constructor's real prototype, invoked with a receiver that IS an instance of
    // that constructor. Routing back through `call_method` would re-resolve this
    // very thunk off the receiver's own chain and recurse forever, which is why
    // each such prototype needed a hand-written bypass; now they all have one.
    if crate::stdlib::native_tag(recv).as_deref() == Some(ctor) {
        return crate::stdlib::instance_call(ctor, recv, method, args);
    }
    host::call_method(recv, method, args)
}

/// The value of `v[Symbol.toStringTag]` for a builtin that genuinely carries
/// one, or `None` when reading that symbol must yield `undefined`.
///
/// Every builtin brand is already computed in exactly one place (`object_tag`),
/// so this reuses it and subtracts the legacy builtins, which brand for
/// `Object.prototype.toString` but expose no `Symbol.toStringTag` property.
/// The subtracted list is measured against node v26.7.0, not assumed: `[]`,
/// `function(){}`, `{}`, `new Date()`, `/x/` and `new Error()` all read
/// `undefined`, while `Map`/`Set`/`Promise`/typed arrays/`ArrayBuffer`/
/// `DataView`/`WeakRef`/`FinalizationRegistry`/`BigInt`/`Symbol`/generators/
/// async+generator functions/`Math`/`JSON`/`Reflect`/`URL`/`URLSearchParams`/
/// `TextEncoder`/`TextDecoder` all read their brand.
fn well_known_tag(h: &host::JsHost, v: &Value) -> Option<String> {
    // A primitive never carries the symbol except a BigInt/Symbol wrapper, both
    // of which `object_tag` already brands.
    let tag = object_brand(h, v);
    const NO_TAG: &[&str] = &[
        "Undefined",
        "Null",
        "Boolean",
        "Number",
        "String",
        "Array",
        "Function",
        "Object",
        "Date",
        "RegExp",
        "Error",
    ];
    if NO_TAG.contains(&tag.as_str()) {
        return None;
    }
    Some(tag)
}

/// The `Object.prototype.toString` brand tag for `v` (`[object Array]` etc.).
/// Every builtin exotic object reports its own brand, which is how packages
/// type-test values they did not construct (`toString.call(x) ===
/// '[object Uint8Array]'`). A `Buffer` reports `Uint8Array` because in Node it
/// IS a `Uint8Array` subclass and inherits that `Symbol.toStringTag`.
fn object_tag(h: &host::JsHost, v: &Value) -> String {
    format!("[object {}]", object_brand(h, v))
}

/// The bare brand name behind `Object.prototype.toString` (`Array`, `Uint8Array`
/// …), without the `[object …]` wrapper. Split out so the brand and the
/// `Symbol.toStringTag` property read cannot disagree about what a value is.
fn object_brand(h: &host::JsHost, v: &Value) -> String {
    let tag: String = match v {
        Value::Undef => "Undefined".into(),
        Value::Bool(_) => "Boolean".into(),
        Value::Int(_) | Value::Float(_) => "Number".into(),
        Value::Str(_) => "String".into(),
        Value::Obj(_) => match h.get(v) {
            Some(JsObj::Null) => "Null".into(),
            Some(JsObj::Str(_)) => "String".into(),
            Some(JsObj::Array(_)) => "Array".into(),
            // 20.1.3.6 step 3 brands by `IsArray`, which follows a Proxy to its
            // `[[ProxyTarget]]` — `Object.prototype.toString.call(new Proxy([],
            // {}))` is `'[object Array]'`. Everything else about a proxy brands
            // as a plain Object (a `Symbol.toStringTag` read through the `get`
            // trap is handled by the caller, before this).
            Some(JsObj::Proxy { target, .. }) => {
                let mut cur = target;
                for _ in 0..100 {
                    match h.get(cur) {
                        Some(JsObj::Proxy { target: t, .. }) => cur = t,
                        _ => break,
                    }
                }
                match h.get(cur) {
                    Some(JsObj::Array(_)) => "Array".into(),
                    _ => "Object".into(),
                }
            }
            // `function*` / `async function` / `async function*` carry their own
            // `Symbol.toStringTag` in V8 (27.3.3.2, 27.7.3.2, 27.4.3.2).
            Some(JsObj::Func(f)) => match h.funcs.get(f.def_id) {
                Some(d) if d.is_generator && d.is_async => "AsyncGeneratorFunction".into(),
                Some(d) if d.is_generator => "GeneratorFunction".into(),
                Some(d) if d.is_async => "AsyncFunction".into(),
                _ => "Function".into(),
            },
            // `Math`/`JSON`/`Reflect` are namespace OBJECTS, not callables, and
            // brand by name (21.3.1.9, 25.5.3, 28.1.14).
            Some(JsObj::Builtin(n)) if matches!(n.as_str(), "Math" | "JSON" | "Reflect") => {
                n.clone()
            }
            Some(JsObj::Class(_))
            | Some(JsObj::Builtin(_))
            | Some(JsObj::BoundFunc { .. })
            | Some(JsObj::BoundMethod { .. }) => "Function".into(),
            // A suspended generator object is `[object Generator]`; an async one
            // `[object AsyncGenerator]`.
            Some(JsObj::Generator { .. }) if h.is_async_gen_val(v) => "AsyncGenerator".into(),
            Some(JsObj::Generator { .. }) => "Generator".into(),
            Some(JsObj::RegExp(_)) => "RegExp".into(),
            Some(JsObj::Map { weak, .. }) => if *weak { "WeakMap" } else { "Map" }.into(),
            Some(JsObj::Set { weak, .. }) => if *weak { "WeakSet" } else { "Set" }.into(),
            Some(JsObj::Promise { .. }) => "Promise".into(),
            Some(JsObj::Symbol { .. }) => "Symbol".into(),
            Some(JsObj::BigInt(_)) => "BigInt".into(),
            // Native-tagged instances brand by their tag; a typed array brands by
            // its element kind (`@@kind`), and every Error subclass is `Error`.
            Some(JsObj::Object(p)) => match p.get("@@native").map(|t| h.str_of(t)).as_deref() {
                Some("TypedArray") => p
                    .get("@@kind")
                    .map(|k| h.str_of(k))
                    .unwrap_or_else(|| "Uint8Array".into()),
                Some("Buffer") => "Uint8Array".into(),
                // Every native class that really carries a `Symbol.toStringTag`
                // in Node brands by its own name. Verified against node v26:
                // `Object.prototype.toString.call(new WeakRef({}))` is
                // `[object WeakRef]`. The rest of the `@@native` tags
                // (`EventEmitter`, `Server`, `Hash`, `Readable`, …) are plain
                // classes with NO tag, so they stay `[object Object]` — listing
                // them here would invent a brand Node does not have.
                Some(
                    t @ ("ArrayBuffer"
                    | "DataView"
                    | "Date"
                    | "WeakRef"
                    | "FinalizationRegistry"
                    | "TextEncoder"
                    | "TextDecoder"
                    | "URL"
                    | "URLSearchParams"),
                ) => t.into(),
                _ if h.error_to_string(v).is_some() => "Error".into(),
                _ => "Object".into(),
            },
            _ => "Object".into(),
        },
        // node-js only produces the Value variants above; fusevm's shell-oriented
        // variants never arise here.
        _ => "Object".into(),
    };
    tag
}

fn b_setattr(vm: &mut VM, _: u8) -> Value {
    let val = vm.pop();
    let name = sval(&vm.pop());
    let recv = vm.pop();
    if let Err(e) = set_property(&recv, &name, val.clone()) {
        return abort(vm, e);
    }
    val
}

/// `NAMED_EVAL` — SetFunctionName (10.2.9) for a function whose name is only
/// known at run time, i.e. one defined under a COMPUTED key: `{ [k]: () => {} }`,
/// `class C { static [k] = function(){} }`.
///
/// The compiler emits this ONLY where the grammar says NamedEvaluation applies
/// (`IsAnonymousFunctionDefinition` is a syntactic predicate, not a runtime one:
/// `{ m: someAlreadyAnonymousFn }` must NOT be renamed), so the name is set
/// unconditionally here.
///
/// A symbol key becomes `[description]` per step 2 of SetFunctionName; `kind`
/// contributes the accessor prefix, so `{ get [k](){} }` is `get <key>`.
fn b_named_eval(vm: &mut VM, _: u8) -> Value {
    let func = vm.pop();
    let kind = vm.pop().to_int();
    let key = vm.pop();
    let key = sval(&key);
    // `@@sym:<id>` / `@@iterator` — an internal symbol key. Step 2: an empty
    // description gives the empty name, not `[undefined]`.
    let base = match with_host(|h| h.symbol_of_key(&key)) {
        Some(sym) => match with_host(|h| h.get(&sym).cloned()) {
            Some(JsObj::Symbol {
                desc: Some(desc), ..
            }) => format!("[{desc}]"),
            _ => String::new(),
        },
        None => key,
    };
    let name = match kind {
        host::member::GET => format!("get {base}"),
        host::member::SET => format!("set {base}"),
        _ => base,
    };
    with_host(|h| {
        let s = h.new_str(name);
        h.set_fn_prop(&func, "name", s);
    });
    func
}

/// `[[Set]]` reachable from `crate::proxy`'s no-trap forward, which has to land
/// on the same path a plain `o.k = v` takes.
pub fn set_property_pub(recv: &Value, name: &str, val: Value) -> Result<(), String> {
    set_property(recv, name, val)
}

fn set_property(recv: &Value, name: &str, val: Value) -> Result<(), String> {
    // `[[PrivateSet]]` (7.3.32) refuses a receiver that carries no such private
    // element. The class's own field initializers install theirs directly
    // (`host::init_one_field`), so a declaration never reaches this check.
    if name.starts_with('#') && !with_host(|h| h.has_private(recv, name)) {
        return Err(private_brand_message(name, true));
    }
    // `[[Set]]` on a Proxy: the handler's `set` trap, or a forward to the target.
    if crate::proxy::set(recv, name, &val, recv)? {
        return Ok(());
    }
    // `globalThis.x = 1` creates a real global binding, so the bare `x` reads it
    // back. Writing only the own property left the two views disagreeing:
    // `globalThis.zz` was 7 while `zz` was still a `ReferenceError`.
    if with_host(|h| h.is_global_object(recv)) && !name.starts_with("@@") {
        with_host(|h| h.set_name(name, val.clone()));
    }
    // `obj.__proto__ = p` re-links the prototype — but only for the two values
    // the Annex B setter accepts, an Object or `null`. Everything else is a
    // silent no-op in Node (`o.__proto__ = 5` leaves `Object.getPrototypeOf(o)`
    // untouched and creates no own key), and a null-prototype object inherits
    // no such setter at all, so there the assignment is an ORDINARY own
    // property write. Re-linking unconditionally made `o.__proto__ = 5` set the
    // prototype to the number 5.
    if name == "__proto__" && with_host(|h| h.kind_of(recv)) == Some(ObjKind::Object) {
        if with_host(|h| h.has_null_proto(recv)) {
            // falls through to the ordinary own-property write below
        } else {
            let assignable =
                with_host(|h| h.is_null(&val) || matches!(h.kind_of(&val), Some(ObjKind::Object)));
            if assignable {
                with_host(|h| h.set_proto(recv, val));
            }
            return Ok(());
        }
    }
    // A non-writable own property, or a new key on a non-extensible object,
    // silently discards the write (sloppy mode — the mode every script runs in).
    if !with_host(|h| h.can_write_prop(recv, name)) {
        return Ok(());
    }
    // An inherited/own setter accessor intercepts the write.
    if let Some((_, Some(setter))) = with_host(|h| host::lookup_accessor(h, recv, name)) {
        let _ = host::invoke(&setter, vec![val], Some(recv.clone()));
        return Ok(());
    }
    // A set-only-elsewhere getter (accessor with no setter): ignore the write.
    if let Some((Some(_), None)) = with_host(|h| host::lookup_accessor(h, recv, name)) {
        return Ok(());
    }
    // Writing `name`/`prototype`/statics on a function value.
    if matches!(
        with_host(|h| h.kind_of(recv)),
        Some(ObjKind::Func) | Some(ObjKind::Class)
    ) {
        with_host(|h| h.set_fn_prop(recv, name, val));
        return Ok(());
    }
    // Writing a static onto a builtin namespace/ctor (`Error.prepareStackTrace`).
    // Each bare reference is a fresh `Builtin` handle, so route to the stable
    // per-namespace side table rather than the per-index `fn_props`.
    if let Some(ns) = peek(recv, |o| match o {
        JsObj::Builtin(ns) => Some(ns.clone()),
        _ => None,
    }) {
        // `process.exitCode` is an accessor in Node, not a data property: the
        // setter validates and stores the code the process will finally exit
        // with. Landing it in the generic static table made it a write-only
        // decoration — `process.exitCode = 3` read back as 3 and the process
        // still exited 0.
        if ns == "process" && name == "exitCode" {
            return crate::stdlib::process::set_exit_code(&val);
        }
        with_host(|h| h.set_builtin_static(&ns, name, val));
        return Ok(());
    }
    // `re.lastIndex = n` on a RegExp advances/resets its match cursor.
    if name == "lastIndex" {
        if let Some(n) = with_host(|h| match h.get(recv) {
            Some(JsObj::RegExp(_)) => Some(h.to_number(&val)),
            _ => None,
        }) {
            with_host(|h| {
                if let Some(JsObj::RegExp(r)) = h.get_mut(recv) {
                    r.last_index = if n.is_finite() && n >= 0.0 {
                        crate::utf16::U16Index::new(n as usize)
                    } else {
                        crate::utf16::U16Index::ZERO
                    };
                }
            });
            return Ok(());
        }
    }
    // Typed-array element write (`ta[i] = v`): coerce + store into `@@elems`.
    if !name.is_empty() && name.bytes().all(|b| b.is_ascii_digit()) {
        let is_ta = crate::stdlib::native_tag(recv).as_deref() == Some("TypedArray");
        if is_ta && crate::stdlib::typedarray::elem_set(recv, name, &val)? {
            return Ok(());
        }
        // `buf[i] = n` writes through to the Buffer's hidden byte array.
        if crate::stdlib::buffer::byte_set(recv, name, &val) {
            return Ok(());
        }
    }
    // An arbitrary own prop on an array (e.g. exec-result `.index`/`.input`).
    if with_host(|h| h.kind_of(recv)) == Some(ObjKind::Array)
        && name != "length"
        && name.parse::<usize>().is_err()
    {
        with_host(|h| h.set_fn_prop(recv, name, val));
        return Ok(());
    }
    // `arr.length = n` (10.4.2.4 `ArraySetLength`) validates BEFORE it resizes,
    // and does so outside the host borrow because `ToNumber` may run a user
    // `valueOf`. An invalid length throws instead of being silently coerced to 0.
    let new_len = if name == "length" && with_host(|h| h.kind_of(recv)) == Some(ObjKind::Array) {
        Some(host::to_array_length(&val)?)
    } else {
        None
    };
    with_host(|h| match h.get_mut(recv) {
        Some(JsObj::Object(props)) => {
            // Adding a *new* array-index key must re-place it into ascending
            // integer-key order (updating an existing key keeps its position).
            let is_new = !props.contains_key(name);
            props.insert(name.to_string(), val);
            if is_new && host::array_index(name).is_some() {
                host::canonicalize_own_keys(props);
            }
        }
        Some(JsObj::Array(items)) => {
            if let Some(n) = new_len {
                // Growing `length` appends HOLES (`a=[1]; a.length=3` still has
                // just the one own key); shrinking drops any hole past the end.
                let old = items.len();
                items.resize(n, Value::Undef);
                if n > old {
                    h.mark_hole_range(recv, old..n);
                } else {
                    h.truncate_holes(recv, n);
                }
            } else if let Ok(i) = name.parse::<usize>() {
                // A write PAST the end leaves the skipped positions elided.
                let old = items.len();
                if i >= old {
                    items.resize(i + 1, Value::Undef);
                }
                items[i] = val;
                if i > old {
                    h.mark_hole_range(recv, old..i);
                }
                // …and the written index itself is no longer one. This is the
                // single site that keeps a hole record from outliving the
                // elision it describes: every array element write in the
                // language reaches it.
                h.clear_hole(recv, i);
            }
        }
        _ => {}
    });
    Ok(())
}

fn b_getitem(vm: &mut VM, _: u8) -> Value {
    let idx = vm.pop();
    let recv = vm.pop();
    let key = match host::to_property_key(&idx) {
        Ok(k) => k,
        Err(e) => return abort(vm, e),
    };
    match get_property(&recv, &key) {
        Ok(v) => v,
        Err(e) => abort(vm, e),
    }
}

fn b_setitem(vm: &mut VM, _: u8) -> Value {
    let val = vm.pop();
    let idx = vm.pop();
    let recv = vm.pop();
    let key = match host::to_property_key(&idx) {
        Ok(k) => k,
        Err(e) => return abort(vm, e),
    };
    if let Err(e) = set_property(&recv, &key, val.clone()) {
        return abort(vm, e);
    }
    val
}

/// `[[Delete]]` (10.1.10) for an already-resolved property key: the one place
/// `delete o[k]`, `delete o.k` and `Reflect.deleteProperty` all go through, so
/// the three cannot drift. Reports `false` for a non-configurable property
/// (sloppy mode ignores the failure rather than throwing) and `true` otherwise,
/// which is also what deleting an absent key reports.
pub fn delete_property(recv: &Value, key: &str) -> Result<bool, String> {
    // `[[Delete]]` on a Proxy runs the handler's `deleteProperty` trap, which may
    // throw — the reason this reports a `Result` rather than a bare `bool`.
    if let Some(b) = crate::proxy::delete(recv, key)? {
        return Ok(b);
    }
    // `delete require.cache[id]` drops the module so the next `require` of that
    // file runs it again — the whole point of exposing the cache.
    if peek(recv, |o| match o {
        JsObj::Builtin(ns) => Some(ns == REQUIRE_CACHE),
        _ => None,
    }) == Some(true)
    {
        return Ok(crate::module::cache_delete(key));
    }
    if !with_host(|h| h.prop_attrs(recv, key).configurable) {
        return Ok(false);
    }
    with_host(|h| {
        let index = key.parse::<usize>();
        match h.get_mut(recv) {
            Some(JsObj::Object(props)) => {
                props.shift_remove(key);
                return;
            }
            Some(JsObj::Array(items)) => {
                if let Ok(i) = index {
                    if i < items.len() {
                        // `delete a[i]` punches a HOLE: the length is unchanged
                        // but the index stops being an own property.
                        items[i] = Value::Undef;
                        h.mark_hole(recv, i);
                    }
                    return;
                }
            }
            _ => {}
        }
        // A non-index key on an array (`arr.foo`, `arr[sym]`), or any own key on
        // a function/class, is an ordinary own property kept in the side table.
        h.remove_fn_prop(recv, key);
    });
    Ok(true)
}

fn b_delitem(vm: &mut VM, _: u8) -> Value {
    let idx = vm.pop();
    let recv = vm.pop();
    // `delete o[k]` keys through ToPropertyKey (7.1.19), exactly as the read and
    // the write do: `String(k)` would turn a Symbol into its `Symbol(desc)`
    // description and delete a key nothing ever wrote.
    let key = match host::to_property_key(&idx) {
        Ok(k) => k,
        Err(e) => return abort(vm, e),
    };
    match delete_property(&recv, &key) {
        Ok(b) => Value::Bool(b),
        Err(e) => abort(vm, e),
    }
}

fn b_delprop_name(vm: &mut VM, _: u8) -> Value {
    let name = sval(&vm.pop());
    let recv = vm.pop();
    match delete_property(&recv, &name) {
        Ok(b) => Value::Bool(b),
        Err(e) => abort(vm, e),
    }
}

// ── constructors ──────────────────────────────────────────────────────────────

fn b_mkstr(vm: &mut VM, argc: u8) -> Value {
    let parts = pop_n(vm, argc as usize);
    let s: String = with_host(|h| parts.iter().map(|p| h.str_of(p)).collect());
    with_host(|h| h.new_str(s))
}

fn b_mkarr(vm: &mut VM, argc: u8) -> Value {
    let items = pop_n(vm, argc as usize);
    with_host(|h| h.new_array(items))
}

/// `MARK_HOLE [arr, index]`: record `arr[index]` as an ELIDED element. Emitted
/// only for an array literal that actually contains an elision, so a dense
/// literal costs nothing. Returns `undefined`; the array stays on the stack
/// underneath (the compiler `Dup`s it).
fn b_mark_hole(vm: &mut VM, _: u8) -> Value {
    let idx = vm.pop();
    let arr = vm.pop();
    let i = match idx {
        Value::Int(i) if i >= 0 => i as usize,
        _ => return Value::Undef,
    };
    with_host(|h| h.mark_hole(&arr, i));
    Value::Undef
}

fn b_mkobj(vm: &mut VM, argc: u8) -> Value {
    let flat = pop_n(vm, argc as usize);
    let mut props: IndexMap<String, Value> = IndexMap::new();
    // A literal `__proto__: x` key sets the object's prototype (not an own prop).
    let mut proto_override: Option<Value> = None;
    let mut i = 0;
    while i + 2 < flat.len() || (i + 2 == flat.len() && flat.len() % 3 == 0 && i < flat.len()) {
        if i + 2 >= flat.len() {
            break;
        }
        // Tag 2: an ACCESSOR's position. An accessor lives in its own table, so
        // the literal reserves its slot here with the `@@ord:` marker key that
        // `own_enum_data_keys` resolves back — otherwise `{ get g(){}, d: 2 }`
        // enumerated `d, g`, because `DEF_ACCESSOR` runs after `MKOBJ` and its
        // marker landed at the end.
        if matches!(flat[i], Value::Int(2)) {
            let key = with_host(|h| h.str_of(&flat[i + 1]));
            props
                .entry(format!("{}{key}", host::ORD_MARKER))
                .or_insert(Value::Undef);
            i += 3;
            continue;
        }
        let spread = matches!(flat[i], Value::Int(1));
        if spread {
            let src = flat[i + 1].clone();
            // A STRING source spreads its index properties (`{..."ab"}` is
            // `{0:'a',1:'b'}`): CopyDataProperties (7.3.25) calls ToObject, and a
            // String exotic object owns one enumerable property per UTF-16 code
            // UNIT (10.4.3). `own_enum_entries_deep` only walks heap objects, so
            // a string source contributed nothing and `{..."ab"}` was `{}`.
            // Every other primitive (number/boolean/symbol) boxes to an object
            // with no own enumerable properties, and null/undefined are ignored,
            // so those correctly stay no-ops on the path below.
            if let Some(s) = with_host(|h| h.as_str(&src)) {
                for idx in 0..crate::utf16::len(&s) {
                    if let Ok(ch) = get_property(&src, &idx.to_string()) {
                        props.insert(idx.to_string(), ch);
                    }
                }
                i += 3;
                continue;
            }
            // Object spread copies own *enumerable* properties only — never the
            // hidden `@@…` slots (copying `@@native` used to turn `{...buf}`
            // into something that still claimed to be a Buffer) and never a
            // property a descriptor marked non-enumerable.
            let entries = host::own_enum_entries_deep(&src);
            for (k, v) in entries {
                props.insert(k, v);
            }
            // `CopyDataProperties` (7.3.25) copies own enumerable SYMBOL keys
            // too — only `Object.keys`/`for-in`/`JSON.stringify` skip them.
            for (k, v) in with_host(|h| h.own_symbol_entries(&src)) {
                props.insert(k, v);
            }
        } else {
            let key = with_host(|h| h.str_of(&flat[i + 1]));
            if key == "__proto__" {
                proto_override = Some(flat[i + 2].clone());
            } else {
                props.insert(key, flat[i + 2].clone());
            }
        }
        i += 3;
    }
    with_host(|h| {
        let o = h.new_object(props);
        if let Some(p) = proto_override {
            if matches!(p, Value::Obj(_)) {
                h.set_proto(&o, p);
            }
        }
        o
    })
}

fn b_mkfunc(vm: &mut VM, _: u8) -> Value {
    let def_id = match vm.pop() {
        Value::Int(n) => n as usize,
        Value::Float(f) => f as usize,
        _ => return abort(vm, "internal: MKFUNC id".into()),
    };
    let (is_arrow, self_name) = with_host(|h| match h.funcs.get(def_id) {
        Some(d) => (
            d.is_arrow,
            (d.self_name && !d.name.is_empty()).then(|| d.name.clone()),
        ),
        None => (false, None),
    });
    with_host(|h| {
        let mut env = h.current_env_capture();
        let this = h.current_this();
        // A named function expression closes over an extra scope holding its own
        // name, so the body can recurse through it (`function f(){ … f() … }`)
        // independently of whatever the outer binding is later set to.
        if self_name.is_some() {
            env = host::child_env(env);
        }
        let f = h.alloc(JsObj::Func(FuncVal {
            def_id,
            env: Some(env.clone()),
            this,
            is_arrow,
            home_class: None,
        }));
        if let Some(n) = self_name {
            env.borrow_mut().vars.insert(n, f.clone());
        }
        f
    })
}

// ── truthiness / coercion / equality ──────────────────────────────────────────

fn b_truthy(vm: &mut VM, _: u8) -> Value {
    let v = vm.pop();
    Value::Bool(with_host(|h| h.truthy(&v)))
}

fn b_nullish(vm: &mut VM, _: u8) -> Value {
    let v = vm.pop();
    Value::Bool(with_host(|h| h.is_nullish(&v)))
}

fn b_tostr(vm: &mut VM, _: u8) -> Value {
    let v = vm.pop();
    // ToString with user-`toString`/`valueOf` dispatch (template interpolation,
    // `String(x)`, object keys).
    match host::to_string_value(&v) {
        Ok(s) => s,
        Err(e) => abort(vm, e),
    }
}

fn b_typeof(vm: &mut VM, _: u8) -> Value {
    let v = vm.pop();
    with_host(|h| {
        let t = h.type_of(&v);
        h.new_str(t)
    })
}

/// `typeof <bare ident>`: read the name like `b_getlocal` but return "undefined"
/// (never a ReferenceError) when the name is unbound — JS `typeof` semantics.
fn b_typeof_name(vm: &mut VM, _: u8) -> Value {
    let name = sval(&vm.pop());
    // Bound name (user variable) → typeof its value.
    if let Some(v) = with_host(|h| h.read_name(&name)) {
        return with_host(|h| {
            let t = h.type_of(&v);
            h.new_str(t)
        });
    }
    // Lazily-bound globals mirror `b_getlocal`: resolve to the same value it
    // would produce, then take its type (so object-namespaces like `console`/
    // `Math`/`JSON`/`process` report "object", constructors report "function").
    let t = match name.as_str() {
        "undefined" => "undefined".to_string(),
        "NaN" | "Infinity" => "number".to_string(),
        "globalThis" | "global" => "object".to_string(),
        n if is_namespace(n) || is_known_builtin(n) => {
            let v = with_host(|h| h.alloc(JsObj::Builtin(name.clone())));
            with_host(|h| h.type_of(&v)).to_string()
        }
        _ => "undefined".to_string(), // genuinely unbound → JS returns "undefined"
    };
    with_host(|h| h.new_str(t))
}

fn b_strict_eq(vm: &mut VM, _: u8) -> Value {
    let b = vm.pop();
    let a = vm.pop();
    Value::Bool(with_host(|h| h.strict_eq(&a, &b)))
}

fn b_loose_eq(vm: &mut VM, _: u8) -> Value {
    let b = vm.pop();
    let a = vm.pop();
    // Abstract Equality steps 10-11 (7.2.15): object ⇄ primitive converts the
    // object with `ToPrimitive` — a JS `valueOf`/`Symbol.toPrimitive` call, so it
    // runs before the host borrow. Object ⇄ object stays a reference check.
    let (a, b) = match with_host(|h| (host::is_primitive(h, &a), host::is_primitive(h, &b))) {
        (false, true) if coerces_against_object(&b) => match host::to_primitive(&a, "default") {
            Ok(p) => (p, b),
            Err(e) => return abort(vm, e),
        },
        (true, false) if coerces_against_object(&a) => match host::to_primitive(&b, "default") {
            Ok(p) => (a, p),
            Err(e) => return abort(vm, e),
        },
        _ => (a, b),
    };
    Value::Bool(with_host(|h| h.loose_eq(&a, &b)))
}

fn b_instanceof(vm: &mut VM, _: u8) -> Value {
    let ctor = vm.pop();
    let obj = vm.pop();
    match host::instance_of(&obj, &ctor) {
        Ok(b) => Value::Bool(b),
        Err(e) => abort(vm, e),
    }
}

// ── bitwise / unary ───────────────────────────────────────────────────────────

fn b_binop(vm: &mut VM, _: u8) -> Value {
    let b = vm.pop();
    let a = vm.pop();
    let tag = match vm.pop() {
        Value::Int(n) => n,
        _ => 0,
    };
    // Both operands are ToPrimitive-d with the number hint before ToInt32
    // (ECMA-262 13.12.1), which has to happen outside the host borrow.
    let r = host::to_primitive(&a, "number")
        .and_then(|a| host::to_primitive(&b, "number").map(|b| (a, b)))
        .and_then(|(a, b)| with_host(|h| h.bitwise(tag, &a, &b)));
    finish(vm, r)
}

fn b_unary(vm: &mut VM, _: u8) -> Value {
    let v = vm.pop();
    let tag = match vm.pop() {
        Value::Int(n) => n,
        _ => 0,
    };
    // Unary `+`/`~` on a BigInt: `+` is a hard TypeError in JS; `~x` is `-x - 1`
    // computed in arbitrary precision.
    if with_host(|h| h.is_bigint_val(&v)) {
        return match tag {
            host::unop::POS => abort(
                vm,
                host::type_error("Cannot convert a BigInt value to a number"),
            ),
            host::unop::BITNOT => {
                let b = with_host(|h| h.as_bigint(&v)).unwrap();
                let r = -(b + num_bigint::BigInt::from(1));
                with_host(|h| h.new_bigint(r))
            }
            _ => Value::Undef,
        };
    }
    // `ToNumber` outside the host borrow: an object operand's `valueOf` /
    // `Symbol.toPrimitive` is a JS call, so it cannot run under `with_host`.
    let n = match host::to_number_value(&v) {
        Ok(n) => n,
        Err(e) => return abort(vm, e),
    };
    match tag {
        host::unop::POS => Value::Float(n),
        host::unop::BITNOT => {
            let i = if n.is_finite() {
                n.trunc() as i64 as i32
            } else {
                0
            };
            Value::Float(!i as f64)
        }
        _ => Value::Undef,
    }
}

// ── membership ────────────────────────────────────────────────────────────────

fn b_contains(vm: &mut VM, _: u8) -> Value {
    let container = vm.pop();
    let key = vm.pop();
    // `x in y` requires y to be an object. V8 names both operands:
    // `Cannot use 'in' operator to search for 'a' in 5`.
    if !matches!(container, Value::Obj(_)) {
        let (k, c) = with_host(|h| (h.property_key(&key), h.str_of(&container)));
        return abort(
            vm,
            host::type_error(&format!(
                "Cannot use 'in' operator to search for '{k}' in {c}"
            )),
        );
    }
    let k = with_host(|h| h.property_key(&key));
    match has_property(&container, &k) {
        Ok(b) => Value::Bool(b),
        Err(e) => abort(vm, e),
    }
}

// ── control ───────────────────────────────────────────────────────────────────

fn b_sig_return(vm: &mut VM, _: u8) -> Value {
    let v = vm.pop();
    with_host(|h| h.signal = Some(host::Signal::Return(v.clone())));
    vm.ip = vm.chunk.ops.len();
    v
}

/// `break [label]` whose target loop lives in an enclosing chunk (the statement is
/// inside a `try` block, which the host runs as its own chunk). Raise the signal
/// and halt this chunk; `SIG_UNWIND` after the `TRY` op re-dispatches it.
fn b_sig_break(vm: &mut VM, _: u8) -> Value {
    let label = sval(&vm.pop());
    let label = (!label.is_empty()).then_some(label);
    with_host(|h| h.signal = Some(host::Signal::Break(label)));
    vm.ip = vm.chunk.ops.len();
    Value::Undef
}

/// `continue [label]` out of a `try` block — see [`b_sig_break`].
fn b_sig_continue(vm: &mut VM, _: u8) -> Value {
    let label = sval(&vm.pop());
    let label = (!label.is_empty()).then_some(label);
    with_host(|h| h.signal = Some(host::Signal::Continue(label)));
    vm.ip = vm.chunk.ops.len();
    Value::Undef
}

/// Dispatch a pending control signal at the instruction after a `TRY`. `tag`
/// describes what the `try` is nested in (see [`host::unwind`]):
///
/// * no signal → `NONE`, execution continues normally;
/// * `Return`, or no enclosing loop in this chunk → halt the chunk so the signal
///   keeps travelling outward;
/// * `break`/`continue` targeting the enclosing loop → consume it and report
///   `BREAK`/`CONTINUE` so the compiler-emitted jump lands on the loop's exit /
///   continue target;
/// * a LABELED `break`/`continue` for some outer loop → report `BREAK` but leave
///   the signal pending, so leaving this loop re-dispatches it one level out.
fn b_sig_unwind(vm: &mut VM, _: u8) -> Value {
    let cont_tag = sval(&vm.pop());
    let brk_tag = sval(&vm.pop());
    let sig = match with_host(|h| h.signal.clone()) {
        Some(s) => s,
        None => return Value::Int(host::unwind::NONE),
    };
    // Nothing in this chunk can catch a `break`: halt so the signal keeps going.
    let propagate = |vm: &mut VM| {
        vm.ip = vm.chunk.ops.len();
        Value::Int(host::unwind::NONE)
    };
    match &sig {
        host::Signal::Return(_) => propagate(vm),
        host::Signal::Break(label) => {
            if brk_tag == host::unwind::NO_LOOP {
                return propagate(vm);
            }
            let mine = match label {
                None => true, // unlabeled: always the innermost enclosing context
                Some(l) => brk_tag == *l,
            };
            if mine {
                with_host(|h| h.signal = None);
            }
            // Not ours: still leave this context by its break exit, keeping the
            // signal pending for the next dispatch point one level out.
            Value::Int(host::unwind::BREAK)
        }
        host::Signal::Continue(label) => {
            let mine = match label {
                // Unlabeled `continue` binds to the innermost continue-catching
                // loop — which a `switch` between here and it is NOT.
                None => cont_tag != host::unwind::NO_LOOP,
                Some(l) => cont_tag == *l,
            };
            if mine {
                with_host(|h| h.signal = None);
                return Value::Int(host::unwind::CONTINUE);
            }
            if brk_tag == host::unwind::NO_LOOP {
                return propagate(vm);
            }
            // The target loop is further out: exit the innermost context here and
            // re-dispatch there.
            Value::Int(host::unwind::BREAK)
        }
    }
}

fn b_throw(vm: &mut VM, _: u8) -> Value {
    let v = vm.pop();
    let msg = with_host(|h| {
        h.exc = Some(v.clone());
        // Prefer an error object's message for the top-level report.
        error_display(h, &v)
    });
    abort(vm, msg)
}

fn error_display(h: &host::JsHost, v: &Value) -> String {
    if let Some(JsObj::Object(props)) = h.get(v) {
        let name = props
            .get("name")
            .map(|x| h.str_of(x))
            .unwrap_or_else(|| "Error".into());
        if let Some(m) = props.get("message") {
            return format!("Uncaught {name}: {}", h.str_of(m));
        }
    }
    format!("Uncaught {}", h.str_of(v))
}

fn b_try(vm: &mut VM, _: u8) -> Value {
    let id = match vm.pop() {
        Value::Int(n) => n as usize,
        _ => return abort(vm, "internal: TRY id".into()),
    };
    // Shape only. Running a `try` used to clone the whole `TryDef` — its block,
    // its handler and its finalizer bytecode — every time control entered it,
    // which for a `try` inside a loop is once per iteration.
    let (has_handler, catch_bind, has_finalizer) = match with_host(|h| h.try_shape(id)) {
        Some(t) => t,
        None => return abort(vm, "internal: unknown try id".into()),
    };
    let mut pending: Option<String> = None;
    // Each sub-block runs as its own chunk on THIS frame, so a throw part-way
    // through can leave block scopes open. Snapshot the scope and restore it
    // before the handler and after the whole statement.
    let scope = with_host(|h| h.scope_snapshot());

    with_host(|h| h.push_scope()); // the try block is its own block scope
    let body_res = host::run_chunk_keyed(host::try_key(id, 0), || {
        with_host(|h| h.try_chunk(id, 0)).expect("try block exists")
    });
    with_host(|h| h.restore_scope(scope.clone()));
    let signal_after = with_host(|h| h.signal.is_some());
    if let Err(e) = body_res {
        if signal_after {
            pending = Some(e);
        } else if has_handler {
            // Bind the thrown value (or a synthesized error) to the catch param.
            let thrown =
                with_host(|h| h.exc.clone()).unwrap_or_else(|| with_host(|h| synth_error(h, &e)));
            with_host(|h| {
                h.error = None;
                h.exc = None;
            });
            // The catch parameter is block-scoped to the handler.
            with_host(|h| h.push_scope());
            if let Some(name) = &catch_bind {
                with_host(|h| h.declare_name(name, thrown));
            }
            let hres = host::run_chunk_keyed(host::try_key(id, 1), || {
                with_host(|h| h.try_chunk(id, 1)).expect("handler exists")
            });
            with_host(|h| h.restore_scope(scope.clone()));
            if let Err(e2) = hres {
                pending = Some(e2);
            }
        } else {
            pending = Some(e);
        }
    }

    // finally always runs; a finally error/signal supersedes.
    if has_finalizer {
        let sig_before = with_host(|h| h.signal.take());
        with_host(|h| h.push_scope()); // ditto for `finally`
        let fres = host::run_chunk_keyed(host::try_key(id, 2), || {
            with_host(|h| h.try_chunk(id, 2)).expect("finalizer exists")
        });
        with_host(|h| h.restore_scope(scope.clone()));
        match fres {
            Ok(_) => {
                if with_host(|h| h.signal.is_none()) {
                    // The finalizer completed normally: the try/catch block's own
                    // abrupt completion resumes.
                    with_host(|h| h.signal = sig_before);
                } else {
                    // ECMA-262 14.15.3 TryStatement evaluation: when the finalizer's
                    // completion is abrupt (`return`/`break`/`continue` inside
                    // `finally`), that completion REPLACES the try/catch block's —
                    // including a pending throw, which is discarded, not rethrown.
                    pending = None;
                    with_host(|h| {
                        h.error = None;
                        h.exc = None;
                    });
                }
            }
            Err(e) => pending = Some(e),
        }
    }

    if let Some(e) = pending {
        return abort(vm, e);
    }
    Value::Undef
}

/// Synthesize an `Error`-shaped object from an internal error string, linked to
/// the matching builtin error prototype so `instanceof`/`.constructor` work.
pub(crate) fn synth_error(h: &mut host::JsHost, e: &str) -> Value {
    h.ensure_error_protos();
    // A `Name [ERR_CODE]: message` head carries a Node error `code` next to the
    // error class, exactly as Node's internal errors render it in `.stack`.
    let (head, rest) = match e.split_once(": ") {
        Some((n, m)) => (n, m.to_string()),
        None => ("", e.to_string()),
    };
    let (base, code) = match head.split_once(" [") {
        Some((n, c)) if c.ends_with(']') => (n, Some(c[..c.len() - 1].to_string())),
        _ => (head, None),
    };
    let (name, mut message) = if host::ERROR_NAMES.contains(&base) {
        (base.to_string(), rest)
    } else {
        ("Error".to_string(), e.to_string())
    };
    // A `host::plain_coded_error` marker: the code rides at the head of the
    // MESSAGE rather than in the class, because Node's native-layer errors set
    // `.code` while leaving `String(err)` unbracketed (`TypeError: Invalid URL`
    // with `code === 'ERR_INVALID_URL'`). Strip it back off here — the marker is
    // internal and must never reach a user-visible `.message`.
    let mut code = code;
    // Whether `String(err)`/`err.stack` show `Name [CODE]:` — true for the
    // bracketed head, false for the marker form.
    let mut bracketed = code.is_some();
    if let Some(rest) = message.strip_prefix(host::CODE_MARK) {
        if let Some((c, m)) = rest.split_once('\u{1}') {
            code = Some(c.to_string());
            bracketed = false;
            message = m.to_string();
        }
    }
    let mut props: IndexMap<String, Value> = IndexMap::new();
    let mv = h.new_str(message.clone());
    props.insert("message".into(), mv);
    if let Some(c) = &code {
        let cv = h.new_str(c.clone());
        props.insert("code".into(), cv);
        if bracketed {
            // Marks this as a Node JS-layer error, whose `toString` brackets the
            // code. A native-layer error has the same `.code` and does not.
            props.insert("@@nodeError".into(), Value::Bool(true));
        }
    }
    let label = match (&code, bracketed) {
        (Some(c), true) => format!("{name} [{c}]"),
        _ => name.clone(),
    };
    let frames = h.stack_frames();
    let stack = if message.is_empty() {
        format!("{label}{frames}")
    } else {
        format!("{label}: {message}{frames}")
    };
    let sv = h.new_str(stack);
    props.insert("stack".into(), sv);
    // A libuv system-error message is itself the canonical encoding of the
    // error's metadata — `ENOENT: no such file or directory, open '/x'` — so a
    // filesystem/network failure recovers the enumerable `code`/`errno`/
    // `syscall`/`path` own properties that `err.code === 'ENOENT'` checks (the
    // single most common error-handling idiom in Node packages) depend on.
    for (k, v) in syscall_error_fields(&message) {
        let sv = match v {
            SysField::Str(s) => h.new_str(s),
            SysField::Num(n) => Value::Float(n),
        };
        props.insert(k.into(), sv);
    }
    let obj = h.new_object(props);
    if let Some(p) = host::error_proto_of(h, &name) {
        h.set_proto(&obj, p);
    }
    // `message`/`stack` are non-enumerable; a Node `ERR_*` error's `code` is not
    // (`Object.keys(e)` on an `ERR_INVALID_ARG_TYPE` reads `["code"]`).
    h.hide_prop(&obj, "message");
    h.hide_prop(&obj, "stack");
    obj
}

enum SysField {
    Str(String),
    Num(f64),
}

/// Decompose a libuv-shaped message (`ECODE: reason, syscall 'path'`) into the
/// own properties Node hangs off a system error. Returns empty for any message
/// that is not in that shape.
fn syscall_error_fields(message: &str) -> Vec<(&'static str, SysField)> {
    let (code, rest) = match message.split_once(": ") {
        Some((c, r))
            if c.len() >= 2
                && c.starts_with('E')
                && c.bytes()
                    .all(|b| b.is_ascii_uppercase() || b.is_ascii_digit()) =>
        {
            (c, r)
        }
        _ => return Vec::new(),
    };
    let mut out: Vec<(&'static str, SysField)> = vec![
        ("errno", SysField::Num(errno_for(code))),
        ("code", SysField::Str(code.to_string())),
    ];
    // `reason, syscall 'path'` — the path is optional (`EPIPE: …, write`).
    if let Some((_, tail)) = rest.split_once(", ") {
        let (syscall, path) = match tail.split_once(" '") {
            Some((s, p)) => (s, p.strip_suffix('\'')),
            None => (tail, None),
        };
        out.push(("syscall", SysField::Str(syscall.to_string())));
        if let Some(p) = path {
            out.push(("path", SysField::Str(p.to_string())));
        }
    }
    out
}

/// The negative `errno` Node reports for a libuv error code on this platform.
/// Only the codes `err_str` can produce are mapped; anything else reports the
/// generic `EIO` number rather than inventing a value.
fn errno_for(code: &str) -> f64 {
    let n: i32 = match code {
        "ENOENT" => 2,
        "EACCES" => 13,
        "EEXIST" => 17,
        "ENOTDIR" => 20,
        "EISDIR" => 21,
        "EINVAL" => 22,
        "EPIPE" => 32,
        "ENOTEMPTY" => 66,
        _ => 5, // EIO
    };
    -f64::from(n)
}

// ── iteration ─────────────────────────────────────────────────────────────────

fn b_getiter(vm: &mut VM, _: u8) -> Value {
    let v = vm.pop();
    // A generator is its own iterator (resumed lazily by FORITER).
    if with_host(|h| h.is_generator_val(&v)) {
        return v;
    }
    // A Proxy's iterator comes from its traps, materialized eagerly: the
    // `lookup_chain` probe below reads the property map a proxy does not have.
    if with_host(|h| h.kind_of(&v)) == Some(ObjKind::Proxy) {
        return match crate::proxy::iterate(&v) {
            Ok(Some(items)) => with_host(|h| h.alloc(JsObj::Iter { items, idx: 0 })),
            Ok(None) => abort(vm, "internal: kind_of said Proxy".into()),
            Err(e) => abort(vm, e),
        };
    }
    // An object with a user `Symbol.iterator`: call it to get the iterator object.
    if let Some(iter_fn) = with_host(|h| host::lookup_chain(h, &v, "@@iterator")) {
        if with_host(|h| host::is_callable(h, &iter_fn)) {
            return match host::invoke(&iter_fn, Vec::new(), Some(v.clone())) {
                Ok(it) => it,
                Err(e) => abort(vm, e),
            };
        }
    }
    match with_host(|h| h.iter_vec(&v)) {
        Ok(items) => with_host(|h| h.alloc(JsObj::Iter { items, idx: 0 })),
        Err(e) => abort(vm, e),
    }
}

fn b_forin_keys(vm: &mut VM, _: u8) -> Value {
    let v = vm.pop();
    // `for-in` over a Proxy is 14.7.5.9 `EnumerateObjectProperties`: the
    // `ownKeys` trap filtered by `[[GetOwnProperty]]`'s `enumerable`. Both traps
    // are user code, so this cannot run inside `enum_keys`'s `&mut` host borrow.
    if with_host(|h| h.kind_of(&v)) == Some(ObjKind::Proxy) {
        return match crate::proxy::own_enum_string_keys(&v) {
            Ok(keys) => with_host(|h| {
                let out: Vec<Value> = keys.into_iter().map(|k| h.new_str(k)).collect();
                h.new_array(out)
            }),
            Err(e) => abort(vm, e),
        };
    }
    let keys = with_host(|h| h.enum_keys(&v));
    with_host(|h| h.new_array(keys))
}

fn b_foriter(vm: &mut VM, _: u8) -> Value {
    let it = match vm.stack.last() {
        Some(v) => v.clone(),
        None => return abort(vm, "internal: FORITER with empty stack".into()),
    };
    // Eager array-backed iterator (arrays/strings/Map/Set).
    let eager = with_host(|h| {
        if let Some(JsObj::Iter { items, idx }) = h.get_mut(&it) {
            if *idx < items.len() {
                let v = items[*idx].clone();
                *idx += 1;
                return Some(Some(v));
            }
            return Some(None);
        }
        None
    });
    if let Some(step) = eager {
        return match step {
            Some(v) => {
                vm.push(v);
                Value::Bool(true)
            }
            None => Value::Bool(false),
        };
    }
    // Generator: resume one step.
    if with_host(|h| h.is_generator_val(&it)) {
        return match host::gen_resume(&it, Value::Undef) {
            Ok(host::GenStep::Yield(v)) => {
                vm.push(v);
                Value::Bool(true)
            }
            Ok(host::GenStep::Done(_)) => Value::Bool(false),
            Err(e) => abort(vm, e),
        };
    }
    // A user iterator object with a `.next()` returning `{ value, done }`.
    match host::call_method(&it, "next", Vec::new()) {
        Ok(step) => {
            let done = get_property(&step, "done")
                .map(|d| with_host(|h| h.truthy(&d)))
                .unwrap_or(true);
            if done {
                Value::Bool(false)
            } else {
                match get_property(&step, "value") {
                    Ok(v) => {
                        vm.push(v);
                        Value::Bool(true)
                    }
                    Err(e) => abort(vm, e),
                }
            }
        }
        Err(e) => abort(vm, e),
    }
}

fn b_unpack(vm: &mut VM, _: u8) -> Value {
    let star = match vm.pop() {
        Value::Int(n) => n,
        _ => -1,
    };
    let count = match vm.pop() {
        Value::Int(n) => n as usize,
        _ => 0,
    };
    let iterable = vm.pop();
    let items = match host::iter_all(&iterable) {
        Ok(v) => v,
        Err(e) => return abort(vm, e),
    };
    let ordered: Vec<Value> = if star < 0 {
        (0..count)
            .map(|i| items.get(i).cloned().unwrap_or(Value::Undef))
            .collect()
    } else {
        let si = star as usize;
        let after = count.saturating_sub(si + 1);
        let rest_end = items.len().saturating_sub(after).max(si);
        let mut out: Vec<Value> = Vec::with_capacity(count);
        for i in 0..si {
            out.push(items.get(i).cloned().unwrap_or(Value::Undef));
        }
        let rest: Vec<Value> = items
            .get(si..rest_end)
            .map(|s| s.to_vec())
            .unwrap_or_default();
        out.push(with_host(|h| h.new_array(rest)));
        for j in 0..after {
            out.push(items.get(rest_end + j).cloned().unwrap_or(Value::Undef));
        }
        out
    };
    if ordered.is_empty() {
        return Value::Undef;
    }
    for it in ordered[1..].iter().rev().cloned() {
        vm.push(it);
    }
    ordered[0].clone()
}

fn b_build_args(vm: &mut VM, argc: u8) -> Value {
    let flat = pop_n(vm, argc as usize);
    let mut out = Vec::new();
    // Elided positions of an array literal (tag 2), recorded as the run-time
    // index each lands on — which only this walk knows, because a preceding
    // spread contributes an unknown number of elements. Call-argument lists,
    // the other `BUILD_ARGS` caller, cannot contain an elision, so this stays
    // empty for them.
    let mut holes: rustc_hash::FxHashSet<usize> = rustc_hash::FxHashSet::default();
    let mut i = 0;
    while i + 1 < flat.len() {
        let val = flat[i + 1].clone();
        match flat[i] {
            Value::Int(1) => match host::iter_all(&val) {
                Ok(items) => out.extend(items),
                Err(e) => return abort(vm, e),
            },
            Value::Int(2) => {
                holes.insert(out.len());
                out.push(Value::Undef);
            }
            _ => out.push(val),
        }
        i += 2;
    }
    with_host(|h| {
        let arr = h.new_array(out);
        h.install_holes(&arr, holes);
        arr
    })
}

// ── calls ──────────────────────────────────────────────────────────────────────

fn b_call(vm: &mut VM, argc: u8) -> Value {
    let mut args = pop_n(vm, argc as usize);
    let name = sval(&args.remove(0));
    let r = host::call_named(&name, args);
    // A bare name that resolved to a non-callable reports the VALUE
    // (`undefined is not a function`); node names the identifier. Resolving it
    // again to learn what the message said costs nothing off the error path.
    let r = r.map_err(|e| {
        let shown = global_binding(&name)
            .map(|v| with_host(|h| h.str_of(&v)))
            .unwrap_or_default();
        host::name_call_site(vm, &shown, e)
    });
    finish(vm, r)
}

fn b_call_method(vm: &mut VM, argc: u8) -> Value {
    let mut args = pop_n(vm, argc as usize);
    let recv = args.remove(0);
    let name = sval(&args.remove(0));
    let r = host::call_method(&recv, &name, args);
    // `z.f()` on a missing method is `z.f is not a function` in node, not
    // `f is not a function`: V8 names the callee as the source wrote it. The
    // text was recorded for this op at compile time.
    let r = r.map_err(|e| host::name_call_site(vm, &name, e));
    finish(vm, r)
}

fn b_call_value(vm: &mut VM, argc: u8) -> Value {
    let mut args = pop_n(vm, argc as usize);
    let callable = args.remove(0);
    let r = host::invoke(&callable, args, None);
    // The callee here is an expression, not a name, so the message it produced
    // describes the VALUE (`undefined is not a function`); node names the
    // expression. Same site table, keyed on that rendering.
    let r = r.map_err(|e| {
        let shown = with_host(|h| h.str_of(&callable));
        host::name_call_site(vm, &shown, e)
    });
    finish(vm, r)
}

fn b_new(vm: &mut VM, argc: u8) -> Value {
    let mut args = pop_n(vm, argc as usize);
    let ctor = args.remove(0);
    let r = host::construct(&ctor, args);
    // `new (o.a.b.c)()` on a non-constructor names the expression, as a failed
    // call does.
    let r = r.map_err(|e| {
        let shown = with_host(|h| h.str_of(&ctor));
        host::name_call_site(vm, &shown, e)
    });
    finish(vm, r)
}

fn b_apply(vm: &mut VM, _: u8) -> Value {
    let args_arr = vm.pop();
    let callable = vm.pop();
    let args = host::iter_all(&args_arr).unwrap_or_default();
    let r = host::invoke(&callable, args, None);
    finish(vm, r)
}

fn b_apply_method(vm: &mut VM, _: u8) -> Value {
    let args_arr = vm.pop();
    let name = sval(&vm.pop());
    let recv = vm.pop();
    let args = host::iter_all(&args_arr).unwrap_or_default();
    let r = host::call_method(&recv, &name, args);
    finish(vm, r)
}

// ── numeric hook ──────────────────────────────────────────────────────────────

/// Host callback for arithmetic fusevm cannot complete natively (a non-`Int`/
/// non-`Float` operand). Supplies JavaScript `+` concatenation and coercion.
///
/// Every operand is run through `ToPrimitive` FIRST (ECMA-262 13.15.3 for `+`,
/// 13.6.3 for the other arithmetic ops, 13.10.1 for the relational ones), which
/// is what invokes a user `valueOf`/`Symbol.toPrimitive`. It has to happen here
/// rather than inside `JsHost::arith`, because calling back into JS re-enters
/// the VM and `arith` runs under the host's `RefCell` borrow.
pub fn numeric_hook(op: NumOp, a: &Value, b: &Value) -> Result<Value, String> {
    use NumOp::*;
    let (a, b) = match op {
        // `==`/`!=` only convert when the OTHER side is a primitive that can be
        // compared numerically or textually; `{} == {}` stays a reference check.
        Eq | Ne => {
            let (pa, pb) = with_host(|h| (host::is_primitive(h, a), host::is_primitive(h, b)));
            match (pa, pb) {
                (false, true) if coerces_against_object(b) => {
                    (host::to_primitive(a, "default")?, b.clone())
                }
                (true, false) if coerces_against_object(a) => {
                    (a.clone(), host::to_primitive(b, "default")?)
                }
                _ => (a.clone(), b.clone()),
            }
        }
        // `+` uses the default hint (`valueOf` first, but a string result still
        // selects concatenation); everything else uses the number hint.
        Add => (
            host::to_primitive(a, "default")?,
            host::to_primitive(b, "default")?,
        ),
        _ => (
            host::to_primitive(a, "number")?,
            host::to_primitive(b, "number")?,
        ),
    };
    reject_symbol_operand(op, &a, &b)?;
    with_host(|h| h.arith(op, &a, &b))
}

/// A symbol has no `ToNumber` and no `ToString`, so every operator except the
/// equality family rejects it (7.1.4 step 2, 7.1.17 step 2). node-js instead
/// concatenated `Symbol(desc)` into the result.
///
/// Which of the two messages V8 uses is decided by whether the operation is
/// STRING concatenation — measured on node v26.7.0, `Symbol() + ''` is
/// `Cannot convert a Symbol value to a string` while `Symbol() + 1`,
/// `Symbol() + Symbol()` and `Symbol() * 1` are all
/// `Cannot convert a Symbol value to a number`. `==`/`===` never convert
/// (`Symbol() == 1` is `false`), so they are left alone.
fn reject_symbol_operand(op: NumOp, a: &Value, b: &Value) -> Result<(), String> {
    use NumOp::*;
    if matches!(op, Eq | Ne) {
        return Ok(());
    }
    let (sym, concat) = with_host(|h| {
        let is_sym = |v: &Value| matches!(h.get(v), Some(JsObj::Symbol { .. }));
        let is_str =
            |v: &Value| matches!(v, Value::Str(_)) || matches!(h.get(v), Some(JsObj::Str(_)));
        (is_sym(a) || is_sym(b), is_str(a) || is_str(b))
    });
    if !sym {
        return Ok(());
    }
    Err(host::type_error(if matches!(op, Add) && concat {
        "Cannot convert a Symbol value to a string"
    } else {
        "Cannot convert a Symbol value to a number"
    }))
}

/// Whether a primitive `v` makes `==` against an object convert that object
/// (7.2.15 steps 10-11): numbers, strings, bigints and symbols do; `null`,
/// `undefined` and booleans are settled without a `ToPrimitive` call
/// (a boolean is coerced to a number first, and then it does).
fn coerces_against_object(v: &Value) -> bool {
    match v {
        Value::Undef => false,
        Value::Bool(_) | Value::Int(_) | Value::Float(_) | Value::Str(_) => true,
        _ => with_host(|h| !h.is_null(v)),
    }
}

// ══ standard library ═══════════════════════════════════════════════════════════

/// Namespaces reachable as bare globals.
fn is_namespace(name: &str) -> bool {
    matches!(
        name,
        "console"
            | "Math"
            | "JSON"
            | "Object"
            | "Array"
            | "Number"
            | "String"
            | "Boolean"
            | "Symbol"
            | "Reflect"
            | "Promise"
            | "process"
            | "Buffer"
            | "URL"
            | "URLSearchParams"
    )
}

const GLOBAL_FUNCS: &[&str] = &[
    "parseInt",
    "parseFloat",
    "isNaN",
    "isFinite",
    "encodeURIComponent",
    "decodeURIComponent",
    "encodeURI",
    "decodeURI",
    // Annex B legacy encoders. Still globals on every engine, and still called
    // by pre-`encodeURIComponent` library code.
    "escape",
    "unescape",
    "eval",
    "String",
    "Number",
    "Boolean",
    "Array",
    "Object",
    "Function",
    "Symbol",
    "Map",
    "Set",
    "WeakMap",
    "WeakSet",
    "Promise",
    "Error",
    "TypeError",
    "RangeError",
    "SyntaxError",
    "ReferenceError",
    "EvalError",
    "URIError",
    "AggregateError",
    "BigInt",
    "RegExp",
    "Date",
    "ArrayBuffer",
    "Uint8Array",
    "Int8Array",
    "Uint8ClampedArray",
    "Int16Array",
    "Uint16Array",
    "Int32Array",
    "Uint32Array",
    "Float32Array",
    "Float64Array",
    "BigInt64Array",
    "BigUint64Array",
    "WeakRef",
    "FinalizationRegistry",
    "TextEncoder",
    "TextDecoder",
    // WHATWG Fetch globals (see `stdlib::fetch`).
    "fetch",
    "Headers",
    "Request",
    "Response",
    "Blob",
    "File",
    "FormData",
    "AbortController",
    "AbortSignal",
    "queueMicrotask",
    "setTimeout",
    "setInterval",
    "setImmediate",
    "clearTimeout",
    "clearInterval",
    "clearImmediate",
    "structuredClone",
    "Proxy",
    "require",
    // CommonJS loader dispatch targets referenced by per-module `require`
    // closures (see `module.rs`); never written by user code.
    "__cjs_require",
    "__cjs_resolve",
    "__cjs_cache",
];

const NS_METHODS: &[&str] = &[
    "console.log",
    "console.error",
    "console.warn",
    "console.info",
    "console.debug",
    "Math.floor",
    "Math.ceil",
    "Math.round",
    "Math.trunc",
    "Math.abs",
    "Math.sign",
    "Math.max",
    "Math.min",
    "Math.pow",
    "Math.sqrt",
    "Math.cbrt",
    "Math.random",
    "Math.hypot",
    "Math.clz32",
    "Math.fround",
    "Math.imul",
    "Math.sinh",
    "Math.cosh",
    "Math.tanh",
    "Math.asinh",
    "Math.acosh",
    "Math.atanh",
    "Math.log1p",
    "Math.expm1",
    "Math.log",
    "Math.log2",
    "Math.log10",
    "Math.exp",
    "Math.sin",
    "Math.cos",
    "Math.tan",
    "Math.atan",
    "Math.atan2",
    "Math.asin",
    "Math.acos",
    "JSON.stringify",
    "JSON.parse",
    "Object.keys",
    "Object.values",
    "Object.entries",
    "Object.assign",
    "Object.freeze",
    "Object.is",
    "Object.fromEntries",
    "Object.getPrototypeOf",
    "Object.setPrototypeOf",
    "Object.create",
    "Object.getOwnPropertyNames",
    "Object.getOwnPropertySymbols",
    "Object.defineProperty",
    "Object.getOwnPropertyDescriptor",
    "Object.getOwnPropertyDescriptors",
    "Object.defineProperties",
    "Object.isFrozen",
    "Object.isSealed",
    "Object.seal",
    "Object.preventExtensions",
    "Object.isExtensible",
    "Object.hasOwn",
    "Object.groupBy",
    "Array.isArray",
    "Array.from",
    "Array.fromAsync",
    "Array.of",
    "Number.isInteger",
    "Number.isNaN",
    "Number.isFinite",
    "Number.isSafeInteger",
    "Number.parseInt",
    "Number.parseFloat",
    "String.fromCharCode",
    "String.fromCodePoint",
    "String.raw",
    "Symbol.for",
    "Symbol.keyFor",
    "BigInt.asIntN",
    "BigInt.asUintN",
    "Proxy.revocable",
    "Reflect.ownKeys",
    "Reflect.has",
    "Reflect.get",
    "Reflect.set",
    "Reflect.getPrototypeOf",
    "Reflect.setPrototypeOf",
    "Reflect.getOwnPropertyDescriptor",
    "Reflect.defineProperty",
    "Reflect.deleteProperty",
    "Reflect.apply",
    "Reflect.construct",
    "Reflect.isExtensible",
    "Reflect.preventExtensions",
    "Promise.resolve",
    "Promise.reject",
    "Promise.all",
    "Promise.allSettled",
    "Promise.race",
    "Promise.any",
    "Promise.withResolvers",
    "Map.groupBy",
    "Response.json",
    "Response.error",
    "Response.redirect",
    "AbortSignal.abort",
    "AbortSignal.timeout",
    "process.nextTick",
    "Error.captureStackTrace",
    "require.resolve",
];

pub fn is_known_builtin(name: &str) -> bool {
    GLOBAL_FUNCS.contains(&name)
        || NS_METHODS.contains(&name)
        || is_namespace(name)
        || crate::stdlib::is_method(name)
}

// ── dynamic functions (runtime source → callable) ────────────────────────────

/// Build a callable from a complete function-expression source text — the ONE
/// dynamic-function generator on this frontend.
///
/// `src` is the exact source V8 synthesizes for the construct, WITHOUT the
/// wrapping parentheses needed to parse it as an expression: those are added
/// here, and `src` itself is retained so `Function.prototype.toString` reports
/// what V8 reports. The two callers synthesize different text and both shapes
/// are observable — see `stdlib::vm::compile_function` for the measured diff.
///
/// The body runs in the MODULE scope, never the constructing function's scope
/// (20.2.1.1.1 step 26 instantiates a dynamic function's body against the
/// *global* environment). That also makes a `var` inside the body a function
/// local: measured on node v26.7.0, `new Function('a','var zz = 5; return zz + a')`
/// returns 6 and leaves `globalThis.zz` `undefined`.
pub fn dynamic_function(src: &str) -> Result<Value, String> {
    let f = crate::eval_in_global_scope(&format!("({src})"))?;
    with_host(|h| {
        let s = h.new_str(src.to_string());
        h.set_fn_prop(&f, "@@source", s);
    });
    Ok(f)
}

/// `new Function(p1, …, pN, body)` / `Function(p1, …, pN, body)`.
///
/// Argument convention (20.2.1.1.1): the LAST argument is the body and the rest
/// are parameter-list fragments joined with `,` — so a fragment may itself hold
/// several parameters (`new Function('a,b', 'c', …)` takes three). With no
/// arguments at all, both the parameter list and the body are empty.
///
/// Measured on node v26.7.0:
///
/// ```text
/// new Function('a','b','return a+b').toString() === 'function anonymous(a,b\n) {\nreturn a+b\n}'
/// new Function().toString()                     === 'function anonymous(\n) {\n\n}'
/// new Function('a,b','c','return [a,b,c]').length === 3
/// new Function('a','b','return a+b').name       === 'anonymous'
/// ```
pub fn function_ctor(args: &[Value]) -> Result<Value, String> {
    let parts: Vec<String> = args.iter().map(|a| with_host(|h| h.str_of(a))).collect();
    let (params, body) = match parts.split_last() {
        Some((body, params)) => (params.join(","), body.clone()),
        None => (String::new(), String::new()),
    };
    dynamic_function(&format!("function anonymous({params}\n) {{\n{body}\n}}"))
}

/// `eval(src)`. `direct` selects the scope the source runs in: a DIRECT eval —
/// the literal `eval(...)` call form — evaluates in the CALLER's scope, every
/// other route to the same function value is an INDIRECT eval and evaluates in
/// the global scope (ECMA-262 19.2.1.1 `PerformEval`). The two are told apart in
/// `host::call_named`, which `ops::CALL` reaches and `ops::CALL_VALUE`/`APPLY`
/// do not.
///
/// A non-string argument is returned unchanged (19.2.1.1 step 2).
pub fn eval_source(arg: Option<&Value>, direct: bool) -> Result<Value, String> {
    let v = arg.cloned().unwrap_or(Value::Undef);
    let is_string =
        matches!(v, Value::Str(_)) || with_host(|h| matches!(h.get(&v), Some(JsObj::Str(_))));
    if !is_string {
        return Ok(v);
    }
    let src = with_host(|h| h.str_of(&v));
    let chunk = crate::load_merged(crate::compile_completion(&src)?);
    if direct {
        host::run_chunk_on(chunk)
    } else {
        host::run_chunk_in_global_scope(chunk)
    }
}

/// Call a resolved builtin function (global or `namespace.method`).
pub fn call_builtin_function(name: &str, args: Vec<Value>) -> Result<Value, String> {
    // `require(spec)`: the ENTRY script's top-level require — core module first,
    // else the CommonJS loader resolving from the entry file's directory.
    if name == "require" {
        let spec = with_host(|h| h.str_of(&arg0(&args)));
        return crate::module::require(&spec, &crate::module::entry_dir());
    }
    // `__cjs_require(spec, fromDir)`: a per-module `require` closure's dispatch
    // into the loader, resolving `spec` against the module's own directory.
    if name == "__cjs_require" {
        let spec = with_host(|h| h.str_of(&arg0(&args)));
        let from = with_host(|h| h.str_of(args.get(1).unwrap_or(&Value::Undef)));
        return crate::module::require(&spec, std::path::Path::new(&from));
    }
    // `require.resolve(spec)` at the ENTRY level: resolve from the entry dir.
    if name == "require.resolve" {
        let spec = with_host(|h| h.str_of(&arg0(&args)));
        if crate::stdlib::resolve(&spec).is_some() {
            return Ok(with_host(|h| h.new_str(spec)));
        }
        return match crate::module::resolve(&spec, &crate::module::entry_dir()) {
            Some(p) => Ok(with_host(|h| h.new_str(p.to_string_lossy().to_string()))),
            None => Err(crate::host::plain_coded_error(
                "Error",
                "MODULE_NOT_FOUND",
                &format!("Cannot find module '{spec}'"),
            )),
        };
    }
    // `__cjs_resolve(spec, fromDir)`: `require.resolve` — the resolved absolute
    // path (core modules resolve to the bare specifier, as in Node).
    if name == "__cjs_resolve" {
        let spec = with_host(|h| h.str_of(&arg0(&args)));
        let from = with_host(|h| h.str_of(args.get(1).unwrap_or(&Value::Undef)));
        if crate::stdlib::resolve(&spec).is_some() {
            return Ok(with_host(|h| h.new_str(spec)));
        }
        return match crate::module::resolve(&spec, std::path::Path::new(&from)) {
            Some(p) => Ok(with_host(|h| h.new_str(p.to_string_lossy().to_string()))),
            None => Err(crate::host::plain_coded_error(
                "Error",
                "MODULE_NOT_FOUND",
                &format!("Cannot find module '{spec}'"),
            )),
        };
    }
    // `Error.captureStackTrace(target[, ctor])`: V8's stack capture. Sets
    // `target.stack`; when a custom `Error.prepareStackTrace` is installed (the
    // stack-introspection pattern used by `depd`), it is called with a synthetic
    // CallSite array and its result becomes `.stack`, else `.stack` is a string.
    if name == "Error.captureStackTrace" {
        let target = arg0(&args);
        let prep = with_host(|h| h.builtin_static("Error", "prepareStackTrace"));
        let stack = match prep {
            Some(f)
                if matches!(
                    with_host(|h| h.get(&f).cloned()),
                    Some(JsObj::Func(_)) | Some(JsObj::Builtin(_)) | Some(JsObj::BoundFunc { .. })
                ) =>
            {
                let sites = crate::module::callsite_stack(10)?;
                host::invoke(&f, vec![target.clone(), sites], None)?
            }
            _ => with_host(|h| h.new_str("")),
        };
        let _ = set_property(&target, "stack", stack);
        return Ok(Value::Undef);
    }
    // Native stdlib module methods (path/os/fs/util/assert/crypto/buffer/url).
    if let Some(r) = crate::stdlib::call(name, &args) {
        return r;
    }
    match name {
        "console.log" | "console.info" | "console.debug" => {
            print_line(&args, false);
            Ok(Value::Undef)
        }
        "console.error" | "console.warn" => {
            print_line(&args, true);
            Ok(Value::Undef)
        }
        "parseInt" | "Number.parseInt" => Ok(Value::Float(parse_int(&args))),
        "parseFloat" | "Number.parseFloat" => Ok(Value::Float(parse_float(&args))),
        "isNaN" => Ok(Value::Bool(arg_num(&args, 0).is_nan())),
        "isFinite" => Ok(Value::Bool(arg_num(&args, 0).is_finite())),
        "encodeURIComponent" => uri_encode(&with_host(|h| h.str_of(&arg0(&args))), false),
        "encodeURI" => uri_encode(&with_host(|h| h.str_of(&arg0(&args))), true),
        "decodeURIComponent" => uri_decode(&with_host(|h| h.str_of(&arg0(&args))), false),
        "decodeURI" => uri_decode(&with_host(|h| h.str_of(&arg0(&args))), true),
        "escape" => legacy_escape(&with_host(|h| h.str_of(&arg0(&args)))),
        "unescape" => legacy_unescape(&with_host(|h| h.str_of(&arg0(&args)))),
        // Reaching `eval` through this table means the eval FUNCTION VALUE was
        // called — `(0, eval)(src)`, `const e = eval; e(src)`, `[eval][0](src)`.
        // Those are INDIRECT evals and run in the global scope. A literal
        // `eval(src)` is intercepted earlier, in `host::call_named`.
        "eval" => eval_source(args.first(), false),
        // `new Function(...)` and `Function(...)` are the same operation
        // (20.2.1.1 `CreateDynamicFunction` is reached from both [[Call]] and
        // [[Construct]]), so both route to the one generator.
        "Function" => function_ctor(&args),
        // `Buffer(arg[, encodingOrOffset[, length]])` — the deprecated call form
        // (DEP0005). Node still supports it and still routes it to the same place
        // `new Buffer` goes, which is why `safe-buffer`'s legacy `SafeBuffer`
        // wrapper is just `return Buffer(arg, encodingOrOffset, length)`. Measured
        // on node v26.7.0: `Buffer('abc').toString() === 'abc'`,
        // `Buffer([1,2]).toString('hex') === '0102'`, `Buffer(3).length === 3`.
        // Node emits DEP0005 once, on stderr, through the same one-shot machinery
        // `url.parse`'s DEP0169 uses, so this does too rather than staying silent
        // where Node warns.
        "Buffer" => {
            crate::stdlib::process::emit_deprecation_warning(
                "DEP0005",
                "Buffer() is deprecated due to security and usability issues. \
                 Please use the Buffer.alloc(), Buffer.allocUnsafe(), or \
                 Buffer.from() methods instead.",
            );
            crate::stdlib::construct("Buffer", &args)
                .unwrap_or_else(|| Err(host::type_error("Buffer is not a function")))
        }
        "Number.isInteger" => Ok(Value::Bool(is_integer(arg0(&args)))),
        "Number.isSafeInteger" => Ok(Value::Bool(is_safe_integer(arg0(&args)))),
        "Number.isNaN" => Ok(Value::Bool(
            matches!(arg0(&args), Value::Float(f) if f.is_nan()),
        )),
        "Number.isFinite" => Ok(Value::Bool(
            matches!(arg0(&args), Value::Float(f) if f.is_finite())
                || matches!(arg0(&args), Value::Int(_)),
        )),
        "String" => {
            if args.is_empty() {
                Ok(with_host(|h| h.new_str("")))
            } else {
                // A symbol argument stringifies to `Symbol(desc)` (explicit String()
                // is allowed); everything else via ToString method dispatch.
                host::string_ctor_value(&args[0])
            }
        }
        "Number" => Ok(Value::Float(if args.is_empty() {
            0.0
        } else {
            // ToNumber, which for an object runs ToPrimitive (a JS `valueOf` call).
            host::to_number_value(&args[0])?
        })),
        "BigInt" => bigint_ctor(&arg0(&args)),
        "RegExp" => regexp_ctor(&args),
        "BigInt.asIntN" | "BigInt.asUintN" => bigint_as_n(name.ends_with("asUintN"), &args),
        "Boolean" => Ok(Value::Bool(with_host(|h| h.truthy(&arg0(&args))))),
        // Each argument is truncated to a uint16 and taken as one code UNIT, so
        // `String.fromCharCode(0x1D4B3)` is U+D4B3, NOT the astral U+1D4B3, and
        // a surrogate PAIR of arguments composes into one character.
        "String.fromCharCode" => Ok(with_host(|h| {
            let units: Vec<u16> = args
                .iter()
                .map(|a| crate::utf16::to_uint16(h.to_number(a)))
                .collect();
            let s = crate::utf16::to_string_lossy(&units);
            h.new_str(s)
        })),
        // `fromCodePoint` takes whole code POINTS and rejects anything that is
        // not one — including a lone surrogate, which `fromCharCode` accepts.
        "String.fromCodePoint" => {
            let mut s = String::new();
            for a in &args {
                let n = with_host(|h| h.to_number(a));
                let cp = if n.is_finite() && n.trunc() == n && (0.0..=0x10FFFF as f64).contains(&n)
                {
                    char::from_u32(n as u32)
                } else {
                    None
                };
                match cp {
                    Some(c) => s.push(c),
                    None => {
                        return Err(format!(
                            "RangeError: Invalid code point {}",
                            with_host(|h| h.str_of(a))
                        ))
                    }
                }
            }
            Ok(new_s(s))
        }
        "String.raw" => string_raw(&args),
        // `Array(5)` === `new Array(5)` (length-5 empty), but `Array.of(5)` is `[5]`.
        "Array" => construct_builtin("Array", args),
        "Array.of" => Ok(with_host(|h| h.new_array(args))),
        // 23.1.2.2 `IsArray` follows a Proxy to its `[[ProxyTarget]]` rather than
        // consulting any trap, so `Array.isArray(new Proxy([], {}))` is `true`.
        "Array.isArray" => {
            let v = arg0(&args);
            let subject = crate::proxy::ultimate_target(&v).unwrap_or(v);
            Ok(Value::Bool(matches!(
                with_host(|h| h.get(&subject).cloned()),
                Some(JsObj::Array(_))
            )))
        }
        "Array.from" => array_from(args),
        "Array.fromAsync" => array_from_async(args),
        "Object" => Ok(object_call(args)),
        "Object.keys" => object_keys(args, 0),
        "Object.values" => object_keys(args, 1),
        "Object.entries" => object_keys(args, 2),
        "Object.assign" => object_assign(args),
        "Object.freeze" => {
            let v = arg0(&args);
            with_host(|h| h.seal_object(&v, true));
            Ok(v)
        }
        "Object.seal" => {
            let v = arg0(&args);
            with_host(|h| h.seal_object(&v, false));
            Ok(v)
        }
        "Object.preventExtensions" => {
            let v = arg0(&args);
            if crate::proxy::prevent_extensions(&v)? {
                return Ok(v);
            }
            with_host(|h| h.prevent_extensions(&v));
            Ok(v)
        }
        "Object.isFrozen" => Ok(Value::Bool(with_host(|h| h.is_sealed(&arg0(&args), true)))),
        "Object.isSealed" => Ok(Value::Bool(with_host(|h| h.is_sealed(&arg0(&args), false)))),
        "Object.isExtensible" => {
            let v = arg0(&args);
            match crate::proxy::is_extensible(&v)? {
                Some(b) => Ok(Value::Bool(b)),
                None => Ok(Value::Bool(with_host(|h| h.is_extensible(&v)))),
            }
        }
        // Object.is — SameValue: like `===` but NaN is equal to NaN and +0 is
        // distinct from -0.
        "Object.is" => {
            let a = arg0(&args);
            let b = args.get(1).cloned().unwrap_or(Value::Undef);
            let num = |v: &Value| match v {
                Value::Int(n) => Some(*n as f64),
                Value::Float(f) => Some(*f),
                _ => None,
            };
            let r = match (num(&a), num(&b)) {
                (Some(x), Some(y)) => {
                    if x.is_nan() && y.is_nan() {
                        true
                    } else if x == 0.0 && y == 0.0 {
                        x.is_sign_negative() == y.is_sign_negative()
                    } else {
                        x == y
                    }
                }
                _ => with_host(|h| h.strict_eq(&a, &b)),
            };
            Ok(Value::Bool(r))
        }
        "Object.fromEntries" => object_from_entries(args),
        // `[[GetPrototypeOf]]`: a Proxy answers from its trap (which may throw),
        // so the proxy form cannot share `prototype_of`'s infallible signature.
        "Object.getPrototypeOf" | "Reflect.getPrototypeOf" => {
            let v = arg0(&args);
            match crate::proxy::get_prototype_of(&v)? {
                Some(p) => Ok(p),
                None => Ok(prototype_of(&v)),
            }
        }
        "Object.setPrototypeOf" => {
            let obj = arg0(&args);
            let proto = args.get(1).cloned().unwrap_or(Value::Undef);
            if with_host(|h| h.kind_of(&obj)) == Some(ObjKind::Proxy) {
                reject_bad_prototype(&proto)?;
                crate::proxy::set_prototype_of(&obj, &proto)?;
                return Ok(obj);
            }
            // 20.1.2.23: `RequireObjectCoercible` on the target, then the
            // prototype type check, then — only for an actual object target —
            // the extensibility check. A PRIMITIVE target is returned untouched
            // (`Object.setPrototypeOf(1, {})` is `1`), which is why the
            // extensibility test cannot come first.
            if with_host(|h| matches!(obj, Value::Undef) || h.is_null(&obj)) {
                return Err(host::type_error(
                    "Object.setPrototypeOf called on null or undefined",
                ));
            }
            reject_bad_prototype(&proto)?;
            if with_host(|h| is_object_like(h, &obj)) {
                // Setting the SAME prototype is a no-op and stays legal even on a
                // frozen object: node v26.7.0 accepts
                // `Object.setPrototypeOf(Object.freeze({}), Object.prototype)`.
                // `prototype_of`, not `proto_of`: an object with no EXPLICIT
                // link still has `Object.prototype`, and comparing against the
                // absent link would call that a change.
                let cur = prototype_of(&obj);
                let same = with_host(|h| h.strict_eq(&cur, &proto));
                if !same && !with_host(|h| h.is_extensible(&obj)) {
                    return Err(host::type_error("#<Object> is not extensible"));
                }
                with_host(|h| h.set_proto(&obj, proto));
            }
            Ok(obj)
        }
        "Object.create" => object_create(args),
        "Object.getOwnPropertyNames" => object_keys(args, 3),
        "Object.getOwnPropertySymbols" => {
            let v = arg0(&args);
            require_object_coercible(&v)?;
            let syms = proxy_or_own_symbol_keys(&v)?;
            Ok(with_host(|h| h.new_array(syms)))
        }
        // `Object.hasOwn(obj, key)` — the static form of `hasOwnProperty`.
        "Object.hasOwn" => {
            let obj = arg0(&args);
            let key = args.get(1).cloned().unwrap_or(Value::Undef);
            object_builtin_method(&obj, "hasOwnProperty", vec![key])
        }
        "Object.defineProperty" => object_define_property(args),
        "Object.getOwnPropertyDescriptor" => object_get_own_descriptor(args),
        "Object.getOwnPropertyDescriptors" => object_get_own_descriptors(args),
        "Object.defineProperties" => object_define_properties(args),
        // `Object.groupBy(items, cb)` (ES2024): group into a null-prototype object
        // keyed by `ToPropertyKey(cb(item, i))`, each value an array of members.
        "Object.groupBy" => object_group_by(args),
        "Symbol" => Ok(with_host(|h| {
            let desc = args
                .first()
                .filter(|a| !matches!(a, Value::Undef))
                .map(|a| h.str_of(a));
            h.new_symbol(desc)
        })),
        "Symbol.for" => Ok(with_host(|h| {
            let key = h.str_of(&arg0(&args));
            h.symbol_for(&key)
        })),
        // `Symbol.keyFor(sym)` (20.4.2.6) is a REGISTRY lookup, not a
        // description read: it answers only for symbols `Symbol.for` created.
        // Returning the description made every symbol look registered —
        // `Symbol.keyFor(Symbol("k"))` was `"k"` where node says `undefined`.
        "Symbol.keyFor" => Ok(with_host(|h| h.symbol_registry_key(&arg0(&args)))),
        "Map" | "WeakMap" | "Set" | "WeakSet" | "Promise" => construct_builtin(name, args),
        // `Proxy` has no `[[Call]]` slot: it is constructor-only (28.2.1).
        "Proxy" => Err(host::type_error("Constructor Proxy requires 'new'")),
        "Proxy.revocable" => crate::proxy::revocable(&args),
        // `Reflect.ownKeys` reports EVERY own key, non-enumerable included —
        // the same set as `getOwnPropertyNames` (node-js has no symbol-keyed
        // own properties, so there is no second half to append).
        // `Reflect.ownKeys` is `OwnPropertyKeys` (7.3.23): every own key,
        // non-enumerable included, strings first and then the SYMBOLS.
        "Reflect.ownKeys" => {
            let v = arg0(&args);
            let names = object_keys(args, 3)?;
            let syms = proxy_or_own_symbol_keys(&v)?;
            if syms.is_empty() {
                return Ok(names);
            }
            let mut all = with_host(|h| h.iter_vec(&names)).unwrap_or_default();
            all.extend(syms);
            Ok(with_host(|h| h.new_array(all)))
        }
        "Reflect.getOwnPropertyDescriptor" => object_get_own_descriptor(args),
        "Reflect.defineProperty" => {
            object_define_property(args)?;
            Ok(Value::Bool(true))
        }
        "Reflect.deleteProperty" => {
            let obj = arg0(&args);
            let k = with_host(|h| h.property_key(&args.get(1).cloned().unwrap_or(Value::Undef)));
            Ok(Value::Bool(delete_property(&obj, &k)?))
        }
        "Reflect.setPrototypeOf" => {
            let obj = arg0(&args);
            let p = args.get(1).cloned().unwrap_or(Value::Undef);
            if with_host(|h| h.kind_of(&obj)) == Some(ObjKind::Proxy) {
                crate::proxy::set_prototype_of(&obj, &p)?;
                return Ok(Value::Bool(true));
            }
            with_host(|h| h.set_proto(&obj, p));
            Ok(Value::Bool(true))
        }
        "Reflect.isExtensible" => {
            let v = arg0(&args);
            match crate::proxy::is_extensible(&v)? {
                Some(b) => Ok(Value::Bool(b)),
                None => Ok(Value::Bool(with_host(|h| h.is_extensible(&v)))),
            }
        }
        "Reflect.preventExtensions" => {
            let v = arg0(&args);
            if crate::proxy::prevent_extensions(&v)? {
                return Ok(Value::Bool(true));
            }
            with_host(|h| h.prevent_extensions(&v));
            Ok(Value::Bool(true))
        }
        // `Reflect.apply(target, thisArg, argsList)` / `Reflect.construct(t, a)`.
        "Reflect.apply" => {
            let f = arg0(&args);
            let this = args.get(1).cloned();
            let list = with_host(|h| h.iter_vec(&args.get(2).cloned().unwrap_or(Value::Undef)))
                .unwrap_or_default();
            host::invoke(&f, list, this.filter(|t| !with_host(|h| h.is_nullish(t))))
        }
        "Reflect.construct" => {
            let f = arg0(&args);
            let list = with_host(|h| h.iter_vec(&args.get(1).cloned().unwrap_or(Value::Undef)))
                .unwrap_or_default();
            host::construct(&f, list)
        }
        "Reflect.has" => {
            let obj = arg0(&args);
            let k = with_host(|h| h.property_key(&args.get(1).cloned().unwrap_or(Value::Undef)));
            Ok(Value::Bool(has_property(&obj, &k)?))
        }
        // `Reflect.get(target, key, receiver)` — the optional third argument is
        // what a getter sees as `this` (28.1.6). Defaults to the target.
        "Reflect.get" => {
            let obj = arg0(&args);
            let k = with_host(|h| h.property_key(&args.get(1).cloned().unwrap_or(Value::Undef)));
            let receiver = args.get(2).cloned().unwrap_or_else(|| obj.clone());
            get_property_recv(&obj, &k, &receiver)
        }
        "Reflect.set" => {
            let obj = arg0(&args);
            let k = with_host(|h| h.property_key(&args.get(1).cloned().unwrap_or(Value::Undef)));
            let v = args.get(2).cloned().unwrap_or(Value::Undef);
            let _ = set_property(&obj, &k, v);
            Ok(Value::Bool(true))
        }
        "JSON.stringify" => json_stringify(args),
        "JSON.parse" => json_parse(args),
        "structuredClone" => Ok(deep_clone(&arg0(&args))),
        "fetch" => crate::stdlib::fetch::fetch(&args),
        // An `AbortSignal.timeout` deadline reached its macrotask: the thunk's
        // suffix is the signal's heap index.
        _ if name.starts_with("@@aborttimeout:") => {
            let idx: u32 = name["@@aborttimeout:".len()..].parse().unwrap_or(0);
            crate::stdlib::fetch::fire_timeout_abort(idx)
        }
        "queueMicrotask" | "process.nextTick" => {
            let cb = arg0(&args);
            let rest = args.get(1..).map(|s| s.to_vec()).unwrap_or_default();
            enqueue_microtask(name == "process.nextTick", cb, rest);
            Ok(Value::Undef)
        }
        "setTimeout" | "setInterval" | "setImmediate" => Ok(schedule_timer(name, args)),
        "clearTimeout" | "clearInterval" | "clearImmediate" => {
            clear_timer(&arg0(&args));
            Ok(Value::Undef)
        }
        "Promise.resolve" => promise_resolve(arg0(&args)),
        "Promise.reject" => promise_reject(arg0(&args)),
        "Promise.all" => promise_all(args, AllMode::All),
        "Promise.allSettled" => promise_all(args, AllMode::AllSettled),
        "Promise.race" => promise_race(args, false),
        "Promise.any" => promise_race(args, true),
        // `Promise.withResolvers()` (ES2024): a new pending promise plus its own
        // resolve/reject functions, returned as `{ promise, resolve, reject }`.
        "Promise.withResolvers" => promise_with_resolvers(),
        // `Map.groupBy(items, cb)` (ES2024): group into a `Map` keyed by the raw
        // `cb(item, i)` result (SameValueZero), each value an array of members.
        "Map.groupBy" => map_group_by(args),
        n if host::ERROR_NAMES.contains(&n) => Ok(make_error(name, &args)),
        _ if name.starts_with("Math.") => math_fn(&name[5..], &args),
        // Internal continuations (Promise resolve/reject fns, `.finally` wrappers).
        _ if name.starts_with("@@presolve:") => {
            let id: u32 = name[11..].parse().unwrap_or(0);
            host::resolve_promise_val(id, arg0(&args));
            Ok(Value::Undef)
        }
        _ if name.starts_with("@@preject:") => {
            let id: u32 = name[10..].parse().unwrap_or(0);
            host::reject_promise_val(id, arg0(&args));
            Ok(Value::Undef)
        }
        // The revoker `Proxy.revocable` hands back, keyed by the proxy's heap
        // index so calling it twice is the spec's no-op rather than a re-tear.
        _ if name.starts_with("@@prevoke:") => {
            let i: u32 = name[10..].parse().unwrap_or(0);
            Ok(crate::proxy::revoke(i))
        }
        _ if name.starts_with("@@finpass:") => {
            // finally(cb) on fulfill: run cb, then pass the value through.
            let i: u32 = name[10..].parse().unwrap_or(0);
            let cb = Value::Obj(i);
            host::invoke(&cb, Vec::new(), None)?;
            Ok(arg0(&args))
        }
        _ if name.starts_with("@@finthrow:") => {
            // finally(cb) on reject: run cb, then re-throw the reason.
            let i: u32 = name[11..].parse().unwrap_or(0);
            let cb = Value::Obj(i);
            host::invoke(&cb, Vec::new(), None)?;
            let reason = arg0(&args);
            with_host(|h| h.exc = Some(reason.clone()));
            Err(with_host(|h| error_string(h, &reason)))
        }
        _ => Err(host::type_error(&format!("{name} is not a function"))),
    }
}

/// `BigInt(x)`: convert a boolean/number/string/bigint to a BigInt. A
/// non-integer number is a `RangeError`; an unparseable string a `SyntaxError`
/// (matching Node's messages).
/// V8 names the offending value: `BigInt(undefined)` is `Cannot convert
/// undefined to a BigInt`, `BigInt({})` is `Cannot convert [object Object] to a
/// BigInt`. The old text said "value" literally, for every input.
fn bigint_convert_error(v: &Value) -> String {
    let shown = with_host(|h| h.str_of(v));
    host::type_error(&format!("Cannot convert {shown} to a BigInt"))
}

fn bigint_ctor(v: &Value) -> Result<Value, String> {
    use num_bigint::BigInt;
    let big = match v {
        Value::Bool(b) => BigInt::from(*b as i64),
        Value::Int(n) => BigInt::from(*n),
        Value::Float(f) => {
            if !f.is_finite() || f.fract() != 0.0 {
                let disp = with_host(|h| h.str_of(v));
                return Err(format!(
                    "RangeError: The number {disp} cannot be converted to a BigInt because it is not an integer"
                ));
            }
            // The decimal EXPANSION, not `fmt_number`: `Number.prototype
            // .toString` switches to exponential notation at 1e21, and
            // `BigInt::parse_bytes` cannot read `"1e+21"` — so `BigInt(1e21)`
            // threw `Cannot convert value to a BigInt` where node returns
            // `1000000000000000000000n`. `{:.0}` prints an integral f64's exact
            // value, which is also what node reports for a magnitude past the
            // exactly-representable range (`BigInt(1e30)` is
            // `1000000000000000019884624838656n` in both).
            match BigInt::parse_bytes(format!("{f:.0}").as_bytes(), 10) {
                Some(b) => b,
                None => return Err(bigint_convert_error(v)),
            }
        }
        Value::Str(s) => match host::parse_bigint_str(s) {
            Some(b) => b,
            None => return Err(format!("SyntaxError: Cannot convert {s} to a BigInt")),
        },
        Value::Obj(_) => match with_host(|h| h.get(v).cloned()) {
            Some(JsObj::BigInt(b)) => b,
            Some(JsObj::Str(s)) => match host::parse_bigint_str(&s) {
                Some(b) => b,
                None => return Err(format!("SyntaxError: Cannot convert {s} to a BigInt")),
            },
            _ => return Err(bigint_convert_error(v)),
        },
        _ => return Err(bigint_convert_error(v)),
    };
    Ok(with_host(|h| h.new_bigint(big)))
}

/// `new RegExp(source[, flags])` / `RegExp(...)`. A first `RegExp` argument copies
/// its source (and flags, unless new ones are given).
fn regexp_ctor(args: &[Value]) -> Result<Value, String> {
    let (source, existing_flags) = match with_host(|h| h.get(&arg0(args)).cloned()) {
        Some(JsObj::RegExp(r)) => (r.source.clone(), Some(r.flags.clone())),
        _ => {
            let a0 = arg0(args);
            let src = if matches!(a0, Value::Undef) {
                String::new()
            } else {
                with_host(|h| h.str_of(&a0))
            };
            (src, None)
        }
    };
    let flags = match args.get(1) {
        Some(v) if !matches!(v, Value::Undef) => with_host(|h| h.str_of(v)),
        _ => existing_flags.unwrap_or_default(),
    };
    // An empty source compiles as the JS canonical `(?:)`.
    let src = if source.is_empty() {
        "(?:)".to_string()
    } else {
        source
    };
    crate::regexp::build_regexp(&src, &flags)
}

/// `BigInt.asIntN(bits, x)` / `BigInt.asUintN(bits, x)`: wrap `x` to a `bits`-wide
/// two's-complement (signed) or unsigned integer.
fn bigint_as_n(unsigned: bool, args: &[Value]) -> Result<Value, String> {
    use num_bigint::BigInt;
    use num_traits::Signed;
    let bits = with_host(|h| h.to_number(&arg0(args))) as i64;
    if bits < 0 {
        return Err("RangeError: Invalid value: not (convertible to) a safe integer".into());
    }
    let x = match with_host(|h| h.as_bigint(&args.get(1).cloned().unwrap_or(Value::Undef))) {
        Some(b) => b,
        None => return Err(host::type_error("Cannot convert to a BigInt")),
    };
    let bits = bits as u32;
    if bits == 0 {
        return Ok(with_host(|h| h.new_bigint(BigInt::from(0))));
    }
    let modulus = BigInt::from(1) << bits; // 2^bits
                                           // Reduce into [0, 2^bits); for the signed form fold the top half negative.
    let mut r = &x % &modulus;
    if r.is_negative() {
        r += &modulus;
    }
    if !unsigned {
        let half = BigInt::from(1) << (bits - 1);
        if r >= half {
            r -= &modulus;
        }
    }
    Ok(with_host(|h| h.new_bigint(r)))
}

/// `String.raw(callSite, ...subs)`: concatenate the raw quasis (`callSite.raw`)
/// interleaved with the substitutions.
fn string_raw(args: &[Value]) -> Result<Value, String> {
    let call_site = arg0(args);
    let raw = get_property(&call_site, "raw")?;
    let raws = with_host(|h| h.iter_vec(&raw)).unwrap_or_default();
    let mut out = String::new();
    for (i, r) in raws.iter().enumerate() {
        out.push_str(&with_host(|h| h.str_of(r)));
        if i + 1 < raws.len() {
            if let Some(sub) = args.get(i + 1) {
                out.push_str(&with_host(|h| h.str_of(sub)));
            }
        }
    }
    Ok(with_host(|h| h.new_str(out)))
}

/// `Object(x)`: box/pass-through — for our model, non-object args just return a
/// fresh object; objects pass through.
fn object_call(args: Vec<Value>) -> Value {
    let a = arg0(&args);
    if matches!(
        with_host(|h| h.get(&a).cloned()),
        Some(JsObj::Object(_)) | Some(JsObj::Array(_))
    ) {
        a
    } else {
        with_host(|h| h.new_object(IndexMap::new()))
    }
}

/// Construct via `new` for the builtin constructors.
pub fn construct_builtin(name: &str, args: Vec<Value>) -> Result<Value, String> {
    // Native stdlib constructors (`new URL(...)`, `new EventEmitter()`, `new Buffer(...)`).
    if let Some(r) = crate::stdlib::construct(name, &args) {
        return r;
    }
    match name {
        "Array" => {
            // `new Array(n)` -> length-n array; `new Array(a, b)` -> [a, b].
            // A single NUMBER argument is a length and is validated as one
            // (23.1.1.1 step 6), so `new Array(-1)` / `new Array(1.5)` /
            // `new Array(2**32)` are all `RangeError: Invalid array length` on
            // node v26.7.0; only a non-number single argument is an element.
            if args.len() == 1 {
                if let Value::Float(_) | Value::Int(_) = args[0] {
                    let n = host::to_array_length(&args[0])?;
                    // Every element of `new Array(n)` is a HOLE, not a stored
                    // `undefined`: `Object.keys(Array(3))` is `[]`.
                    return Ok(with_host(|h| {
                        let a = h.new_array(vec![Value::Undef; n]);
                        h.mark_hole_range(&a, 0..n);
                        a
                    }));
                }
            }
            Ok(with_host(|h| h.new_array(args)))
        }
        "Object" => Ok(object_call(args)),
        "Map" | "WeakMap" => {
            let weak = name == "WeakMap";
            let m = with_host(|h| {
                h.alloc(JsObj::Map {
                    entries: indexmap::IndexMap::new(),
                    weak,
                })
            });
            if let Some(init) = args
                .first()
                .filter(|a| !matches!(a, Value::Undef) && !with_host(|h| h.is_null(a)))
            {
                let pairs = host::iter_all(init)?;
                for p in pairs {
                    let kv = host::iter_all(&p)?;
                    let k = kv.first().cloned().unwrap_or(Value::Undef);
                    let v = kv.get(1).cloned().unwrap_or(Value::Undef);
                    map_method(&m, "set", vec![k, v])?;
                }
            }
            Ok(m)
        }
        "Set" | "WeakSet" => {
            let weak = name == "WeakSet";
            let s = with_host(|h| {
                h.alloc(JsObj::Set {
                    entries: indexmap::IndexMap::new(),
                    weak,
                })
            });
            if let Some(init) = args
                .first()
                .filter(|a| !matches!(a, Value::Undef) && !with_host(|h| h.is_null(a)))
            {
                let vals = host::iter_all(init)?;
                for v in vals {
                    set_method(&s, "add", vec![v])?;
                }
            }
            Ok(s)
        }
        "Promise" => new_promise(arg0(&args)),
        "Proxy" => crate::proxy::create(&args),
        // `new Function(p…, body)` — the same `CreateDynamicFunction` the plain
        // call form runs (20.2.1.1). `depd`'s `wrapfunction` builds its
        // deprecation wrapper this way, so `require('body-parser')` — and with it
        // `require('express')` — dies at load without it.
        "Function" => function_ctor(&args),
        "RegExp" => regexp_ctor(&args),
        "BigInt" => Err(host::type_error("BigInt is not a constructor")),
        "Error" => Ok(make_error(name, &args)),
        n if host::ERROR_NAMES.contains(&n) => Ok(make_error(name, &args)),
        _ => Err(host::type_error(&format!("{name} is not a constructor"))),
    }
}

fn make_error(name: &str, args: &[Value]) -> Value {
    // `new AggregateError(errors, message)` takes the causes FIRST; every other
    // error constructor takes the message first.
    let agg = name == "AggregateError";
    let (errors, args) = if agg {
        (
            Some(args.first().cloned().unwrap_or(Value::Undef)),
            args.get(1..).unwrap_or(&[]),
        )
    } else {
        (None, args)
    };
    with_host(|h| {
        h.ensure_error_protos();
        let mut props: IndexMap<String, Value> = IndexMap::new();
        let msg = args
            .first()
            .filter(|a| !matches!(a, Value::Undef))
            .map(|a| h.str_of(a));
        if let Some(m) = &msg {
            let mv = h.new_str(m.clone());
            props.insert("message".into(), mv);
        }
        // `.stack` is engine-specific; a simple `Name: message` header line
        // suffices for parity (the fuzzer never prints raw stacks).
        let frames = h.stack_frames();
        let stack = match &msg {
            Some(m) if !m.is_empty() => format!("{name}: {m}{frames}"),
            _ => format!("{name}{frames}"),
        };
        let sv = h.new_str(stack);
        props.insert("stack".into(), sv);
        if let Some(errs) = errors {
            // Materialize the iterable into the own `errors` array property.
            let items = h.iter_vec(&errs).unwrap_or_default();
            let arr = h.new_array(items);
            props.insert("errors".into(), arr);
        }
        // `new Error(msg, { cause })` (ES2022): installed only when the options
        // bag actually has a `cause` key, so `new Error(m, {})` leaves none.
        let opts = args.get(1);
        if let Some(cause) = opts.and_then(|o| match h.get(o) {
            Some(JsObj::Object(p)) => p.get("cause").cloned(),
            _ => None,
        }) {
            props.insert("cause".into(), cause);
        }
        let e = h.new_object(props);
        if let Some(p) = host::error_proto_of(h, name) {
            h.set_proto(&e, p);
        }
        // Every own slot an error constructor installs is non-enumerable in V8,
        // which is why `Object.keys(err)` is `[]` and `JSON.stringify(err)` is
        // `{}` — properties a *script* later assigns stay enumerable.
        for k in ["message", "stack", "errors", "cause"] {
            h.hide_prop(&e, k);
        }
        e
    })
}

fn print_line(args: &[Value], stderr: bool) {
    // Node's console.log(...args) === util.format(...args): printf-style
    // substitution when the first arg is a format string, else inspect-and-join.
    let line: String = crate::stdlib::util::format(args);
    with_host(|h| h.write_out(&format!("{line}\n"), stderr));
}

fn arg0(args: &[Value]) -> Value {
    args.first().cloned().unwrap_or(Value::Undef)
}
fn arg_num(args: &[Value], i: usize) -> f64 {
    with_host(|h| h.to_number(&args.get(i).cloned().unwrap_or(Value::Undef)))
}

fn is_integer(v: Value) -> bool {
    match v {
        Value::Int(_) => true,
        Value::Float(f) => f.is_finite() && f.fract() == 0.0,
        _ => false,
    }
}
fn is_safe_integer(v: Value) -> bool {
    match v {
        Value::Float(f) => f.is_finite() && f.fract() == 0.0 && f.abs() <= 9007199254740991.0,
        Value::Int(_) => true,
        _ => false,
    }
}

/// `encodeURI`/`encodeURIComponent`: percent-encode `s`'s UTF-8 bytes, leaving
/// the unreserved set unescaped. `encodeURI` additionally preserves the reserved
/// URI characters (`;,/?:@&=+$#`) that delimit a URI's structure.
fn uri_encode(s: &str, uri: bool) -> Result<Value, String> {
    // Always-unescaped (`encodeURIComponent`'s unreserved set), per the spec.
    const UNRESERVED: &[u8] =
        b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789-_.!~*'()";
    // Reserved characters `encodeURI` leaves intact on top of the unreserved set.
    const RESERVED: &[u8] = b";,/?:@&=+$#";
    let mut out = String::with_capacity(s.len());
    for &b in s.as_bytes() {
        if UNRESERVED.contains(&b) || (uri && RESERVED.contains(&b)) {
            out.push(b as char);
        } else {
            out.push('%');
            out.push(
                char::from_digit((b >> 4) as u32, 16)
                    .unwrap()
                    .to_ascii_uppercase(),
            );
            out.push(
                char::from_digit((b & 0xf) as u32, 16)
                    .unwrap()
                    .to_ascii_uppercase(),
            );
        }
    }
    Ok(with_host(|h| h.new_str(out)))
}

/// `decodeURI`/`decodeURIComponent`: reverse `%XX` escapes back to UTF-8 text.
/// For `decodeURI`, escapes of the reserved delimiters are left as-is (the spec's
/// asymmetry with `encodeURI`). Throws `URIError` on a malformed escape.
fn uri_decode(s: &str, uri: bool) -> Result<Value, String> {
    const RESERVED: &[u8] = b";,/?:@&=+$#";
    let bytes = s.as_bytes();
    let mut out: Vec<u8> = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b'%' {
            if i + 2 >= bytes.len() {
                return Err("URIError: URI malformed".into());
            }
            let hi = (bytes[i + 1] as char).to_digit(16);
            let lo = (bytes[i + 2] as char).to_digit(16);
            match (hi, lo) {
                (Some(h), Some(l)) => {
                    let byte = (h * 16 + l) as u8;
                    // decodeURI keeps reserved-delimiter escapes literal.
                    if uri && RESERVED.contains(&byte) {
                        out.extend_from_slice(&bytes[i..i + 3]);
                    } else {
                        out.push(byte);
                    }
                    i += 3;
                }
                _ => return Err("URIError: URI malformed".into()),
            }
        } else {
            out.push(bytes[i]);
            i += 1;
        }
    }
    match String::from_utf8(out) {
        Ok(decoded) => Ok(with_host(|h| h.new_str(decoded))),
        Err(_) => Err("URIError: URI malformed".into()),
    }
}

/// `escape` (Annex B.2.1.1) — the pre-`encodeURIComponent` legacy encoder, still
/// present in every engine and still reached by old libraries (jQuery's cookie
/// plugin, `querystring`-era code). It works on UTF-16 CODE UNITS, not UTF-8
/// bytes, which is what separates it from `encodeURIComponent`: a unit below
/// `0x100` becomes `%XX`, anything above becomes `%uXXXX`, so an astral
/// character yields the two escapes of its surrogate pair
/// (`escape("\u{1D4B3}")` is `"%uD835%uDCB3"` on node v26.7.0).
///
/// The unescaped set is frozen by the spec and is NOT the URI unreserved set —
/// it keeps `@*_+-./` and drops `!~'()`.
fn legacy_escape(s: &str) -> Result<Value, String> {
    const KEEP: &[u8] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789@*_+-./";
    let mut out = String::with_capacity(s.len());
    for u in s.encode_utf16() {
        if u < 0x100 {
            if KEEP.contains(&(u as u8)) {
                out.push(u as u8 as char);
            } else {
                out.push_str(&format!("%{u:02X}"));
            }
        } else {
            out.push_str(&format!("%u{u:04X}"));
        }
    }
    Ok(with_host(|h| h.new_str(out)))
}

/// `unescape` (Annex B.2.1.2) — the inverse of [`legacy_escape`]. Unlike
/// `decodeURIComponent` it never throws: a `%` that does not begin a well-formed
/// `%XX` or `%uXXXX` escape is passed through literally
/// (`unescape("%u0041%42%zz%2")` is `"AB%zz%2"` on node v26.7.0).
///
/// Decoding is done in code-unit space and re-joined at the end so a
/// `%uD835%uDCB3` pair recomposes into the one astral character it came from.
fn legacy_unescape(s: &str) -> Result<Value, String> {
    let b = s.as_bytes();
    let hex = |i: usize, n: usize| -> Option<u16> {
        if i + n > b.len() {
            return None;
        }
        let mut v: u16 = 0;
        for &c in &b[i..i + n] {
            v = v.checked_mul(16)? + (c as char).to_digit(16)? as u16;
        }
        Some(v)
    };
    let units: Vec<u16> = s.encode_utf16().collect();
    let mut out: Vec<u16> = Vec::with_capacity(units.len());
    let mut i = 0;
    while i < b.len() {
        // Escapes are pure ASCII, so a byte index is a unit index up to here —
        // but the tail may not be, so non-`%` bytes are re-decoded as chars.
        if b[i] == b'%' {
            if let Some(u) = hex(i + 1, 2) {
                out.push(u);
                i += 3;
                continue;
            }
            if b.get(i + 1) == Some(&b'u') {
                if let Some(u) = hex(i + 2, 4) {
                    out.push(u);
                    i += 6;
                    continue;
                }
            }
        }
        let c = s[i..].chars().next().unwrap_or('%');
        let mut buf = [0u16; 2];
        out.extend_from_slice(c.encode_utf16(&mut buf));
        i += c.len_utf8();
    }
    Ok(with_host(|h| {
        h.new_str(crate::utf16::to_string_lossy(&out))
    }))
}

fn parse_int(args: &[Value]) -> f64 {
    let s = with_host(|h| h.str_of(&arg0(args)));
    // 19.2.5 step 8: an EXPLICIT radix outside 2..=36 is `NaN`, it does not fall
    // back to auto-detection. The old `.filter()` silently discarded a bad radix,
    // so `parseInt("10", 37)` answered 10 where every engine says NaN.
    let radix_arg = args
        .get(1)
        .map(|r| with_host(|h| host::to_int32(h.to_number(r))));
    let radix = match radix_arg {
        Some(0) | None => None,
        Some(r) if (2..=36).contains(&r) => Some(r as u32),
        Some(_) => return f64::NAN,
    };
    let t = crate::utf16::js_trim_start(&s);
    let (neg, digits) = match t.strip_prefix('-') {
        Some(rest) => (true, rest),
        None => (false, t.strip_prefix('+').unwrap_or(t)),
    };
    let (radix, digits) = match radix {
        Some(16) => (
            16u32,
            digits
                .strip_prefix("0x")
                .or_else(|| digits.strip_prefix("0X"))
                .unwrap_or(digits),
        ),
        Some(r) => (r, digits),
        None => {
            if let Some(hex) = digits
                .strip_prefix("0x")
                .or_else(|| digits.strip_prefix("0X"))
            {
                (16, hex)
            } else {
                (10, digits)
            }
        }
    };
    let valid: String = digits.chars().take_while(|c| c.is_digit(radix)).collect();
    if valid.is_empty() {
        return f64::NAN;
    }
    // Accumulate in `f64`, not `i64`. `i64::from_str_radix` OVERFLOWS past ~19
    // digits and the error was mapped to `NaN`, so
    // `parseInt("999999999999999999999999")` was NaN instead of 1e+24. The spec
    // asks for the mathematical value rounded to a Number, which is what
    // repeated multiply-accumulate in `f64` produces.
    let n = if radix == 10 {
        // Rust's decimal float parser is correctly rounded; digit-by-digit
        // multiply-accumulate is not, and drifted a ULP on long inputs
        // (`parseInt("999999999999999999999999")` came out
        // 1.0000000000000003e+24 rather than 1e+24).
        valid.parse::<f64>().unwrap_or(f64::NAN)
    } else {
        let mut n = 0.0f64;
        for c in valid.chars() {
            n = n * radix as f64 + c.to_digit(radix).unwrap_or(0) as f64;
        }
        n
    };
    if neg {
        -n
    } else {
        n
    }
}

fn parse_float(args: &[Value]) -> f64 {
    let s = with_host(|h| h.str_of(&arg0(args)));
    let t = crate::utf16::js_trim_start(&s);
    // `Infinity` / `+Infinity` / `-Infinity` are valid parseFloat prefixes.
    let inf_body = t
        .strip_prefix('+')
        .or_else(|| t.strip_prefix('-'))
        .unwrap_or(t);
    if inf_body.starts_with("Infinity") {
        return if t.starts_with('-') {
            f64::NEG_INFINITY
        } else {
            f64::INFINITY
        };
    }
    // The LONGEST prefix that is itself a complete `StrDecimalLiteral`, which is
    // not the same as the longest run of characters that could appear in one:
    // `"1e"` and `"1e+"` are `1` in every engine, because the exponent part is
    // only valid once a digit follows `e`. Tracking `end` at every character
    // accepted the dangling `e`, `parse::<f64>` then failed, and the whole call
    // came back NaN.
    let mut end = 0;
    let bytes = t.as_bytes();
    let mut seen_dot = false;
    let mut seen_e = false;
    let mut digits_before_dot = false;
    for (i, &c) in bytes.iter().enumerate() {
        match c {
            b'0'..=b'9' => {
                if !seen_dot && !seen_e {
                    digits_before_dot = true;
                }
                end = i + 1;
            }
            // A sign is only meaningful leading, or straight after the exponent
            // marker; it never completes a literal on its own.
            b'+' | b'-' if i == 0 || bytes[i - 1] == b'e' || bytes[i - 1] == b'E' => {}
            // `1.` is a complete literal; a bare `.` is not.
            b'.' if !seen_dot && !seen_e => {
                seen_dot = true;
                if digits_before_dot {
                    end = i + 1;
                }
            }
            b'e' | b'E' if !seen_e && end > 0 => seen_e = true,
            _ => break,
        }
    }
    if end == 0 {
        return f64::NAN;
    }
    t[..end].parse::<f64>().unwrap_or(f64::NAN)
}

/// ECMA-262 `Number::exponentiate` (6.1.6.1.3), backing both `Math.pow` and the
/// `**` operator. Three clauses differ from IEEE-754 `pow`, which is what Rust's
/// `powf` implements: a NaN exponent is NaN even for base 1, a NaN base is NaN
/// for any non-zero exponent, and `|base| == 1` with an infinite exponent is NaN
/// rather than 1.
pub(crate) fn js_pow(base: f64, exp: f64) -> f64 {
    if exp == 0.0 {
        return 1.0;
    }
    if base.is_nan() || exp.is_nan() {
        return f64::NAN;
    }
    if base.abs() == 1.0 && exp.is_infinite() {
        return f64::NAN;
    }
    base.powf(exp)
}

fn math_fn(fname: &str, args: &[Value]) -> Result<Value, String> {
    // Every `Math` function coerces its arguments with `ToNumber`, and `ToNumber`
    // of a BigInt is a TypeError (7.1.4 step 2) — the whole point of BigInt being
    // a separate numeric type. `arg_num` reads a BigInt's magnitude instead, so
    // `Math.max(1n)` quietly answered 1 where V8 throws. `Math.random` is the one
    // exception: it never reads an argument, so `Math.random(1n)` is fine.
    if fname != "random"
        && args
            .iter()
            .any(|a| with_host(|h| matches!(h.get(a), Some(JsObj::BigInt(_)))))
    {
        return Err(host::type_error(
            "Cannot convert a BigInt value to a number",
        ));
    }
    let x = arg_num(args, 0);
    let r = match fname {
        "floor" => x.floor(),
        "ceil" => x.ceil(),
        // ECMA-262 `Math.round` (21.3.2.28) transcribed clause by clause. The
        // obvious `(x + 0.5).floor()` is NOT this function: the addition rounds
        // before the floor sees it, so it answers 1 for the largest double below
        // 0.5 (`Math.round(0.49999999999999994)` is 0 in every engine) and it
        // perturbs integers above 2^52, where `x + 0.5` is no longer
        // representable (`Math.round(4503599627370497)` must be the input).
        // Splitting the zero-band cases out first also carries the signed zero
        // the spec asks for without a post-hoc patch.
        "round" => {
            if !x.is_finite() || x == 0.0 {
                x
            } else if x > 0.0 && x < 0.5 {
                0.0
            } else if (-0.5..0.0).contains(&x) {
                -0.0
            } else {
                // |x| >= 0.5, so `floor` and the subtraction are both exact
                // (every double >= 2^52 is already an integer and yields 0 here).
                let f = x.floor();
                if x - f >= 0.5 {
                    f + 1.0
                } else {
                    f
                }
            }
        }
        "trunc" => x.trunc(),
        "abs" => x.abs(),
        "sign" => {
            if x.is_nan() {
                f64::NAN
            } else if x > 0.0 {
                1.0
            } else if x < 0.0 {
                -1.0
            } else {
                x
            }
        }
        "sqrt" => x.sqrt(),
        "cbrt" => x.cbrt(),
        "exp" => x.exp(),
        "log" => x.ln(),
        "log2" => x.log2(),
        "log10" => x.log10(),
        "sin" => x.sin(),
        "cos" => x.cos(),
        "tan" => x.tan(),
        "asin" => x.asin(),
        "acos" => x.acos(),
        "atan" => x.atan(),
        "atan2" => x.atan2(arg_num(args, 1)),
        // Rust `powf` is IEEE-754 `pow`, which is NOT JS `**`/`Math.pow`: IEEE
        // makes `pow(x, ±0)` and `pow(±1, y)` return 1 unconditionally, so
        // `(-1) ** Infinity` and `1 ** NaN` come back 1 where the spec
        // (6.1.6.1.3 Number::exponentiate) says NaN. Only the exponent-is-zero
        // clause is shared.
        "pow" => js_pow(x, arg_num(args, 1)),
        // Hyperbolics and the two precision-preserving log/exp forms.
        "sinh" => x.sinh(),
        "cosh" => x.cosh(),
        "tanh" => x.tanh(),
        "asinh" => x.asinh(),
        "acosh" => x.acosh(),
        "atanh" => x.atanh(),
        "log1p" => x.ln_1p(),
        "expm1" => x.exp_m1(),
        // C-style 32-bit integer multiply: ToInt32 both operands, multiply with
        // wraparound, reinterpret as a signed 32-bit result.
        "imul" => (host::to_int32(x).wrapping_mul(host::to_int32(arg_num(args, 1)))) as f64,
        "hypot" => {
            // Scale by the largest magnitude before squaring — this avoids the
            // last-ULP error of the naive `sqrt(Σ xᵢ²)` and matches V8's result.
            let xs: Vec<f64> = args.iter().map(|a| with_host(|h| h.to_number(a))).collect();
            let mut max = 0.0f64;
            for x in &xs {
                if x.abs() > max {
                    max = x.abs();
                }
            }
            if xs.iter().any(|x| x.is_infinite()) {
                f64::INFINITY
            } else if max == 0.0 || !max.is_finite() {
                max
            } else {
                let s: f64 = xs.iter().map(|x| (x / max) * (x / max)).sum();
                max * s.sqrt()
            }
        }
        "random" => pseudo_random(),
        "max" => {
            if args.is_empty() {
                f64::NEG_INFINITY
            } else {
                let mut m = f64::NEG_INFINITY;
                for a in args {
                    let n = with_host(|h| h.to_number(a));
                    if n.is_nan() {
                        return Ok(Value::Float(f64::NAN));
                    }
                    // `>` cannot separate the zeroes (`0.0 > -0.0` is false), but
                    // the spec ranks +0 above -0, so `Math.max(-0, 0)` is +0 and
                    // must not keep the -0 the first iteration installed.
                    if n > m || (n == m && n == 0.0 && n.is_sign_positive()) {
                        m = n;
                    }
                }
                m
            }
        }
        "min" => {
            if args.is_empty() {
                f64::INFINITY
            } else {
                let mut m = f64::INFINITY;
                for a in args {
                    let n = with_host(|h| h.to_number(a));
                    if n.is_nan() {
                        return Ok(Value::Float(f64::NAN));
                    }
                    // Mirror of `max`: -0 ranks below +0 even though `<` says
                    // they are equal, so `Math.min(0, -0)` is -0.
                    if n < m || (n == m && n == 0.0 && n.is_sign_negative()) {
                        m = n;
                    }
                }
                m
            }
        }
        // Count leading zero bits of ToUint32(x) (Math.clz32(1) === 31).
        "clz32" => {
            let u = if x.is_finite() {
                x.trunc().rem_euclid(4294967296.0) as u32
            } else {
                0
            };
            u.leading_zeros() as f64
        }
        // Round to the nearest single-precision float.
        "fround" => (x as f32) as f64,
        _ => return Err(host::type_error(&format!("Math.{fname} is not a function"))),
    };
    Ok(Value::Float(r))
}

/// A small deterministic PRNG for `Math.random` (output is non-reproducible vs
/// Node by nature; kept simple).
fn pseudo_random() -> f64 {
    use std::cell::Cell;
    thread_local!(static SEED: Cell<u64> = const { Cell::new(0x2545F4914F6CDD1D) });
    SEED.with(|s| {
        let mut x = s.get();
        x ^= x << 13;
        x ^= x >> 7;
        x ^= x << 17;
        s.set(x);
        (x >> 11) as f64 / (1u64 << 53) as f64
    })
}

// ── Object.* ──────────────────────────────────────────────────────────────────

fn object_keys(args: Vec<Value>, mode: u8) -> Result<Value, String> {
    let v = arg0(&args);
    require_object_coercible(&v)?;
    // A Proxy answers from its `ownKeys` trap. `getOwnPropertyNames` (mode 3)
    // reports every own STRING key the trap named; the enumerating modes
    // additionally filter by each key's `[[GetOwnProperty]]`, so both traps run.
    if with_host(|h| h.kind_of(&v)) == Some(ObjKind::Proxy) {
        if mode == 3 {
            let keys = crate::proxy::own_keys(&v)?.unwrap_or_default();
            return Ok(with_host(|h| {
                let out: Vec<Value> = keys
                    .into_iter()
                    .filter(|k| !host::is_symbol_key(k))
                    .map(|k| h.new_str(k))
                    .collect();
                h.new_array(out)
            }));
        }
        let entries = crate::proxy::own_enum_entries(&v)?;
        return Ok(with_host(|h| {
            let out: Vec<Value> = entries
                .into_iter()
                .map(|(k, val)| match mode {
                    0 => h.new_str(k),
                    1 => val,
                    _ => {
                        let ks = h.new_str(k);
                        h.new_array(vec![ks, val])
                    }
                })
                .collect();
            h.new_array(out)
        }));
    }
    // A builtin prototype namespace that exposes enumerable methods for copying
    // (`Object.getOwnPropertyNames(EventEmitter.prototype)` — express's mixin).
    if let Some(JsObj::Builtin(ns)) = with_host(|h| h.get(&v).cloned()) {
        if let Some(names) = builtin_proto_method_names(&ns) {
            return Ok(with_host(|h| {
                let out: Vec<Value> = names
                    .iter()
                    .map(|name| match mode {
                        1 => h.alloc(JsObj::Builtin(format!(
                            "@proto:{}:{name}",
                            ns.trim_end_matches(".prototype")
                        ))),
                        2 => {
                            let ks = h.new_str(*name);
                            let val = h.alloc(JsObj::Builtin(format!(
                                "@proto:{}:{name}",
                                ns.trim_end_matches(".prototype")
                            )));
                            h.new_array(vec![ks, val])
                        }
                        _ => h.new_str(*name),
                    })
                    .collect();
                h.new_array(out)
            }));
        }
        // A stdlib namespace (`Buffer`, `require('buffer')`): its own enumerable
        // keys are the members node-js implements, each resolved to the same
        // first-class value a property read would give.
        let mut names = crate::stdlib::namespace_keys(&ns);
        // A core namespace (`Reflect`, `Math`, `JSON`) has no stdlib key list —
        // its members live in the builtin dispatch table. They are
        // non-enumerable in V8, so they surface only under
        // `getOwnPropertyNames`/`Reflect.ownKeys` (mode 3), never `Object.keys`.
        if names.is_empty() && mode == 3 {
            let prefix = format!("{ns}.");
            names = NS_METHODS
                .iter()
                .filter_map(|q| q.strip_prefix(&prefix))
                .map(|m| m.to_string())
                .collect();
        }
        if !names.is_empty() {
            let entries: Vec<(String, Value)> = names
                .into_iter()
                .map(|k| {
                    let val = namespace_property(&ns, &k);
                    (k, val)
                })
                .collect();
            return Ok(with_host(|h| {
                let out: Vec<Value> = entries
                    .into_iter()
                    .map(|(k, val)| match mode {
                        1 => val,
                        2 => {
                            let ks = h.new_str(k);
                            h.new_array(vec![ks, val])
                        }
                        _ => h.new_str(k),
                    })
                    .collect();
                h.new_array(out)
            }));
        }
    }
    // mode 3 (`getOwnPropertyNames`) reports every own string key including the
    // non-enumerable ones, plus the exotic `length` an array carries.
    let entries: Vec<(String, Value)> = with_host(|h| {
        if mode == 3 {
            // An array's exotic `length` is already placed (after the indices,
            // before the ordinary string keys) by `own_key_names`.
            return h
                .own_key_names(&v, false)
                .into_iter()
                .map(|k| (k, Value::Undef))
                .collect();
        }
        Vec::new()
    });
    let entries = if mode == 3 {
        entries
    } else {
        host::own_enum_entries_deep(&v)
    };
    Ok(with_host(|h| {
        let out: Vec<Value> = entries
            .into_iter()
            .map(|(k, val)| match mode {
                0 | 3 => h.new_str(k),
                1 => val,
                _ => {
                    let ks = h.new_str(k);
                    h.new_array(vec![ks, val])
                }
            })
            .collect();
        h.new_array(out)
    }))
}

fn object_assign(args: Vec<Value>) -> Result<Value, String> {
    let target = arg0(&args);
    // 20.1.2.1 step 1 is `ToObject(target)`, so a nullish TARGET throws while a
    // nullish SOURCE is skipped (`Object.assign({}, null)` is `{}`).
    require_object_coercible(&target)?;
    for src in args.iter().skip(1) {
        // `Object.assign` copies own *enumerable* properties, running any getter
        // — symbol-keyed ones included (7.3.25).
        let entries = host::own_enum_entries_deep(src);
        let syms = with_host(|h| h.own_symbol_entries(src));
        // A plain object target is filled in place (one borrow, then a single
        // re-canonicalization of the integer-index keys).
        let filled = with_host(|h| {
            if let Some(JsObj::Object(p)) = h.get_mut(&target) {
                for (k, v) in entries.iter().cloned().chain(syms.iter().cloned()) {
                    p.insert(k, v);
                }
                host::canonicalize_own_keys(p);
                return true;
            }
            false
        });
        // Any OTHER target — an array being the common one — goes through the
        // ordinary Set path. The in-place branch above matched `JsObj::Object`
        // only, so `Object.assign([1,2], {extra:9})` silently copied NOTHING and
        // returned the untouched array: no error, just a missing property. The
        // Set path is what an `arr.extra = 9` assignment already used, so index
        // and non-index keys land where they do for a direct write.
        if !filled {
            for (k, v) in entries.into_iter().chain(syms) {
                set_property(&target, &k, v)?;
            }
        }
    }
    Ok(target)
}

fn object_from_entries(args: Vec<Value>) -> Result<Value, String> {
    let pairs = with_host(|h| h.iter_vec(&arg0(&args))).unwrap_or_default();
    let mut props: IndexMap<String, Value> = IndexMap::new();
    for p in pairs {
        let kv = with_host(|h| h.iter_vec(&p)).unwrap_or_default();
        let key = with_host(|h| h.str_of(&kv.first().cloned().unwrap_or(Value::Undef)));
        let val = kv.get(1).cloned().unwrap_or(Value::Undef);
        props.insert(key, val);
    }
    Ok(with_host(|h| h.new_object(props)))
}

/// `Object.groupBy(items, cb)` — group the iterable `items` into a null-prototype
/// object. Keys are `ToPropertyKey(cb(item, index))`; values are arrays of the
/// members mapped to that key, in first-seen key order.
fn object_group_by(args: Vec<Value>) -> Result<Value, String> {
    let items = host::iter_all(&arg0(&args))?;
    let cb = args.get(1).cloned().unwrap_or(Value::Undef);
    let mut groups: IndexMap<String, Vec<Value>> = IndexMap::new();
    for (i, item) in items.into_iter().enumerate() {
        let key_v = host::invoke(&cb, vec![item.clone(), Value::Float(i as f64)], None)?;
        let key = with_host(|h| h.property_key(&key_v));
        groups.entry(key).or_default().push(item);
    }
    let props: IndexMap<String, Value> = with_host(|h| {
        groups
            .into_iter()
            .map(|(k, v)| (k, h.new_array(v)))
            .collect()
    });
    let obj = with_host(|h| h.new_object(props));
    // A null-prototype object (as Node returns), so it has no inherited members.
    with_host(|h| {
        let nv = h.null();
        h.set_proto(&obj, nv);
    });
    Ok(obj)
}

/// `Map.groupBy(items, cb)` — like `Object.groupBy` but returns a `Map` keyed by
/// the raw `cb(item, index)` value under SameValueZero (so object/any keys work).
fn map_group_by(args: Vec<Value>) -> Result<Value, String> {
    let items = host::iter_all(&arg0(&args))?;
    let cb = args.get(1).cloned().unwrap_or(Value::Undef);
    let m = with_host(|h| {
        h.alloc(JsObj::Map {
            entries: IndexMap::new(),
            weak: false,
        })
    });
    for (i, item) in items.into_iter().enumerate() {
        let key_v = host::invoke(&cb, vec![item.clone(), Value::Float(i as f64)], None)?;
        let existing = map_method(&m, "get", vec![key_v.clone()])?;
        if matches!(existing, Value::Undef) {
            let arr = with_host(|h| h.new_array(vec![item]));
            map_method(&m, "set", vec![key_v, arr])?;
        } else {
            with_host(|h| {
                if let Some(JsObj::Array(a)) = h.get_mut(&existing) {
                    a.push(item);
                }
            });
        }
    }
    Ok(m)
}

/// `Array.fromAsync(items[, mapFn])` — a Promise for an array, awaiting each
/// element and each `mapFn` result.
///
/// Written in JavaScript and compiled once, because the operation IS an async
/// function: a Rust builtin runs outside any coroutine and has no way to await,
/// so draining a promise from there would mean running the microtask queue by
/// hand. Delegating to the engine's own `async`/`for await` keeps the
/// suspension semantics — and the ordering they imply — exactly the language's.
///
/// The source may be an async iterable, a sync iterable, a bare iterator, or an
/// array-like. Everything iterable goes through `for await`, which awaits a sync
/// source's elements individually — that is what makes
/// `Array.fromAsync([1, Promise.resolve(2)])` answer `[1, 2]`. A bare `.next` is
/// accepted because an async generator object does not expose
/// `Symbol.asyncIterator` on this frontend.
fn array_from_async(args: Vec<Value>) -> Result<Value, String> {
    thread_local! {
        static IMPL: std::cell::RefCell<Option<Value>> = const { std::cell::RefCell::new(None) };
    }
    const SRC: &str = "(async function (items, mapFn, thisArg) {\n\
        const out = []; let i = 0;\n\
        const step = async (v) => { const a = await v; out.push(mapFn ? await mapFn.call(thisArg, a, i) : a); i++; };\n\
        const iterable = items != null && (typeof items[Symbol.asyncIterator] === 'function'\n\
            || typeof items[Symbol.iterator] === 'function' || typeof items.next === 'function');\n\
        if (iterable) {\n\
            for await (const v of items) { out.push(mapFn ? await mapFn.call(thisArg, v, i) : v); i++; }\n\
            return out;\n\
        }\n\
        const len = items == null ? 0 : (Math.trunc(Number(items.length)) || 0);\n\
        while (i < len) { await step(items[i]); }\n\
        return out;\n\
    })";
    let f = IMPL.with(|c| c.borrow().clone());
    let f = match f {
        Some(f) => f,
        None => {
            let f = crate::eval_in_global_scope(SRC)?;
            IMPL.with(|c| *c.borrow_mut() = Some(f.clone()));
            f
        }
    };
    host::invoke(&f, args, None)
}

fn array_from(args: Vec<Value>) -> Result<Value, String> {
    // `Array.from` accepts generators and user iterables, plus array-likes with a
    // numeric `.length`.
    let src = arg0(&args);
    let items = match host::iter_all(&src) {
        Ok(v) => v,
        Err(_) => array_like_items(&src),
    };
    if let Some(cb) = args.get(1).cloned() {
        let mut out = Vec::with_capacity(items.len());
        for (i, it) in items.into_iter().enumerate() {
            out.push(host::invoke(&cb, vec![it, Value::Float(i as f64)], None)?);
        }
        return Ok(with_host(|h| h.new_array(out)));
    }
    Ok(with_host(|h| h.new_array(items)))
}

/// Items of an array-like `{ length, 0, 1, … }` object (for `Array.from`).
fn array_like_items(src: &Value) -> Vec<Value> {
    let len = get_property(src, "length")
        .ok()
        .map(|l| with_host(|h| h.to_number(&l)))
        .unwrap_or(0.0);
    if !len.is_finite() || len <= 0.0 {
        return Vec::new();
    }
    (0..len as usize)
        .map(|i| get_property(src, &i.to_string()).unwrap_or(Value::Undef))
        .collect()
}

// ── JSON ──────────────────────────────────────────────────────────────────────

fn json_stringify(args: Vec<Value>) -> Result<Value, String> {
    // A CALLABLE second argument is the replacer function, and it is checked
    // before the array form (`IsCallable` precedes `IsArray` in the spec), so a
    // callable never also reaches the key-filter path below.
    let replacer = args
        .get(1)
        .filter(|r| with_host(|h| host::is_callable(h, r)))
        .cloned();
    // `toJSON` and the replacer run BEFORE serialization and are user code, so
    // the tree is rewritten first — outside the host borrow `json_str` holds,
    // and before the BigInt walk, which has no cycle guard of its own.
    //
    // The top-level value is a property of a synthetic wrapper `{ "": value }`
    // under key `""`, which is exactly the holder the replacer receives as
    // `this` on its first call.
    let root = arg0(&args);
    let wrapper = with_host(|h| {
        let mut m: IndexMap<String, Value> = IndexMap::new();
        m.insert(String::new(), root.clone());
        h.new_object(m)
    });
    let v = apply_to_json(&wrapper, "", &root, &mut Vec::new(), replacer.as_ref())?;
    // A BigInt anywhere in a serializable position is a TypeError (JSON has no
    // bigint form), matching Node's exact message.
    if with_host(|h| json_has_bigint(h, &v)) {
        return Err(host::type_error("Do not know how to serialize a BigInt"));
    }
    let indent = match args.get(2) {
        Some(Value::Float(f)) => " ".repeat((*f as usize).min(10)),
        Some(other) => with_host(|h| h.as_str(other)).unwrap_or_default(),
        None => String::new(),
    };
    // A replacer array (args[1]) restricts which object keys are serialized.
    let keys: Option<Vec<String>> = args.get(1).and_then(|r| {
        with_host(|h| match h.get(r) {
            Some(JsObj::Array(items)) => {
                Some(items.iter().map(|k| h.str_of(k)).collect::<Vec<_>>())
            }
            _ => None,
        })
    });
    let s = with_host(|h| json_str(h, &v, &indent, 0, keys.as_deref()));
    match s {
        Some(s) => Ok(with_host(|h| h.new_str(s))),
        None => Ok(Value::Undef),
    }
}

/// One `SerializeJSONProperty(key, holder)` step: rewrite `v` (the value read
/// from `holder[key]`) by calling its `toJSON(key)` and then the replacer
/// function as `replacer.call(holder, key, value)`, then recurse into whatever
/// object survives. Applies to user methods, class methods, and the native
/// `Date`/`Buffer`/`URL` accessors alike.
///
/// Returns a fresh tree; the input is never mutated. `path` carries the chain of
/// objects currently being walked so a cyclic structure is reported rather than
/// spinning forever.
///
/// `toJSON` is called on the value ONCE and is NOT re-applied to its own result
/// — `{toJSON(){ return {toJSON(){ return 1 }} }}` serializes as `{}` in Node,
/// because the inner method is a plain (unserializable) function property of the
/// returned object, not a second conversion hook.
fn apply_to_json(
    holder: &Value,
    key: &str,
    v: &Value,
    path: &mut Vec<Value>,
    rep: Option<&Value>,
) -> Result<Value, String> {
    let mut v = v.clone();
    if matches!(v, Value::Obj(_)) {
        let tag = crate::stdlib::native_tag(&v);
        let has_to_json = with_host(|h| match host::lookup_chain(h, &v, "toJSON") {
            Some(f) => host::is_callable(h, &f),
            None => false,
        }) || tag
            .as_deref()
            .map(crate::stdlib::has_to_json)
            .unwrap_or(false);
        if has_to_json {
            let k = with_host(|h| h.new_str(key.to_string()));
            v = host::call_method(&v, "toJSON", vec![k])?;
        }
    }
    if let Some(rep) = rep {
        let k = with_host(|h| h.new_str(key.to_string()));
        v = host::invoke(rep, vec![k, v.clone()], Some(holder.clone()))?;
    }
    json_walk_children(&v, path, rep)
}

/// Whether a raw property key of a host object is one `json_str` serializes. The
/// internal slots (`@@`-prefixed symbol keys, `#`-prefixed private fields) are
/// invisible to JSON, so the replacer must not be invoked for them either.
fn json_visible_key(k: &str) -> bool {
    !k.starts_with("@@") && !k.starts_with('#')
}

/// Recurse into the elements/properties of an already-converted value, running
/// `apply_to_json` for each with this value as the holder.
fn json_walk_children(
    v: &Value,
    path: &mut Vec<Value>,
    rep: Option<&Value>,
) -> Result<Value, String> {
    if !matches!(v, Value::Obj(_)) {
        return Ok(v.clone());
    }
    // A value that contains itself has no JSON form.
    if with_host(|h| path.iter().any(|p| h.strict_eq(p, v))) {
        return Err(host::type_error("Converting circular structure to JSON"));
    }
    // A Proxy owns no property map, so it is snapshotted through its traps into
    // the plain array/object `SerializeJSONArray`/`SerializeJSONObject` describe
    // — which read every member through `[[Get]]`, exactly as the snapshot does.
    if with_host(|h| h.kind_of(v)) == Some(ObjKind::Proxy) {
        let snap = crate::proxy::json_snapshot(v)?;
        path.push(v.clone());
        let out = json_walk_children(&snap, path, rep);
        path.pop();
        return out;
    }
    let obj = with_host(|h| h.get(v).cloned());
    path.push(v.clone());
    let out = (|| match obj {
        Some(JsObj::Array(items)) => {
            let mut out = Vec::with_capacity(items.len());
            let mut changed = false;
            for (i, it) in items.iter().enumerate() {
                let nv = apply_to_json(v, &i.to_string(), it, path, rep)?;
                changed |= !with_host(|h| h.strict_eq(&nv, it));
                out.push(nv);
            }
            // Keep identity when nothing changed, so an enclosing object is not
            // needlessly rebuilt (which would drop its property attributes).
            if changed {
                Ok(with_host(|h| h.new_array(out)))
            } else {
                Ok(v.clone())
            }
        }
        Some(JsObj::Object(props)) => {
            // An enumerable own accessor must have its getter RUN and the result
            // serialized. That cannot happen inside `json_str` (which holds the
            // host borrow), so materialize here — the same reason `toJSON` is
            // applied in this pass.
            let has_accessor = with_host(|h| {
                h.own_accessor_keys(v)
                    .iter()
                    .any(|k| h.prop_attrs(v, k).enumerable)
            });
            if has_accessor {
                let mut next: IndexMap<String, Value> = IndexMap::new();
                for (k, val) in host::own_enum_entries_deep(v) {
                    let nv = if json_visible_key(&k) {
                        apply_to_json(v, &k, &val, path, rep)?
                    } else {
                        val
                    };
                    next.insert(k, nv);
                }
                return Ok(with_host(|h| h.new_object(next)));
            }
            // Only rebuild when a descendant actually changed, so plain data keeps
            // its identity (and its prototype / native tag).
            let mut next: IndexMap<String, Value> = IndexMap::new();
            let mut changed = false;
            for (k, val) in &props {
                let nv = if json_visible_key(k) {
                    apply_to_json(v, k, val, path, rep)?
                } else {
                    val.clone()
                };
                changed |= !with_host(|h| h.strict_eq(&nv, val));
                next.insert(k.clone(), nv);
            }
            if changed {
                Ok(with_host(|h| {
                    let o = h.new_object(next);
                    h.copy_prop_attrs(v, &o);
                    o
                }))
            } else {
                Ok(v.clone())
            }
        }
        _ => Ok(v.clone()),
    })();
    path.pop();
    out
}

/// Whether a value tree contains a `BigInt` in a position `JSON.stringify` would
/// try to serialize (a value in an array/object) — such a value throws.
fn json_has_bigint(h: &host::JsHost, v: &Value) -> bool {
    match h.get(v) {
        Some(JsObj::BigInt(_)) => true,
        Some(JsObj::Array(items)) => items.iter().any(|x| json_has_bigint(h, x)),
        Some(JsObj::Object(props)) => props
            .iter()
            .filter(|(k, _)| !k.starts_with("@@") && !k.starts_with('#'))
            .any(|(_, val)| json_has_bigint(h, val)),
        _ => false,
    }
}

fn json_str(
    h: &host::JsHost,
    v: &Value,
    indent: &str,
    depth: usize,
    keys: Option<&[String]>,
) -> Option<String> {
    let sep = if indent.is_empty() { ":" } else { ": " };
    match v {
        Value::Undef => None,
        Value::Bool(b) => Some(if *b { "true".into() } else { "false".into() }),
        Value::Int(n) => Some(n.to_string()),
        Value::Float(f) => Some(if f.is_finite() {
            host::fmt_number(*f)
        } else {
            "null".into()
        }),
        Value::Str(s) => Some(json_quote(s)),
        Value::Obj(_) => match h.get(v) {
            Some(JsObj::Str(s)) => Some(json_quote(s)),
            Some(JsObj::Null) => Some("null".into()),
            // Map/Set have no enumerable own string keys → serialize as `{}`.
            Some(JsObj::Map { .. }) | Some(JsObj::Set { .. }) => Some("{}".into()),
            // Functions and symbols are omitted (undefined) as values.
            Some(JsObj::Func(_))
            | Some(JsObj::Builtin(_))
            | Some(JsObj::BoundMethod { .. })
            | Some(JsObj::BoundFunc { .. })
            | Some(JsObj::Class(_))
            | Some(JsObj::Symbol { .. })
            | Some(JsObj::Generator { .. }) => None,
            Some(JsObj::Array(items)) => {
                if items.is_empty() {
                    return Some("[]".into());
                }
                let parts: Vec<String> = items
                    .iter()
                    .map(|x| {
                        json_str(h, x, indent, depth + 1, keys).unwrap_or_else(|| "null".into())
                    })
                    .collect();
                Some(wrap(&parts, "[", "]", indent, depth))
            }
            Some(JsObj::Object(props)) => {
                // A replacer array restricts (and orders) which keys are emitted.
                let parts: Vec<String> = match keys {
                    Some(allow) => allow
                        .iter()
                        .filter_map(|k| {
                            props.get(k).and_then(|val| {
                                json_str(h, val, indent, depth + 1, keys)
                                    .map(|vs| format!("{}{sep}{vs}", json_quote(k)))
                            })
                        })
                        .collect(),
                    None => h
                        .own_enum_entries(v)
                        .iter()
                        .filter_map(|(k, val)| {
                            json_str(h, val, indent, depth + 1, keys)
                                .map(|vs| format!("{}{sep}{vs}", json_quote(k)))
                        })
                        .collect(),
                };
                if parts.is_empty() {
                    return Some("{}".into());
                }
                Some(wrap(&parts, "{", "}", indent, depth))
            }
            _ => Some("null".into()),
        },
        _ => Some("null".into()),
    }
}

fn wrap(parts: &[String], open: &str, close: &str, indent: &str, depth: usize) -> String {
    if indent.is_empty() {
        format!("{open}{}{close}", parts.join(","))
    } else {
        let pad = indent.repeat(depth + 1);
        let pad_close = indent.repeat(depth);
        format!(
            "{open}\n{pad}{}\n{pad_close}{close}",
            parts.join(&format!(",\n{pad}"))
        )
    }
}

fn json_quote(s: &str) -> String {
    let mut out = String::from("\"");
    for c in s.chars() {
        match c {
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            '\n' => out.push_str("\\n"),
            '\t' => out.push_str("\\t"),
            '\r' => out.push_str("\\r"),
            // QuoteJSONString (25.5.2.2) names SIX short escapes, not four.
            // Backspace and form feed were missing, so they fell through to the
            // `\uXXXX` arm below and `JSON.stringify("\b")` produced
            // `""` where node produces `"\b"`. Both parse back to the same
            // string, so the difference is invisible to a round trip and shows
            // up only as a byte mismatch against a fixture or a checksum.
            '\u{8}' => out.push_str("\\b"),
            '\u{c}' => out.push_str("\\f"),
            c if (c as u32) < 0x20 => out.push_str(&format!("\\u{:04x}", c as u32)),
            _ => out.push(c),
        }
    }
    out.push('"');
    out
}

fn json_parse(args: Vec<Value>) -> Result<Value, String> {
    let s = with_host(|h| h.str_of(&arg0(&args)));
    let mut p = JsonParser {
        chars: s.chars().collect(),
        pos: 0,
    };
    p.skip_ws();
    if p.peek().is_none() {
        return Err("SyntaxError: Unexpected end of JSON input".into());
    }
    let v = p.parse_value()?;
    let value_end = p.pos;
    p.skip_ws();
    // Anything after the top-level value is an error — the parser used to accept
    // and silently discard it, so `JSON.parse('{"a":1}x')` succeeded.
    if let Some(c) = p.peek() {
        // V8 names the token kind only when it butts directly against the value
        // (`01` -> "Unexpected number at position 1"); with whitespace between
        // it is just a non-whitespace character (`1 2`).
        // Only a digit butted directly against a completed number literal —
        // V8's number scanner is still in number context there. `5"x"` and
        // `[0,1]0` exit the scanner cleanly and get the generic message.
        let after_number = value_end > 0
            && p.pos == value_end
            && p.chars[value_end - 1].is_ascii_digit()
            && c.is_ascii_digit();
        return Err(if after_number {
            p.err_at("Unexpected number", p.pos)
        } else {
            p.err_trailing(p.pos)
        });
    }
    // Optional reviver: walk bottom-up, transforming each (key, value).
    if let Some(reviver) = args
        .get(1)
        .filter(|r| with_host(|h| host::is_callable(h, r)))
        .cloned()
    {
        return json_revive("", v, &reviver);
    }
    Ok(v)
}

/// `JSON.parse` reviver walk: recurse into children first, then call
/// `reviver(key, value)`; a returned `undefined` drops the property.
fn json_revive(key: &str, val: Value, reviver: &Value) -> Result<Value, String> {
    match with_host(|h| h.get(&val).cloned()) {
        Some(JsObj::Array(items)) => {
            for i in 0..items.len() {
                let elem = with_host(|h| match h.get(&val) {
                    Some(JsObj::Array(it)) => it[i].clone(),
                    _ => Value::Undef,
                });
                let nv = json_revive(&i.to_string(), elem, reviver)?;
                with_host(|h| {
                    if let Some(JsObj::Array(it)) = h.get_mut(&val) {
                        it[i] = nv;
                    }
                });
            }
        }
        Some(JsObj::Object(props)) => {
            let keys: Vec<String> = props
                .keys()
                .filter(|k| !k.starts_with("@@"))
                .cloned()
                .collect();
            for k in keys {
                let elem = with_host(|h| match h.get(&val) {
                    Some(JsObj::Object(p)) => p.get(&k).cloned().unwrap_or(Value::Undef),
                    _ => Value::Undef,
                });
                let nv = json_revive(&k, elem, reviver)?;
                with_host(|h| {
                    if let Some(JsObj::Object(p)) = h.get_mut(&val) {
                        if matches!(nv, Value::Undef) {
                            p.shift_remove(&k);
                        } else {
                            p.insert(k.clone(), nv);
                        }
                    }
                });
            }
        }
        _ => {}
    }
    let kv = with_host(|h| h.new_str(key.to_string()));
    host::invoke(reviver, vec![kv, val], None)
}

struct JsonParser {
    chars: Vec<char>,
    pos: usize,
}
impl JsonParser {
    fn peek(&self) -> Option<char> {
        self.chars.get(self.pos).copied()
    }

    /// `at position N (line L column C)` — the location suffix V8 appends to the
    /// positional JSON parse errors. Positions are in UTF-16-ish code units;
    /// node-js counts `char`s, which agree for the BMP.
    fn at(&self, pos: usize) -> String {
        let mut line = 1usize;
        let mut col = 1usize;
        for c in &self.chars[..pos.min(self.chars.len())] {
            if *c == '\n' {
                line += 1;
                col = 1;
            } else {
                col += 1;
            }
        }
        format!(" at position {pos} (line {line} column {col})")
    }

    /// A positional error (`Expected ':' after property name in JSON at …`).
    fn err_at(&self, what: &str, pos: usize) -> String {
        format!("SyntaxError: {what} in JSON{}", self.at(pos))
    }

    /// The one positional message V8 does NOT suffix with `in JSON`.
    fn err_trailing(&self, pos: usize) -> String {
        format!(
            "SyntaxError: Unexpected non-whitespace character after JSON{}",
            self.at(pos)
        )
    }

    /// V8's default parse error: the offending character plus a window of the
    /// source. The whole input is quoted when it is short (<= 20 chars);
    /// otherwise a 10-character context window either side of `pos` is shown,
    /// elided with `...` on whichever side was cut.
    fn err_token(&self, pos: usize) -> String {
        const MAX_WHOLE: usize = 20;
        const CONTEXT: usize = 10;
        let len = self.chars.len();
        let Some(c) = self.chars.get(pos) else {
            return "SyntaxError: Unexpected end of JSON input".into();
        };
        // V8 reports the whole input for the JS literals that are famously not
        // JSON, without naming an offending character.
        let whole: String = self.chars.iter().collect();
        if matches!(
            whole.as_str(),
            "undefined" | "NaN" | "Infinity" | "-Infinity"
        ) {
            return format!("SyntaxError: \"{whole}\" is not valid JSON");
        }
        let snippet = if len <= MAX_WHOLE {
            format!("\"{whole}\"")
        } else {
            let start = pos.saturating_sub(CONTEXT);
            let end = (pos + CONTEXT).min(len);
            let body: String = self.chars[start..end].iter().collect();
            let head = if start > 0 { "..." } else { "" };
            let tail = if end < len { "..." } else { "" };
            format!("{head}\"{body}\"{tail}")
        };
        format!("SyntaxError: Unexpected token '{c}', {snippet} is not valid JSON")
    }

    fn skip_ws(&mut self) {
        while matches!(
            self.peek(),
            Some(' ') | Some('\n') | Some('\t') | Some('\r')
        ) {
            self.pos += 1;
        }
    }
    fn parse_value(&mut self) -> Result<Value, String> {
        self.skip_ws();
        match self.peek() {
            Some('{') => self.parse_object(),
            Some('[') => self.parse_array(),
            Some('"') => {
                let s = self.parse_string()?;
                Ok(with_host(|h| h.new_str(s)))
            }
            Some('t') | Some('f') => self.parse_bool(),
            Some('n') => {
                self.expect_lit("null")?;
                Ok(with_host(|h| h.null()))
            }
            Some(c) if c == '-' || c.is_ascii_digit() => self.parse_number(),
            None => Err("SyntaxError: Unexpected end of JSON input".into()),
            _ => Err(self.err_token(self.pos)),
        }
    }
    fn expect_lit(&mut self, lit: &str) -> Result<(), String> {
        for ch in lit.chars() {
            match self.peek() {
                Some(c) if c == ch => self.pos += 1,
                // V8 reports the first character that broke the literal, which is
                // why `foo` complains about `'o'` (index 2) and not `'f'`.
                None => return Err("SyntaxError: Unexpected end of JSON input".into()),
                _ => return Err(self.err_token(self.pos)),
            }
        }
        Ok(())
    }
    fn parse_bool(&mut self) -> Result<Value, String> {
        if self.peek() == Some('t') {
            self.expect_lit("true")?;
            Ok(Value::Bool(true))
        } else {
            self.expect_lit("false")?;
            Ok(Value::Bool(false))
        }
    }
    /// JSON's number grammar: `-? (0 | [1-9][0-9]*) (. [0-9]+)? ([eE] [+-]? [0-9]+)?`.
    /// A leading zero does NOT swallow the following digits — `01` parses as `0`
    /// and the stray `1` becomes a trailing-token error, which is how V8 reports
    /// it. Each way the grammar can run out has its own message.
    fn parse_number(&mut self) -> Result<Value, String> {
        let start = self.pos;
        if self.peek() == Some('-') {
            self.pos += 1;
            if !matches!(self.peek(), Some(c) if c.is_ascii_digit()) {
                return Err(self.err_at("No number after minus sign", self.pos));
            }
        }
        if self.peek() == Some('0') {
            self.pos += 1;
        } else {
            while matches!(self.peek(), Some(c) if c.is_ascii_digit()) {
                self.pos += 1;
            }
        }
        if self.peek() == Some('.') {
            self.pos += 1;
            if !matches!(self.peek(), Some(c) if c.is_ascii_digit()) {
                return Err(self.err_at("Unterminated fractional number", self.pos));
            }
            while matches!(self.peek(), Some(c) if c.is_ascii_digit()) {
                self.pos += 1;
            }
        }
        if matches!(self.peek(), Some('e') | Some('E')) {
            self.pos += 1;
            if matches!(self.peek(), Some('+') | Some('-')) {
                self.pos += 1;
            }
            if !matches!(self.peek(), Some(c) if c.is_ascii_digit()) {
                return Err(self.err_at("Exponent part is missing a number", self.pos));
            }
            while matches!(self.peek(), Some(c) if c.is_ascii_digit()) {
                self.pos += 1;
            }
        }
        let s: String = self.chars[start..self.pos].iter().collect();
        s.parse::<f64>()
            .map(Value::Float)
            .map_err(|_| self.err_at("Unexpected number", start))
    }
    fn parse_string(&mut self) -> Result<String, String> {
        self.pos += 1; // opening quote
        let mut out = String::new();
        loop {
            match self.peek() {
                None => return Err(self.err_at("Unterminated string", self.pos)),
                Some('"') => {
                    self.pos += 1;
                    break;
                }
                Some('\\') => {
                    self.pos += 1;
                    match self.peek() {
                        Some('n') => out.push('\n'),
                        Some('t') => out.push('\t'),
                        Some('r') => out.push('\r'),
                        Some('"') => out.push('"'),
                        Some('\\') => out.push('\\'),
                        Some('/') => out.push('/'),
                        Some('b') => out.push('\u{08}'),
                        Some('f') => out.push('\u{0C}'),
                        Some('u') => {
                            let h: String = self.chars
                                [self.pos + 1..(self.pos + 5).min(self.chars.len())]
                                .iter()
                                .collect();
                            if let Ok(n) = u32::from_str_radix(&h, 16) {
                                if let Some(ch) = char::from_u32(n) {
                                    out.push(ch);
                                }
                            }
                            self.pos += 4;
                        }
                        _ => {}
                    }
                    self.pos += 1;
                }
                // A raw control character is not legal inside a JSON string; it
                // has to be escaped. V8 rejects it rather than passing it through.
                Some(c) if (c as u32) < 0x20 => {
                    return Err(self.err_at("Bad control character in string literal", self.pos))
                }
                Some(c) => {
                    out.push(c);
                    self.pos += 1;
                }
            }
        }
        Ok(out)
    }
    fn parse_array(&mut self) -> Result<Value, String> {
        self.pos += 1; // [
        let mut items = Vec::new();
        self.skip_ws();
        if self.peek() == Some(']') {
            self.pos += 1;
            return Ok(with_host(|h| h.new_array(items)));
        }
        loop {
            items.push(self.parse_value()?);
            self.skip_ws();
            match self.peek() {
                Some(',') => {
                    self.pos += 1;
                }
                Some(']') => {
                    self.pos += 1;
                    break;
                }
                _ => return Err(self.err_at("Expected ',' or ']' after array element", self.pos)),
            }
        }
        Ok(with_host(|h| h.new_array(items)))
    }
    fn parse_object(&mut self) -> Result<Value, String> {
        self.pos += 1; // {
        let mut props: IndexMap<String, Value> = IndexMap::new();
        self.skip_ws();
        if self.peek() == Some('}') {
            self.pos += 1;
            return Ok(with_host(|h| h.new_object(props)));
        }
        loop {
            self.skip_ws();
            if self.peek() != Some('"') {
                // The first key uses the "or '}'" wording (an empty object is
                // still legal there); a key after a comma does not. End of input
                // reports the same expectation, at the end position.
                return Err(if props.is_empty() {
                    self.err_at("Expected property name or '}'", self.pos)
                } else {
                    self.err_at("Expected double-quoted property name", self.pos)
                });
            }
            let key = self.parse_string()?;
            self.skip_ws();
            if self.peek() != Some(':') {
                return Err(match self.peek() {
                    None => "SyntaxError: Unexpected end of JSON input".into(),
                    _ => self.err_at("Expected ':' after property name", self.pos),
                });
            }
            self.pos += 1;
            let val = self.parse_value()?;
            props.insert(key, val);
            self.skip_ws();
            match self.peek() {
                Some(',') => {
                    self.pos += 1;
                }
                Some('}') => {
                    self.pos += 1;
                    break;
                }
                _ => return Err(self.err_at("Expected ',' or '}' after property value", self.pos)),
            }
        }
        Ok(with_host(|h| h.new_object(props)))
    }
}

// ══ type methods (array / string / number) ═══════════════════════════════════

fn is_array_method(name: &str) -> bool {
    matches!(
        name,
        "push"
            | "pop"
            | "shift"
            | "unshift"
            | "map"
            | "filter"
            | "forEach"
            | "join"
            | "slice"
            | "indexOf"
            | "lastIndexOf"
            | "includes"
            | "reduce"
            | "concat"
            | "reverse"
            | "sort"
            | "find"
            | "findIndex"
            | "some"
            | "every"
            | "flat"
            | "fill"
            | "splice"
            | "keys"
            | "values"
            | "entries"
            | "flatMap"
            | "at"
            | "toString"
            | "reduceRight"
            | "findLast"
            | "findLastIndex"
            | "copyWithin"
    )
}
fn is_string_method(name: &str) -> bool {
    matches!(
        name,
        "toUpperCase"
            | "toLowerCase"
            | "charAt"
            | "charCodeAt"
            | "codePointAt"
            | "indexOf"
            | "lastIndexOf"
            | "includes"
            | "slice"
            | "substring"
            | "substr"
            | "split"
            | "trim"
            | "trimStart"
            | "trimEnd"
            | "replace"
            | "replaceAll"
            | "repeat"
            | "startsWith"
            | "endsWith"
            | "padStart"
            | "padEnd"
            | "concat"
            | "at"
            | "toString"
            | "toLocaleString"
            | "valueOf"
            | "match"
            | "matchAll"
            | "search"
            | "normalize"
            | "localeCompare"
            | "toLocaleUpperCase"
            | "toLocaleLowerCase"
            | "isWellFormed"
            | "toWellFormed"
    )
}

/// Whether `v` is a `RegExp` value (drives the regex path of `match`/`replace`/…).
fn is_regexp_arg(v: &Value) -> bool {
    with_host(|h| h.kind_of(v)) == Some(ObjKind::RegExp)
}

/// `str.replace(strPattern, fn)` — a function replacer against a literal (string)
/// pattern: replace the first (or all) occurrence, calling `fn(match, offset, s)`.
fn replace_str_fn(s: &str, pat: &str, repl: &Value, all: bool) -> Result<String, String> {
    if pat.is_empty() {
        return Ok(s.to_string());
    }
    let mut out = String::new();
    let mut rest = s;
    let mut base = 0usize;
    while let Some(pos) = rest.find(pat) {
        out.push_str(&rest[..pos]);
        let offset = base + pos;
        let m = with_host(|h| h.new_str(pat.to_string()));
        let str_arg = with_host(|h| h.new_str(s.to_string()));
        let r = host::invoke(repl, vec![m, Value::Float(offset as f64), str_arg], None)?;
        out.push_str(&with_host(|h| h.str_of(&r)));
        let consumed = pos + pat.len();
        base += consumed;
        rest = &rest[consumed..];
        if !all {
            break;
        }
    }
    out.push_str(rest);
    Ok(out)
}
fn is_number_method(name: &str) -> bool {
    matches!(
        name,
        "toFixed" | "toExponential" | "toString" | "toPrecision" | "toLocaleString" | "valueOf"
    )
}

/// Dispatch `recv.name(args)` for the built-in prototype methods.
pub fn call_type_method(recv: &Value, name: &str, args: Vec<Value>) -> Result<Value, String> {
    // `Object.prototype.valueOf` is inherited by every exotic that does not
    // override it (an Array does not), and returns the receiver. Without this
    // the `ToPrimitive` probe on `[o] + ''` reached `array_method("valueOf")`
    // and threw `valueOf is not a function`.
    if name == "valueOf"
        && matches!(
            with_host(|h| h.kind_of(recv)),
            Some(
                ObjKind::Array
                    | ObjKind::Map
                    | ObjKind::Set
                    | ObjKind::Generator
                    | ObjKind::Promise
                    | ObjKind::Iter
                    | ObjKind::RegExp
            )
        )
    {
        return Ok(recv.clone());
    }
    // Only the tag is needed to pick the branch — cloning the receiver here made
    // every `arr.push(x)` copy the whole array, so a fill loop was O(n^2).
    match with_host(|h| h.kind_of(recv)) {
        Some(ObjKind::Array) => array_method(recv, name, args),
        Some(ObjKind::Str) => {
            // `string_method` consumes the text itself, so this clone is the
            // payload, not a tag probe.
            let s = peek(recv, |o| match o {
                JsObj::Str(s) => Some(s.clone()),
                _ => None,
            })
            .unwrap_or_default();
            string_method(&s, name, args)
        }
        Some(ObjKind::Map) => map_method(recv, name, args),
        Some(ObjKind::Set) => set_method(recv, name, args),
        Some(ObjKind::Generator) => generator_method(recv, name, args),
        Some(ObjKind::Promise) => promise_method(recv, name, args),
        Some(ObjKind::Iter) => iter_method(recv, name, args),
        Some(ObjKind::Symbol) => symbol_method(recv, name, args),
        Some(ObjKind::BigInt) => {
            let b = peek(recv, |o| match o {
                JsObj::BigInt(b) => Some(b.clone()),
                _ => None,
            })
            .unwrap_or_default();
            bigint_method(&b, name, args)
        }
        Some(ObjKind::RegExp) => crate::regexp::regexp_method(recv, name, args),
        Some(ObjKind::Func) | Some(ObjKind::Class) | Some(ObjKind::BoundFunc) => {
            match function_builtin_method(recv, name, &args)? {
                Some(v) => Ok(v),
                None => Err(host::type_error(&format!("{name} is not a function"))),
            }
        }
        Some(ObjKind::Object) => {
            if let Some(f) = peek(recv, |o| match o {
                JsObj::Object(p) => p.get(name).cloned(),
                _ => None,
            }) {
                host::invoke(&f, args, Some(recv.clone()))
            } else if name == "hasOwnProperty" {
                let k = with_host(|h| h.str_of(&arg0(&args)));
                let has = peek(recv, |o| match o {
                    JsObj::Object(p) => Some(p.contains_key(&k)),
                    _ => None,
                })
                .unwrap_or(false);
                Ok(Value::Bool(has))
            } else if name == "toString" {
                Ok(with_host(|h| h.new_str("[object Object]")))
            } else {
                Err(host::type_error(&format!("{} is not a function", name)))
            }
        }
        _ => {
            // Primitive number/bool/string coercions.
            if let Value::Float(_) | Value::Int(_) = recv {
                return number_method(with_host(|h| h.to_number(recv)), name, args);
            }
            if let Some(s) = with_host(|h| h.as_str(recv)) {
                return string_method(&s, name, args);
            }
            // `Boolean.prototype` (20.3.3): a boolean is not a heap object here,
            // so it reached no branch at all and `true.toString()` threw `is not
            // a function`. Its three methods are `toString`, `valueOf`, and the
            // inherited `Object.prototype.toLocaleString` — which
            // `[1,'a',true].toLocaleString()` invokes per element, so the hole
            // was reachable from the array form too.
            if let Value::Bool(b) = recv {
                return match name {
                    "toString" | "toLocaleString" => {
                        Ok(new_s(if *b { "true" } else { "false" }.to_string()))
                    }
                    "valueOf" => Ok(Value::Bool(*b)),
                    _ => Err(host::type_error(&format!("{name} is not a function"))),
                };
            }
            Err(host::type_error(&format!("{} is not a function", name)))
        }
    }
}

/// A copy of the whole backing store, for the methods that genuinely consume
/// every element (`map`, `filter`, `join`, …). Never call it just to read
/// `.len()` — use [`array_len`], or `push`/`unshift` become O(n) per call.
fn array_items(recv: &Value) -> Vec<Value> {
    with_host(|h| match h.get(recv) {
        Some(JsObj::Array(items)) => items.clone(),
        _ => Vec::new(),
    })
}

/// The ELIDED positions of array `recv` as a membership set. A dense array —
/// which is nearly every array — answers with an empty set after a single
/// negative hash probe and allocates nothing.
///
/// The iteration methods split into two groups, and the split is not a matter of
/// taste: the ones spec'd through `HasProperty` (`forEach`, `map`, `filter`,
/// `some`, `every`, `reduce`, `indexOf`, `flat`, `sort`) SKIP a hole, while the
/// ones spec'd through a bare `Get` (`for…of`, spread, `find`, `includes`,
/// `join`, `entries`, `Array.from`) see the `undefined` a hole reads back as.
fn hole_set(recv: &Value) -> rustc_hash::FxHashSet<usize> {
    with_host(|h| h.hole_indices(recv)).into_iter().collect()
}

/// The element count, without copying the elements.
fn array_len(recv: &Value) -> usize {
    peek(recv, |o| match o {
        JsObj::Array(items) => Some(items.len()),
        _ => None,
    })
    .unwrap_or(0)
}

fn array_method(recv: &Value, name: &str, args: Vec<Value>) -> Result<Value, String> {
    array_method_on(recv, recv, name, args)
}

/// The `Array.prototype` methods that WRITE to their receiver, and so need the
/// generic path to copy the result back onto the array-like.
const ARRAY_MUTATORS: &[&str] = &[
    "push",
    "pop",
    "shift",
    "unshift",
    "splice",
    "sort",
    "reverse",
    "fill",
    "copyWithin",
];

/// Run `Array.prototype.<method>` against an array-LIKE (`{0: 'a', length: 1}`,
/// a DOM-ish collection, `arguments`).
///
/// 23.1.3 defines every one of these over `LengthOfArrayLike(O)` and `Get(O, k)`
/// rather than over an Array's element vector, so the receiver only has to have
/// a `length`. The elements are read out into a temporary Array, the ordinary
/// implementation runs on that, and a MUTATING method writes the result back —
/// which keeps one implementation of each method rather than a second, generic
/// one that could drift from it.
///
/// An index the receiver does not own is a HOLE in the temporary, so the
/// methods that skip holes skip it here too, exactly as `HasProperty` makes them.
fn array_generic(recv: &Value, method: &str, args: Vec<Value>) -> Result<Value, String> {
    let len = match get_property(recv, "length") {
        Ok(v) => host::to_array_length(&v).unwrap_or(0),
        Err(_) => 0,
    };
    // A STRING receiver owns every index of its length; `has_property` answers
    // for objects and reports none of them, which made `[].map.call('abc', f)`
    // an array of three holes.
    let dense = with_host(|h| h.as_str(recv)).is_some();
    let mut items = Vec::with_capacity(len);
    let mut holes: rustc_hash::FxHashSet<usize> = rustc_hash::FxHashSet::default();
    for i in 0..len {
        let k = i.to_string();
        if dense || has_property(recv, &k)? {
            items.push(get_property(recv, &k)?);
        } else {
            holes.insert(i);
            items.push(Value::Undef);
        }
    }
    let tmp = with_host(|h| {
        let a = h.new_array(items);
        h.install_holes(&a, holes);
        a
    });
    let out = array_method_on(&tmp, recv, method, args)?;
    if ARRAY_MUTATORS.contains(&method) {
        let result = with_host(|h| match h.get(&tmp) {
            Some(JsObj::Array(items)) => items.clone(),
            _ => Vec::new(),
        });
        for (i, v) in result.iter().enumerate() {
            set_property(recv, &i.to_string(), v.clone())?;
        }
        set_property(recv, "length", Value::Float(result.len() as f64))?;
    }
    Ok(out)
}

/// `Array.prototype.<name>` on `recv`.
///
/// `this_value` is what a callback receives as its third argument and what a
/// mutating method returns — the same object as `recv` for an ordinary array
/// call, but the ORIGINAL array-like when `array_generic` runs a method against
/// a temporary copy (`Array.prototype.slice.call(arguments)`).
fn array_method_on(
    recv: &Value,
    this_value: &Value,
    name: &str,
    args: Vec<Value>,
) -> Result<Value, String> {
    match name {
        "push" => {
            // `push` returns the new length; take it from the same mutable
            // borrow rather than copying the array back out to count it.
            let len = with_host(|h| {
                if let Some(JsObj::Array(items)) = h.get_mut(recv) {
                    items.extend(args.iter().cloned());
                    items.len()
                } else {
                    0
                }
            });
            Ok(Value::Float(len as f64))
        }
        "pop" => Ok(with_host(|h| {
            let popped = if let Some(JsObj::Array(items)) = h.get_mut(recv) {
                items.pop().unwrap_or(Value::Undef)
            } else {
                Value::Undef
            };
            let len = match h.get(recv) {
                Some(JsObj::Array(items)) => items.len(),
                _ => 0,
            };
            h.truncate_holes(recv, len);
            popped
        })),
        "shift" => Ok(with_host(|h| {
            let shifted = if let Some(JsObj::Array(items)) = h.get_mut(recv) {
                if items.is_empty() {
                    Value::Undef
                } else {
                    items.remove(0)
                }
            } else {
                Value::Undef
            };
            h.remap_holes(recv, |i| i.checked_sub(1));
            shifted
        })),
        "unshift" => {
            with_host(|h| {
                if let Some(JsObj::Array(items)) = h.get_mut(recv) {
                    for (i, a) in args.iter().enumerate() {
                        items.insert(i, a.clone());
                    }
                }
                let n = args.len();
                h.remap_holes(recv, |i| Some(i + n));
            });
            Ok(Value::Float(array_len(recv) as f64))
        }
        "join" => {
            let sep = if args.is_empty() {
                ",".to_string()
            } else {
                with_host(|h| h.str_of(&args[0]))
            };
            join_array(recv, &sep)
        }
        // `Array.prototype.toLocaleString` (23.1.3.32): comma-join the elements'
        // OWN `toLocaleString` results, with `null`/`undefined` contributing the
        // empty string. It threw `is not a function` — the whole method was
        // missing — so `[1234.5, 'x'].toLocaleString()` was unreachable.
        "toLocaleString" => {
            // Shares `join`'s JoinStack: measured on node v26.7.0, `h=[1]`
            // `h.push(h)` makes `h.toLocaleString()` `"1,"`, not a stack overflow.
            if !host::join_stack_push(recv) {
                return Ok(with_host(|h| h.new_str(String::new())));
            }
            let items = array_items(recv);
            let mut parts: Vec<String> = Vec::with_capacity(items.len());
            for it in &items {
                if with_host(|h| h.is_nullish(it)) {
                    parts.push(String::new());
                    continue;
                }
                let v = match host::call_method(it, "toLocaleString", Vec::new()) {
                    Ok(v) => v,
                    Err(e) => {
                        host::join_stack_pop();
                        return Err(e);
                    }
                };
                parts.push(with_host(|h| h.str_of(&v)));
            }
            host::join_stack_pop();
            Ok(with_host(|h| h.new_str(parts.join(","))))
        }
        // `indexOf`/`lastIndexOf` are spec'd through `HasProperty`, so a hole is
        // never a match: `[1,,3].indexOf(undefined)` is `-1`, while the
        // `Get`-based `includes` reports `true` for the same array.
        "indexOf" => {
            let items = array_items(recv);
            let holes = hole_set(recv);
            let target = arg0(&args);
            let idx = with_host(|h| {
                items
                    .iter()
                    .enumerate()
                    .position(|(i, x)| !holes.contains(&i) && h.strict_eq(x, &target))
            });
            Ok(Value::Float(idx.map(|i| i as f64).unwrap_or(-1.0)))
        }
        "lastIndexOf" => {
            let items = array_items(recv);
            let holes = hole_set(recv);
            let target = arg0(&args);
            let idx = with_host(|h| {
                items
                    .iter()
                    .enumerate()
                    .rposition(|(i, x)| !holes.contains(&i) && h.strict_eq(x, &target))
            });
            Ok(Value::Float(idx.map(|i| i as f64).unwrap_or(-1.0)))
        }
        "includes" => {
            // Array.includes uses SameValueZero: unlike `===`, NaN matches NaN.
            let items = array_items(recv);
            let target = arg0(&args);
            let tnan = matches!(target, Value::Float(f) if f.is_nan());
            Ok(Value::Bool(with_host(|h| {
                items.iter().any(|x| {
                    (tnan && matches!(x, Value::Float(f) if f.is_nan())) || h.strict_eq(x, &target)
                })
            })))
        }
        "slice" => {
            let items = array_items(recv);
            let (lo, hi) = slice_bounds(&args, items.len());
            Ok(with_host(|h| {
                let out = h.new_array(items[lo..hi].to_vec());
                h.copy_holes(recv, &out, |i| (i >= lo && i < hi).then(|| i - lo));
                out
            }))
        }
        "concat" => {
            let mut out = array_items(recv);
            // A hole in either the receiver or a spreadable argument stays a hole
            // in the result, at its shifted position.
            let mut holes = hole_set(recv);
            let mut sources: Vec<(Value, usize)> = Vec::new();
            for a in &args {
                match with_host(|h| h.get(a).cloned()) {
                    Some(JsObj::Array(items)) => {
                        sources.push((a.clone(), out.len()));
                        out.extend(items);
                    }
                    _ => out.push(a.clone()),
                }
            }
            for (src, base) in sources {
                holes.extend(
                    with_host(|h| h.hole_indices(&src))
                        .into_iter()
                        .map(|i| i + base),
                );
            }
            Ok(with_host(|h| {
                let arr = h.new_array(out);
                h.install_holes(&arr, holes);
                arr
            }))
        }
        "reverse" => {
            let len = array_len(recv);
            with_host(|h| {
                if let Some(JsObj::Array(items)) = h.get_mut(recv) {
                    items.reverse();
                }
                h.remap_holes(recv, |i| Some(len - 1 - i));
            });
            Ok(this_value.clone())
        }
        "fill" => {
            // fill(value[, start[, end]]) — negative indices count from the end.
            let val = arg0(&args);
            let len = array_len(recv) as i64;
            let norm =
                |v: i64| -> usize { (if v < 0 { (len + v).max(0) } else { v.min(len) }) as usize };
            let start = if args.len() >= 2 {
                norm(arg_num(&args, 1) as i64)
            } else {
                0
            };
            let end = if args.len() >= 3 {
                norm(arg_num(&args, 2) as i64)
            } else {
                len as usize
            };
            with_host(|h| {
                if let Some(JsObj::Array(items)) = h.get_mut(recv) {
                    for it in items.iter_mut().take(end).skip(start) {
                        *it = val.clone();
                    }
                }
                // Every filled position now holds a real value.
                h.remap_holes(recv, |i| (i < start || i >= end).then_some(i));
            });
            Ok(this_value.clone())
        }
        "copyWithin" => {
            // copyWithin(target, start[, end]) — copy a slice within the array.
            let items = array_items(recv);
            let len = items.len() as i64;
            let norm =
                |v: i64| -> usize { (if v < 0 { (len + v).max(0) } else { v.min(len) }) as usize };
            let target = norm(arg_num(&args, 0) as i64);
            let start = if args.len() >= 2 {
                norm(arg_num(&args, 1) as i64)
            } else {
                0
            };
            let end = if args.len() >= 3 {
                norm(arg_num(&args, 2) as i64)
            } else {
                len as usize
            };
            let slice: Vec<Value> = items[start..end.max(start)].to_vec();
            let copied = slice.len();
            // A copied position takes its SOURCE's hole-ness (10.4.2 copyWithin
            // deletes the target when the source has no such property);
            // everything outside the written range keeps its own.
            let src_holes = hole_set(recv);
            with_host(|h| {
                if let Some(JsObj::Array(a)) = h.get_mut(recv) {
                    for (k, v) in slice.into_iter().enumerate() {
                        if target + k < a.len() {
                            a[target + k] = v;
                        }
                    }
                }
                let len = len as usize;
                let mut holes: rustc_hash::FxHashSet<usize> = src_holes
                    .iter()
                    .copied()
                    .filter(|i| *i < target || *i >= (target + copied).min(len))
                    .collect();
                for k in 0..copied {
                    if target + k < len && src_holes.contains(&(start + k)) {
                        holes.insert(target + k);
                    }
                }
                h.install_holes(recv, holes);
            });
            Ok(this_value.clone())
        }
        "at" => {
            let items = array_items(recv);
            let mut i = arg_num(&args, 0) as i64;
            if i < 0 {
                i += items.len() as i64;
            }
            Ok(if i >= 0 && (i as usize) < items.len() {
                items[i as usize].clone()
            } else {
                Value::Undef
            })
        }
        // 23.1.3.21: the callback runs only where `HasProperty` holds, and the
        // result array is created with the SAME holes — `[1,,3].map(f)` calls `f`
        // twice and yields `[2, <1 empty item>, 6]`.
        "map" => {
            let items = array_items(recv);
            let holes = hole_set(recv);
            let cb = arg0(&args);
            let mut out = Vec::with_capacity(items.len());
            for (i, it) in items.iter().enumerate() {
                if holes.contains(&i) {
                    out.push(Value::Undef);
                    continue;
                }
                out.push(host::invoke(
                    &cb,
                    vec![it.clone(), Value::Float(i as f64), this_value.clone()],
                    None,
                )?);
            }
            Ok(with_host(|h| {
                let arr = h.new_array(out);
                h.install_holes(&arr, holes);
                arr
            }))
        }
        "flatMap" => {
            let items = array_items(recv);
            let cb = arg0(&args);
            let holes = hole_set(recv);
            let mut out = Vec::new();
            for (i, it) in items.iter().enumerate() {
                if holes.contains(&i) {
                    continue;
                }
                let r = host::invoke(
                    &cb,
                    vec![it.clone(), Value::Float(i as f64), this_value.clone()],
                    None,
                )?;
                match with_host(|h| h.get(&r).cloned()) {
                    Some(JsObj::Array(inner)) => out.extend(inner),
                    _ => out.push(r),
                }
            }
            Ok(with_host(|h| h.new_array(out)))
        }
        "filter" => {
            let items = array_items(recv);
            let holes = hole_set(recv);
            let cb = arg0(&args);
            let mut out = Vec::new();
            for (i, it) in items.iter().enumerate() {
                if holes.contains(&i) {
                    continue;
                }
                let keep = host::invoke(
                    &cb,
                    vec![it.clone(), Value::Float(i as f64), this_value.clone()],
                    None,
                )?;
                if with_host(|h| h.truthy(&keep)) {
                    out.push(it.clone());
                }
            }
            Ok(with_host(|h| h.new_array(out)))
        }
        "forEach" => {
            let items = array_items(recv);
            let holes = hole_set(recv);
            let cb = arg0(&args);
            for (i, it) in items.iter().enumerate() {
                if holes.contains(&i) {
                    continue;
                }
                host::invoke(
                    &cb,
                    vec![it.clone(), Value::Float(i as f64), this_value.clone()],
                    None,
                )?;
            }
            Ok(Value::Undef)
        }
        "find" => {
            let items = array_items(recv);
            let cb = arg0(&args);
            for (i, it) in items.iter().enumerate() {
                let m = host::invoke(
                    &cb,
                    vec![it.clone(), Value::Float(i as f64), this_value.clone()],
                    None,
                )?;
                if with_host(|h| h.truthy(&m)) {
                    return Ok(it.clone());
                }
            }
            Ok(Value::Undef)
        }
        "findIndex" => {
            let items = array_items(recv);
            let cb = arg0(&args);
            for (i, it) in items.iter().enumerate() {
                let m = host::invoke(
                    &cb,
                    vec![it.clone(), Value::Float(i as f64), this_value.clone()],
                    None,
                )?;
                if with_host(|h| h.truthy(&m)) {
                    return Ok(Value::Float(i as f64));
                }
            }
            Ok(Value::Float(-1.0))
        }
        "some" => {
            let items = array_items(recv);
            let holes = hole_set(recv);
            let cb = arg0(&args);
            for (i, it) in items.iter().enumerate() {
                if holes.contains(&i) {
                    continue;
                }
                let m = host::invoke(
                    &cb,
                    vec![it.clone(), Value::Float(i as f64), this_value.clone()],
                    None,
                )?;
                if with_host(|h| h.truthy(&m)) {
                    return Ok(Value::Bool(true));
                }
            }
            Ok(Value::Bool(false))
        }
        "every" => {
            let items = array_items(recv);
            let holes = hole_set(recv);
            let cb = arg0(&args);
            for (i, it) in items.iter().enumerate() {
                if holes.contains(&i) {
                    continue;
                }
                let m = host::invoke(
                    &cb,
                    vec![it.clone(), Value::Float(i as f64), this_value.clone()],
                    None,
                )?;
                if !with_host(|h| h.truthy(&m)) {
                    return Ok(Value::Bool(false));
                }
            }
            Ok(Value::Bool(true))
        }
        "reduce" => {
            let items = array_items(recv);
            let holes = hole_set(recv);
            let cb = arg0(&args);
            let mut acc;
            let mut start = 0;
            if args.len() >= 2 {
                acc = args[1].clone();
            } else {
                // With no seed the accumulator is the first PRESENT element, so a
                // leading run of holes is skipped rather than seeding `undefined`.
                match (0..items.len()).find(|i| !holes.contains(i)) {
                    Some(i) => {
                        acc = items[i].clone();
                        start = i + 1;
                    }
                    None => {
                        return Err(host::type_error(
                            "Reduce of empty array with no initial value",
                        ))
                    }
                }
            }
            for (i, it) in items.iter().enumerate().skip(start) {
                if holes.contains(&i) {
                    continue;
                }
                acc = host::invoke(
                    &cb,
                    vec![acc, it.clone(), Value::Float(i as f64), this_value.clone()],
                    None,
                )?;
            }
            Ok(acc)
        }
        "reduceRight" => {
            let items = array_items(recv);
            let holes = hole_set(recv);
            let cb = arg0(&args);
            let n = items.len();
            let mut acc;
            let mut i = n; // one past the next index to process (walking down)
            if args.len() >= 2 {
                acc = args[1].clone();
            } else {
                match (0..n).rev().find(|i| !holes.contains(i)) {
                    Some(k) => {
                        acc = items[k].clone();
                        i = k;
                    }
                    None => {
                        return Err(host::type_error(
                            "Reduce of empty array with no initial value",
                        ))
                    }
                }
            }
            while i > 0 {
                i -= 1;
                if holes.contains(&i) {
                    continue;
                }
                acc = host::invoke(
                    &cb,
                    vec![
                        acc,
                        items[i].clone(),
                        Value::Float(i as f64),
                        this_value.clone(),
                    ],
                    None,
                )?;
            }
            Ok(acc)
        }
        "findLast" => {
            let items = array_items(recv);
            let cb = arg0(&args);
            for i in (0..items.len()).rev() {
                let m = host::invoke(
                    &cb,
                    vec![items[i].clone(), Value::Float(i as f64), this_value.clone()],
                    None,
                )?;
                if with_host(|h| h.truthy(&m)) {
                    return Ok(items[i].clone());
                }
            }
            Ok(Value::Undef)
        }
        "findLastIndex" => {
            let items = array_items(recv);
            let cb = arg0(&args);
            for i in (0..items.len()).rev() {
                let m = host::invoke(
                    &cb,
                    vec![items[i].clone(), Value::Float(i as f64), this_value.clone()],
                    None,
                )?;
                if with_host(|h| h.truthy(&m)) {
                    return Ok(Value::Float(i as f64));
                }
            }
            Ok(Value::Float(-1.0))
        }
        // 23.1.3.30: `SortIndexedProperties` collects only the PRESENT elements,
        // and the holes are re-created at the tail — `[3,,1].sort()` is
        // `[1, 3, <1 empty item>]` with own keys `['0','1']`.
        "sort" => {
            let all = array_items(recv);
            let holes = hole_set(recv);
            let mut items: Vec<Value> = all
                .iter()
                .enumerate()
                .filter(|(i, _)| !holes.contains(i))
                .map(|(_, v)| v.clone())
                .collect();
            sort_values(&mut items, args.first())?;
            let present = items.len();
            items.resize(all.len(), Value::Undef);
            with_host(|h| {
                if let Some(JsObj::Array(a)) = h.get_mut(recv) {
                    *a = items;
                }
                h.install_holes(recv, (present..all.len()).collect());
            });
            Ok(this_value.clone())
        }
        // ES2023 change-by-copy: sort a fresh copy, leaving the receiver untouched.
        "toSorted" => {
            let mut items = array_items(recv);
            sort_values(&mut items, args.first())?;
            Ok(with_host(|h| h.new_array(items)))
        }
        "toReversed" => {
            let mut items = array_items(recv);
            items.reverse();
            Ok(with_host(|h| h.new_array(items)))
        }
        "toSpliced" => {
            let mut items = array_items(recv);
            let len = items.len();
            let start = {
                let s = arg_num(&args, 0);
                if s < 0.0 {
                    ((len as f64 + s).max(0.0)) as usize
                } else {
                    (s as usize).min(len)
                }
            };
            let delete = if args.len() >= 2 {
                (arg_num(&args, 1).max(0.0) as usize).min(len - start)
            } else if args.is_empty() {
                0
            } else {
                len - start
            };
            let inserts: Vec<Value> = args.iter().skip(2).cloned().collect();
            items.splice(start..start + delete, inserts);
            Ok(with_host(|h| h.new_array(items)))
        }
        "with" => {
            let mut items = array_items(recv);
            let len = items.len() as i64;
            let rel = arg_num(&args, 0) as i64;
            let idx = if rel < 0 { len + rel } else { rel };
            if idx < 0 || idx >= len {
                return Err(host::range_error(&format!("Invalid index : {rel}")));
            }
            items[idx as usize] = args.get(1).cloned().unwrap_or(Value::Undef);
            Ok(with_host(|h| h.new_array(items)))
        }
        "flat" => {
            // depth defaults to 1; `Infinity` flattens fully. ToIntegerOrInfinity:
            // NaN → 0, otherwise truncate toward zero (negatives act as 0).
            let raw = if args.is_empty() {
                1.0
            } else {
                arg_num(&args, 0)
            };
            let depth = if raw.is_nan() {
                0.0
            } else if raw.is_infinite() {
                raw
            } else {
                raw.trunc()
            };
            let mut out = Vec::new();
            flatten_into(recv, depth, &mut out)?;
            Ok(with_host(|h| h.new_array(out)))
        }
        "keys" => {
            let n = array_len(recv);
            let items: Vec<Value> = (0..n).map(|i| Value::Float(i as f64)).collect();
            Ok(with_host(|h| h.alloc(JsObj::Iter { items, idx: 0 })))
        }
        "values" | "@@iterator" => {
            let items = array_items(recv);
            Ok(with_host(|h| h.alloc(JsObj::Iter { items, idx: 0 })))
        }
        "entries" => {
            let items = array_items(recv);
            let pairs: Vec<Value> = items
                .into_iter()
                .enumerate()
                .map(|(i, v)| with_host(|h| h.new_array(vec![Value::Float(i as f64), v])))
                .collect();
            Ok(with_host(|h| {
                h.alloc(JsObj::Iter {
                    items: pairs,
                    idx: 0,
                })
            }))
        }
        "splice" => array_splice(recv, args),
        // `Array.prototype.toString` IS `join()` with the default separator
        // (23.1.3.36), so it converts each element with `ToString` too — and
        // shares its cycle cut, which is the whole reason it must not call
        // `join_parts` directly: `ToString` of a nested array lands back here.
        "toString" => join_array(recv, ","),
        // An Array inherits from `Object.prototype` too, so the methods it does
        // not override resolve there. `[].hasOwnProperty` already read back as a
        // function through the property path, but CALLING it landed here and
        // threw `is not a function`.
        _ if is_object_builtin_method(name) => object_builtin_method(recv, name, args),
        _ => Err(host::type_error(&format!("{name} is not a function"))),
    }
}

/// `Array.prototype.join` (23.1.3.18) and, with the default separator,
/// `Array.prototype.toString` (23.1.3.36) — one body so both share the cycle
/// cut, which is not optional here: `ToString` of an element that is itself an
/// array re-enters through `toString`, so guarding only `join` left
/// `a=[]; a.push(a); a.join('-')` recursing until the native stack aborted the
/// process. On node v26.7.0 that expression is `""`.
fn join_array(recv: &Value, sep: &str) -> Result<Value, String> {
    if !host::join_stack_push(recv) {
        return Ok(with_host(|h| h.new_str(String::new())));
    }
    let parts = join_parts(&array_items(recv));
    host::join_stack_pop();
    let s = parts?.join(sep);
    Ok(with_host(|h| h.new_str(s)))
}

/// `Array.prototype.join`'s per-element conversion (23.1.3.18 step 4): a
/// `null`/`undefined` element contributes the empty string, every other element
/// is `ToString(element)` — which for an object means invoking its `toString`,
/// so `[{ toString() { return 'x' } }].join()` is `"x"` and not
/// `"[object Object]"`.
///
/// The all-primitive array — the overwhelmingly common one — is rendered under
/// a single host borrow; only an array actually holding an object pays for the
/// re-entrant per-element conversion.
fn join_parts(items: &[Value]) -> Result<Vec<String>, String> {
    let fast = with_host(|h| {
        items
            .iter()
            .map(|x| match x {
                Value::Undef => Some(String::new()),
                _ if h.is_null(x) => Some(String::new()),
                // A SYMBOL element is primitive but has no `ToString`, so it must
                // fall through to the fallible path and throw there:
                // `[Symbol()].join()` is a TypeError on node v26.7.0.
                _ if matches!(h.get(x), Some(JsObj::Symbol { .. })) => None,
                _ if host::is_primitive(h, x) => Some(h.str_of(x)),
                _ => None,
            })
            .collect::<Vec<_>>()
    });
    if fast.iter().all(Option::is_some) {
        return Ok(fast.into_iter().flatten().collect());
    }
    let mut out = Vec::with_capacity(items.len());
    for (x, p) in items.iter().zip(fast) {
        match p {
            Some(s) => out.push(s),
            None => {
                let s = host::to_string_value(x)?;
                out.push(with_host(|h| h.str_of(&s)));
            }
        }
    }
    Ok(out)
}

/// In-place sort of `items` (shared by `sort` and `toSorted`). Stable merge
/// sort — O(n log n) comparisons — with the fallible JS comparator called from
/// the merge step; default order is by the string form of each element.
/// Propagates a comparator error.
///
/// This was an insertion sort, which is O(n²): sorting 200k numbers with a
/// comparator did not finish inside 120s (node v26.7.0: 70ms), and each
/// doubling of the input quadrupled the time — 1k/2k/4k/8k/16k measured at
/// 0.21/0.81/3.39/12.94/51.36s. The comparator contract is unchanged; only the
/// number of times it is called is.
pub(crate) fn sort_values(items: &mut [Value], cmp: Option<&Value>) -> Result<(), String> {
    // 23.1.3.30 step 1: a comparator that is neither `undefined` nor callable is
    // rejected BEFORE any comparison runs. `[2,1].sort(null)` was reaching the
    // invoke path and reporting the generic `null is not a function`.
    let cmp = match cmp {
        Some(Value::Undef) => None,
        Some(v) if !with_host(|h| host::is_callable(h, v)) => {
            let shown = with_host(|h| h.inspect(v));
            return Err(host::type_error(&format!(
                "The comparison function must be either a function or undefined: {shown}"
            )));
        }
        other => other,
    };
    // 23.1.3.30.1 SortIndexedProperties: `undefined` is never handed to the
    // comparator — it sorts to the end after the defined values are ordered.
    // `[3,undefined,1].sort((x,y)=>x-y)` is `[1,3,undefined]` with ONE call on
    // node v26.7.0; the insertion sort called the comparator twice, on
    // `undefined`, and left `[3,undefined,1]`. Every element passed over here
    // is `undefined`, so swapping keeps the defined values in input order.
    let mut defined = 0;
    for i in 0..items.len() {
        if !matches!(items[i], Value::Undef) {
            items.swap(defined, i);
            defined += 1;
        }
    }
    merge_sort(&mut items[..defined], cmp)
}

/// One SortCompare: `> 0` means `b` sorts before `a`. A comparator result runs
/// through ToNumber, so a NaN (or a comparator returning `undefined`) is not
/// `> 0` and the pair keeps its input order.
fn sort_compare(a: &Value, b: &Value, cmp: Option<&Value>) -> Result<f64, String> {
    match cmp {
        Some(cb) => {
            let v = host::invoke(cb, vec![a.clone(), b.clone()], None)?;
            Ok(with_host(|h| h.to_number(&v)))
        }
        None => {
            // 23.1.3.30.2 SortCompare with no comparator: compare the ToString
            // of each element by CODE UNIT (`utf16::cmp_units`), which differs
            // from Rust's `String` order off the BMP.
            let x = with_host(|h| h.str_of(a));
            let y = with_host(|h| h.str_of(b));
            if crate::utf16::cmp_units(&x, &y) == std::cmp::Ordering::Greater {
                Ok(1.0)
            } else {
                Ok(-1.0)
            }
        }
    }
}

/// Bottom-up stable merge sort. Bottom-up rather than recursive so a large
/// array cannot walk the native stack the JS comparator also runs on, and the
/// two buffers are swapped each pass instead of copied back.
fn merge_sort(items: &mut [Value], cmp: Option<&Value>) -> Result<(), String> {
    let n = items.len();
    if n < 2 {
        return Ok(());
    }
    let mut src = items.to_vec();
    let mut dst = src.clone();
    let mut width = 1;
    while width < n {
        let mut lo = 0;
        while lo < n {
            let mid = (lo + width).min(n);
            let hi = (lo + 2 * width).min(n);
            merge(&src[lo..mid], &src[mid..hi], &mut dst[lo..hi], cmp)?;
            lo = hi;
        }
        std::mem::swap(&mut src, &mut dst);
        width *= 2;
    }
    items.clone_from_slice(&src);
    Ok(())
}

/// Merge two sorted runs into `out`. Ties take from `left` first, which is what
/// makes the sort stable — `[{k:1},{k:0},{k:1},{k:0}].sort((x,y)=>x.k-y.k)`
/// keeps the two `k:0` entries in input order, as node does.
fn merge(
    left: &[Value],
    right: &[Value],
    out: &mut [Value],
    cmp: Option<&Value>,
) -> Result<(), String> {
    let (mut i, mut j, mut k) = (0, 0, 0);
    while i < left.len() && j < right.len() {
        if sort_compare(&left[i], &right[j], cmp)? > 0.0 {
            out[k] = right[j].clone();
            j += 1;
        } else {
            out[k] = left[i].clone();
            i += 1;
        }
        k += 1;
    }
    for v in left[i..].iter().chain(&right[j..]) {
        out[k] = v.clone();
        k += 1;
    }
    Ok(())
}

/// Recursively flatten `items` up to `depth` levels into `out`. `depth` is an
/// f64 so `Infinity` (full flatten) and finite counts share one path.
///
/// `flat` has NO cycle cut — unlike `join`, V8 lets it run out of stack, and
/// `a=[1]; a.push(a); a.flat(Infinity)` is `RangeError: Maximum call stack size
/// exceeded` on node v26.7.0. That is reproduced by checking the same native
/// stack floor the VM does, so the answer is a catchable error rather than the
/// `fatal runtime error: stack overflow` abort this used to produce.
/// `FlattenIntoArray` (23.1.3.13.1). Takes the source ARRAY rather than its
/// elements because each level tests `HasProperty` before recursing, so a hole
/// contributes nothing at any depth: `[1,,3].flat()` is the dense `[1, 3]`.
fn flatten_into(src: &Value, depth: f64, out: &mut Vec<Value>) -> Result<(), String> {
    if host::stack_exhausted() {
        return Err(host::stack_overflow_error());
    }
    let items = array_items(src);
    let holes = hole_set(src);
    for (i, it) in items.into_iter().enumerate() {
        if holes.contains(&i) {
            continue;
        }
        let nested = depth > 0.0 && with_host(|h| h.kind_of(&it)) == Some(ObjKind::Array);
        if nested {
            flatten_into(&it, depth - 1.0, out)?;
        } else {
            out.push(it);
        }
    }
    Ok(())
}

fn array_splice(recv: &Value, args: Vec<Value>) -> Result<Value, String> {
    let len = array_len(recv);
    let start = {
        let s = arg_num(&args, 0);
        if s < 0.0 {
            ((len as f64 + s).max(0.0)) as usize
        } else {
            (s as usize).min(len)
        }
    };
    let delete = if args.len() >= 2 {
        (arg_num(&args, 1).max(0.0) as usize).min(len - start)
    } else {
        len - start
    };
    let inserts: Vec<Value> = args.iter().skip(2).cloned().collect();
    let inserted = inserts.len();
    // The receiver's holes shift by (inserted - deleted) past the cut, and the
    // ones inside the cut move into the RETURNED array at their offset there.
    let holes = hole_set(recv);
    let removed = with_host(|h| {
        if let Some(JsObj::Array(items)) = h.get_mut(recv) {
            let removed: Vec<Value> = items.splice(start..start + delete, inserts).collect();
            removed
        } else {
            Vec::new()
        }
    });
    Ok(with_host(|h| {
        h.install_holes(
            recv,
            holes
                .iter()
                .filter_map(|&i| {
                    if i < start {
                        Some(i)
                    } else if i < start + delete {
                        None
                    } else {
                        Some(i - delete + inserted)
                    }
                })
                .collect(),
        );
        let out = h.new_array(removed);
        h.install_holes(
            &out,
            holes
                .iter()
                .filter(|&&i| i >= start && i < start + delete)
                .map(|&i| i - start)
                .collect(),
        );
        out
    }))
}

fn slice_bounds(args: &[Value], len: usize) -> (usize, usize) {
    let norm = |v: f64| -> usize {
        if v < 0.0 {
            ((len as f64 + v).max(0.0)) as usize
        } else {
            (v as usize).min(len)
        }
    };
    let lo = if args.is_empty() || matches!(args[0], Value::Undef) {
        0
    } else {
        norm(arg_num(args, 0))
    };
    let hi = if args.len() < 2 || matches!(args[1], Value::Undef) {
        len
    } else {
        norm(arg_num(args, 1))
    };
    // A start at or past the end (`'World'.slice(2, 1)`) yields the empty range,
    // never a reversed one: JS `slice` clamps `end` up to `start`.
    (lo, hi.max(lo))
}

fn string_method(s: &str, name: &str, args: Vec<Value>) -> Result<Value, String> {
    // Every index-bearing method below counts UTF-16 code units, so they all
    // work off this one decoding rather than off `s.chars()` (code points),
    // which agrees only on the BMP. `@@iterator` is the deliberate exception.
    let u = crate::utf16::Units::of(s);
    match name {
        // `for…of` / spread over a string iterates CODE POINTS, not code units:
        // `[..."𝒳"]` is one element in node even though `"𝒳".length` is 2. This
        // is the one string operation that is specified in chars, so it stays
        // on `s.chars()` on purpose — do not "fix" it to match the others.
        "@@iterator" => {
            let items: Vec<Value> = s.chars().map(|c| new_s(c.to_string())).collect();
            Ok(with_host(|h| h.alloc(JsObj::Iter { items, idx: 0 })))
        }
        "toUpperCase" => Ok(new_s(s.to_uppercase())),
        "toLowerCase" => Ok(new_s(s.to_lowercase())),
        // `toLocaleUpperCase`/`toLocaleLowerCase` (22.1.3.26/22.1.3.24) differ
        // from the plain forms only for the locale-specific mappings (Turkish
        // dotless i, Lithuanian accents); with no locale argument they are the
        // Unicode Default Case Conversion, which is exactly `to_uppercase`/
        // `to_lowercase`. They threw `is not a function` before, so the common
        // no-argument call — the only form this runtime can answer, since it
        // carries no ICU — failed outright rather than agreeing with node.
        // A locale ARGUMENT is accepted and ignored; `'I'.toLocaleLowerCase('tr')`
        // is `'i'` here and `'ı'` in node.
        // `String.prototype.toLocaleString` (22.1.3.27) is `toString` — a string
        // has no locale rendering. Missing it made an ARRAY of strings fail too,
        // since `Array.prototype.toLocaleString` invokes it per element.
        "toLocaleString" => Ok(new_s(s.to_string())),
        "toLocaleUpperCase" => Ok(new_s(s.to_uppercase())),
        "toLocaleLowerCase" => Ok(new_s(s.to_lowercase())),
        // Locale comparison (ASCII approximation of ICU collation): primary by
        // case-folded order, then lowercase sorts before uppercase at a tie.
        "localeCompare" => {
            let other = with_host(|h| h.str_of(&arg0(&args)));
            let (la, lb) = (s.to_lowercase(), other.to_lowercase());
            let r = match la.cmp(&lb) {
                std::cmp::Ordering::Less => -1.0,
                std::cmp::Ordering::Greater => 1.0,
                std::cmp::Ordering::Equal => {
                    let mut t = 0.0;
                    for (ca, cb) in s.chars().zip(other.chars()) {
                        if ca != cb {
                            t = if ca.is_lowercase() { -1.0 } else { 1.0 };
                            break;
                        }
                    }
                    t
                }
            };
            Ok(Value::Float(r))
        }
        // `String.prototype.normalize` (22.1.3.15) — real UAX-15 normalization.
        //
        // This used to return the receiver unchanged and only validate the FORM
        // argument, which made every one of the four forms a no-op: `"Å"` (NFC,
        // one code point) and `"Å"` (NFD, two) stayed distinct under
        // `.normalize()`, so the standard way to compare Unicode text for
        // canonical equivalence silently answered `false`, and `NFKC` never
        // folded a compatibility character (`"ﬁ"` stayed one code point instead
        // of becoming `"fi"`). The tables come from `unicode-normalization`.
        "normalize" => {
            use unicode_normalization::UnicodeNormalization;
            let form = match args.first() {
                Some(v) if !matches!(v, Value::Undef) => with_host(|h| h.str_of(v)),
                _ => "NFC".to_string(),
            };
            let out = match form.as_str() {
                "NFC" => s.nfc().collect::<String>(),
                "NFD" => s.nfd().collect::<String>(),
                "NFKC" => s.nfkc().collect::<String>(),
                "NFKD" => s.nfkd().collect::<String>(),
                _ => {
                    return Err(host::range_error(
                        "The normalization form should be one of NFC, NFD, NFKC, NFKD.",
                    ))
                }
            };
            Ok(new_s(out))
        }
        // ES2024 well-formedness (22.1.3.9 / 22.1.3.29). A `String` here is a
        // Rust `String`, whose `char` type EXCLUDES `U+D800..=U+DFFF`, so every
        // value this runtime can hold is well-formed by construction and
        // `toWellFormed` has nothing to replace. Both answers are therefore
        // exact for every string that survives storage; the one case node
        // answers differently is a surrogate half extracted by `charAt`/`slice`,
        // which is already `U+FFFD` here — the documented lone-surrogate
        // boundary in `utf16`, not a separate gap.
        "isWellFormed" => Ok(Value::Bool(true)),
        "toWellFormed" => Ok(new_s(s.to_string())),
        // The JS `WhiteSpace` set, not Rust's — they differ on `U+FEFF`.
        "trim" => Ok(new_s(crate::utf16::js_trim(s).to_string())),
        "trimStart" => Ok(new_s(crate::utf16::js_trim_start(s).to_string())),
        "trimEnd" => Ok(new_s(crate::utf16::js_trim_end(s).to_string())),
        "toString" | "valueOf" => Ok(new_s(s.to_string())),
        "charAt" => {
            let at = unit_pos(arg_num(&args, 0)).and_then(|i| u.unit_str(i));
            Ok(new_s(at.unwrap_or_default()))
        }
        "at" => {
            let n = arg_num(&args, 0);
            // A negative position counts back from the end; `NaN` is 0. An
            // infinite position is out of range in either direction.
            let i = if n.is_nan() {
                Some(0i64)
            } else if n.is_finite() {
                let i = n.trunc() as i64;
                Some(if i < 0 { i + u.len() as i64 } else { i })
            } else {
                None
            };
            match i
                .and_then(|i| usize::try_from(i).ok())
                .and_then(|i| u.unit_str(i))
            {
                Some(c) => Ok(new_s(c)),
                None => Ok(Value::Undef),
            }
        }
        // `charCodeAt` reports the bare code UNIT — the high surrogate of an
        // astral character, not the character. `codePointAt` looks ahead one
        // unit and reports the whole scalar when the pair is well formed. They
        // agree everywhere on the BMP, which is why they used to share an arm.
        // They also disagree OUT of range: `charCodeAt` yields `NaN` while
        // `codePointAt` yields `undefined` (measured on node v26.7.0).
        "charCodeAt" => {
            let unit = unit_pos(arg_num(&args, 0)).and_then(|i| u.unit(i));
            Ok(Value::Float(unit.map(f64::from).unwrap_or(f64::NAN)))
        }
        "codePointAt" => match unit_pos(arg_num(&args, 0)).and_then(|i| u.code_point(i)) {
            Some(cp) => Ok(Value::Float(f64::from(cp))),
            None => Ok(Value::Undef),
        },
        // The search quartet all honor their optional position argument.
        // `"a&b&c".indexOf("&", 2)` must be 3, not 1 — body-parser's
        // parameterCount walks a query string with exactly that call.
        "indexOf" => {
            let needle = needle_units(&args);
            let from = clamp_pos(arg_num(&args, 1), u.len());
            Ok(Value::Float(
                search_from(u.as_slice(), needle.as_slice(), from)
                    .map(|i| i as f64)
                    .unwrap_or(-1.0),
            ))
        }
        "lastIndexOf" => {
            let needle = needle_units(&args);
            // An absent or NaN position means "search the whole string".
            let n = arg_num(&args, 1);
            let upto = if n.is_nan() {
                u.len()
            } else {
                clamp_pos(n, u.len())
            };
            Ok(Value::Float(
                search_last(u.as_slice(), needle.as_slice(), upto)
                    .map(|i| i as f64)
                    .unwrap_or(-1.0),
            ))
        }
        "includes" => {
            let needle = needle_units(&args);
            let from = clamp_pos(arg_num(&args, 1), u.len());
            Ok(Value::Bool(
                search_from(u.as_slice(), needle.as_slice(), from).is_some(),
            ))
        }
        "startsWith" => {
            let needle = needle_units(&args);
            let from = clamp_pos(arg_num(&args, 1), u.len());
            Ok(Value::Bool(
                u.as_slice()[from..].starts_with(needle.as_slice()),
            ))
        }
        "endsWith" => {
            let needle = needle_units(&args);
            // The 2nd argument is where the string is treated as ENDING.
            let end = if args.len() < 2 || matches!(args[1], Value::Undef) {
                u.len()
            } else {
                clamp_pos(arg_num(&args, 1), u.len())
            };
            Ok(Value::Bool(
                u.as_slice()[..end].ends_with(needle.as_slice()),
            ))
        }
        "slice" => {
            let (lo, hi) = slice_bounds(&args, u.len());
            Ok(new_s(u.slice(lo, hi)))
        }
        "substring" => {
            let mut a = arg_num(&args, 0).max(0.0) as usize;
            let mut b = if args.len() < 2 || matches!(args[1], Value::Undef) {
                u.len()
            } else {
                (arg_num(&args, 1).max(0.0) as usize).min(u.len())
            };
            a = a.min(u.len());
            if a > b {
                std::mem::swap(&mut a, &mut b);
            }
            Ok(new_s(u.slice(a, b)))
        }
        "substr" => {
            // A negative start counts from the end: max(len + start, 0).
            let len = u.len() as i64;
            let mut start = arg_num(&args, 0) as i64;
            if start < 0 {
                start = (len + start).max(0);
            }
            let start = (start as usize).min(u.len());
            let count = if args.len() >= 2 {
                arg_num(&args, 1).max(0.0) as usize
            } else {
                u.len()
            };
            let end = start.saturating_add(count).min(u.len());
            Ok(new_s(u.slice(start, end)))
        }
        "repeat" => {
            let n = arg_num(&args, 0);
            // `RangeError`, not `TypeError`, and the count is named:
            // `"x".repeat(-1)` is `RangeError: Invalid count value: -1`.
            if n < 0.0 || !n.is_finite() {
                return Err(host::range_error(&format!(
                    "Invalid count value: {}",
                    host::fmt_number(n)
                )));
            }
            // The PRODUCT is what V8 bounds, so `''.repeat(2**53)` is legal (and
            // `''`) while `'ab'.repeat(268435445)` is not: measured on node
            // v26.7.0, `'ab'.repeat(268435444).length` is 536870888 and one more
            // is `RangeError: Invalid string length`.
            if n * crate::utf16::len(s) as f64 > host::MAX_STRING_LENGTH as f64 {
                return Err(host::invalid_string_length());
            }
            Ok(new_s(s.repeat(n as usize)))
        }
        "concat" => {
            let mut out = s.to_string();
            for a in &args {
                out.push_str(&with_host(|h| h.str_of(a)));
            }
            Ok(new_s(out))
        }
        "padStart" => Ok(new_s(pad(s, &args, true)?)),
        "padEnd" => Ok(new_s(pad(s, &args, false)?)),
        // Regex-taking string methods: dispatch to the regexp module when the
        // argument is a RegExp; otherwise keep the plain-string behavior.
        "match" => crate::regexp::str_match(s, &arg0(&args)),
        "matchAll" => crate::regexp::str_match_all(s, &arg0(&args)),
        "search" => {
            if is_regexp_arg(&arg0(&args)) {
                crate::regexp::str_search(s, &arg0(&args))
            } else {
                // A string arg is coerced to a (literal) regex; we approximate with
                // a plain substring search, which agrees for non-metacharacter
                // needles.
                let needle = with_host(|h| h.str_of(&arg0(&args)));
                Ok(Value::Float(byte_to_unit_index(s, s.find(&needle))))
            }
        }
        "replace" => {
            let pat = arg0(&args);
            let repl = args.get(1).cloned().unwrap_or(Value::Undef);
            if is_regexp_arg(&pat) {
                crate::regexp::str_replace_regex(s, &pat, &repl, false)
            } else if with_host(|h| host::is_callable(h, &repl)) {
                Ok(new_s(replace_str_fn(
                    s,
                    &with_host(|h| h.str_of(&pat)),
                    &repl,
                    false,
                )?))
            } else {
                let from = with_host(|h| h.str_of(&pat));
                let to = with_host(|h| h.str_of(&repl));
                Ok(new_s(s.replacen(&from, &to, 1)))
            }
        }
        "replaceAll" => {
            let pat = arg0(&args);
            let repl = args.get(1).cloned().unwrap_or(Value::Undef);
            if is_regexp_arg(&pat) {
                crate::regexp::str_replace_regex(s, &pat, &repl, true)
            } else if with_host(|h| host::is_callable(h, &repl)) {
                Ok(new_s(replace_str_fn(
                    s,
                    &with_host(|h| h.str_of(&pat)),
                    &repl,
                    true,
                )?))
            } else {
                let from = with_host(|h| h.str_of(&pat));
                let to = with_host(|h| h.str_of(&repl));
                Ok(new_s(s.replace(&from, &to)))
            }
        }
        "split" => {
            if is_regexp_arg(&arg0(&args)) {
                let limit = args
                    .get(1)
                    .filter(|v| !matches!(v, Value::Undef))
                    .map(|v| with_host(|h| h.to_number(v)) as usize);
                return crate::regexp::str_split_regex(s, &arg0(&args), limit);
            }
            let mut parts: Vec<Value> = if args.is_empty() || matches!(args[0], Value::Undef) {
                vec![new_s(s.to_string())]
            } else {
                let sep = with_host(|h| h.str_of(&args[0]));
                if sep.is_empty() {
                    // `split('')` yields one element per code UNIT, so an astral
                    // character becomes its two surrogate halves.
                    (0..u.len())
                        .filter_map(|i| u.unit_str(i))
                        .map(new_s)
                        .collect()
                } else {
                    s.split(&sep as &str)
                        .map(|p| new_s(p.to_string()))
                        .collect()
                }
            };
            // Optional limit: keep at most `limit` substrings.
            if let Some(lim) = args.get(1).filter(|v| !matches!(v, Value::Undef)) {
                let n = with_host(|h| h.to_number(lim));
                if n.is_finite() && n >= 0.0 {
                    parts.truncate(n as usize);
                }
            }
            Ok(with_host(|h| h.new_array(parts)))
        }
        _ => Err(host::type_error(&format!("{name} is not a function"))),
    }
}

fn new_s(s: String) -> Value {
    with_host(|h| h.new_str(s))
}

/// `ToIntegerOrInfinity(n)` clamped into `0..=len` — the position argument of
/// the `String.prototype` search methods. `NaN` (an absent argument) is `0`.
fn clamp_pos(n: f64, len: usize) -> usize {
    if n.is_nan() || n <= 0.0 {
        0
    } else if n >= len as f64 {
        len
    } else {
        n.trunc() as usize
    }
}

/// `ToIntegerOrInfinity(n)` as a code-unit position, or `None` when there can be
/// no such unit. `NaN` (an absent argument) is 0; a negative or infinite
/// position is out of range — `"abc".charCodeAt(-1)` is `NaN`, not `'a'`.
fn unit_pos(n: f64) -> Option<usize> {
    if n.is_nan() {
        Some(0)
    } else if n < 0.0 || !n.is_finite() {
        None
    } else {
        Some(n.trunc() as usize)
    }
}

/// The search argument of `indexOf`/`includes`/`startsWith`/… as code units, so
/// the needle is compared in the same alphabet the haystack is indexed by.
fn needle_units(args: &[Value]) -> crate::utf16::Units {
    crate::utf16::Units::of(&with_host(|h| h.str_of(&arg0(args))))
}

/// The lowest index `>= from` at which `needle` occurs in `hay`. An empty
/// needle matches at `from` itself, as JS specifies.
fn search_from(hay: &[u16], needle: &[u16], from: usize) -> Option<usize> {
    if needle.is_empty() {
        return Some(from.min(hay.len()));
    }
    if needle.len() > hay.len() {
        return None;
    }
    (from..=hay.len().saturating_sub(needle.len())).find(|&i| &hay[i..i + needle.len()] == needle)
}

/// The highest index `<= upto` at which `needle` occurs in `hay`.
fn search_last(hay: &[u16], needle: &[u16], upto: usize) -> Option<usize> {
    if needle.is_empty() {
        return Some(upto.min(hay.len()));
    }
    if needle.len() > hay.len() {
        return None;
    }
    let last = hay.len() - needle.len();
    (0..=upto.min(last))
        .rev()
        .find(|&i| &hay[i..i + needle.len()] == needle)
}

/// A UTF-8 byte offset reported back to JS as a string position — a UTF-16
/// code-unit index — or `-1` for "not found".
fn byte_to_unit_index(s: &str, byte: Option<usize>) -> f64 {
    match byte {
        Some(b) => crate::utf16::index_of_byte(s, b).get() as f64,
        None => -1.0,
    }
}

fn pad(s: &str, args: &[Value], start: bool) -> Result<String, String> {
    let target_f = arg_num(args, 0);
    let target = if target_f.is_finite() && target_f > 0.0 {
        target_f as usize
    } else {
        0
    };
    // `targetLength` and the padding both count code units: `'𝒳'.padStart(3,'-')`
    // is `'-𝒳'` in node, not `'--𝒳'`.
    let cur = crate::utf16::len(s);
    if cur >= target {
        return Ok(s.to_string());
    }
    let filler = if args.len() >= 2 {
        with_host(|h| h.str_of(&args[1]))
    } else {
        " ".to_string()
    };
    if filler.is_empty() {
        return Ok(s.to_string());
    }
    // Checked only AFTER the two short-circuits, which is the order V8 uses:
    // measured on node v26.7.0, `'ab'.padStart(2**40, '')` is `'ab'` while
    // `'ab'.padStart(536870889, 'x')` is `RangeError: Invalid string length`.
    if target_f > host::MAX_STRING_LENGTH as f64 {
        return Err(host::invalid_string_length());
    }
    let need = target - cur;
    let fill = crate::utf16::Units::of(&filler);
    // The filler repeats and is TRUNCATED to the exact unit count, which can cut
    // a surrogate pair — node yields a lone surrogate there, we yield U+FFFD
    // (see src/utf16.rs).
    let units: Vec<u16> = (0..need)
        .filter_map(|i| fill.unit(i % fill.len()))
        .collect();
    let padding = crate::utf16::to_string_lossy(&units);
    Ok(if start {
        format!("{padding}{s}")
    } else {
        format!("{s}{padding}")
    })
}

/// V8's radix rejection, shared by `Number.prototype.toString` and
/// `BigInt.prototype.toString` — one string, because they are one message and
/// the two sites had drifted apart ("radix must be" vs V8's "radix argument
/// must be").
const RADIX_RANGE: &str = "toString() radix argument must be between 2 and 36";

/// `BigInt.prototype` methods: `toString([radix])`, `valueOf`, `toLocaleString`.
fn bigint_method(b: &num_bigint::BigInt, name: &str, args: Vec<Value>) -> Result<Value, String> {
    match name {
        "toString" => {
            let radix = match args.first() {
                None | Some(Value::Undef) => 10,
                Some(_) => {
                    let t = arg_num(&args, 0).trunc();
                    if !(2.0..=36.0).contains(&t) {
                        return Err(host::range_error(RADIX_RANGE));
                    }
                    t as u32
                }
            };
            Ok(new_s(b.to_str_radix(radix)))
        }
        // `BigInt.prototype.toLocaleString` groups thousands like the Number
        // one does — `(1234567n).toLocaleString()` is `1,234,567` in node, and
        // returning the bare digits made it the only numeric type that skipped
        // grouping. Same en-US-shaped output as `Number.prototype`; the
        // `locales`/`options` arguments are ignored (no ICU here).
        "toLocaleString" => {
            let digits = b.magnitude().to_string();
            let sign = if b.sign() == num_bigint::Sign::Minus {
                "-"
            } else {
                ""
            };
            Ok(new_s(format!("{sign}{}", group_thousands(&digits))))
        }
        "valueOf" => Ok(with_host(|h| h.new_bigint(b.clone()))),
        _ => Err(host::type_error(&format!("{name} is not a function"))),
    }
}

fn number_method(n: f64, name: &str, args: Vec<Value>) -> Result<Value, String> {
    match name {
        "toFixed" => {
            let digits = arg_num(&args, 0);
            if !(0.0..=100.0).contains(&digits.trunc()) {
                return Err(host::range_error(
                    "toFixed() digits argument must be between 0 and 100",
                ));
            }
            Ok(new_s(to_fixed(n, digits as usize)))
        }
        "toExponential" => {
            // `undefined` (or a missing argument) selects the shortest form.
            let f = match args.first() {
                None | Some(Value::Undef) => None,
                Some(_) => {
                    let d = arg_num(&args, 0).trunc();
                    if !(0.0..=100.0).contains(&d) {
                        return Err(host::range_error(
                            "toExponential() argument must be between 0 and 100",
                        ));
                    }
                    Some(d as usize)
                }
            };
            Ok(new_s(to_exponential(n, f)))
        }
        "toString" => {
            // An out-of-range radix THROWS; it does not silently fall back to
            // base 10. `(1).toString(37)` returned "1" here, so a support probe
            // was told every radix worked.
            let radix = match args.first() {
                None | Some(Value::Undef) => 10,
                Some(_) => {
                    let r = arg_num(&args, 0);
                    let t = r.trunc();
                    if !(2.0..=36.0).contains(&t) {
                        return Err(host::range_error(RADIX_RANGE));
                    }
                    t as u32
                }
            };
            if radix == 10 {
                Ok(new_s(host::fmt_number(n)))
            } else {
                Ok(new_s(to_radix(n, radix)))
            }
        }
        "toPrecision" => {
            // `undefined` (or a missing argument) behaves like `toString()`.
            match args.first() {
                None | Some(Value::Undef) => Ok(new_s(host::fmt_number(n))),
                Some(_) => {
                    let p = arg_num(&args, 0).trunc();
                    if !(1.0..=100.0).contains(&p) {
                        return Err(host::range_error(
                            "toPrecision() argument must be between 1 and 100",
                        ));
                    }
                    Ok(new_s(to_precision(n, p as usize)))
                }
            }
        }
        "toLocaleString" => Ok(new_s(to_locale_string(n))),
        "valueOf" => Ok(Value::Float(n)),
        _ => Err(host::type_error(&format!("{name} is not a function"))),
    }
}

/// `Number.prototype.toLocaleString()` with the default locale and options:
/// integer part grouped in threes with `,`, up to 3 fraction digits (rounded
/// half away from zero), trailing fractional zeros dropped. Mirrors V8's default
/// `Intl.NumberFormat().format` output (`(12345.678).toLocaleString()` ⇒
/// `"12,345.678"`; `(1234.5678)` ⇒ `"1,234.568"`). `NaN`, `±Infinity`, and `-0`
/// render as `"NaN"`, `"∞"`/`"-∞"`, and `"-0"`.
fn to_locale_string(n: f64) -> String {
    if n.is_nan() {
        return "NaN".to_string();
    }
    if n.is_infinite() {
        return if n < 0.0 { "-∞" } else { "∞" }.to_string();
    }
    let neg = n.is_sign_negative();
    // Round the magnitude to at most 3 fraction digits, then drop trailing zeros
    // (and a bare trailing point). `to_fixed` rounds half away from zero.
    // `to_fixed` falls back to `ToString` at |x| ≥ 1e21 (spec 21.1.3.3 step 6),
    // which is exponential — and the grouping below then chopped up the
    // exponent, so `(1e21).toLocaleString()` was `1e,+21` instead of node's
    // `1,000,000,000,000,000,000,000`. Expanding the SHORTEST repr is the right
    // source: node groups the shortest decimal form, so `(1e100)
    // .toLocaleString()` is 1 followed by a hundred zeros rather than the exact
    // binary value `1000…159028911…`. (`BigInt(1e100)` is the exact value, a
    // deliberately different rule — see `bigint_ctor`.)
    let fixed = expand_exponential(&to_fixed(n.abs(), 3));
    let trimmed = match fixed.split_once('.') {
        Some(_) => fixed.trim_end_matches('0').trim_end_matches('.'),
        None => fixed.as_str(),
    };
    let (int_part, frac_part) = match trimmed.split_once('.') {
        Some((i, f)) => (i, Some(f)),
        None => (trimmed, None),
    };
    let mut out = String::new();
    if neg {
        out.push('-'); // Intl keeps the sign even for -0.
    }
    out.push_str(&group_thousands(int_part));
    if let Some(f) = frac_part {
        out.push('.');
        out.push_str(f);
    }
    out
}

/// Write a nonnegative decimal string in plain positional form, expanding an
/// `e+NN` exponent into zeros. `"1e+21"` → `"1000000000000000000000"`,
/// `"1.5e+21"` → `"1500000000000000000000"`. A string with no exponent, or a
/// negative exponent (a magnitude below 1, which the caller has already rounded
/// to zero), is returned unchanged.
fn expand_exponential(s: &str) -> String {
    let Some((mantissa, exp)) = s.split_once(['e', 'E']) else {
        return s.to_string();
    };
    let Ok(exp) = exp.trim_start_matches('+').parse::<i32>() else {
        return s.to_string();
    };
    if exp <= 0 {
        return s.to_string();
    }
    let (int_digits, frac_digits) = match mantissa.split_once('.') {
        Some((i, f)) => (i.to_string(), f.to_string()),
        None => (mantissa.to_string(), String::new()),
    };
    let mut digits = int_digits;
    digits.push_str(&frac_digits);
    // The exponent consumes the fractional digits first; whatever is left
    // becomes trailing zeros.
    let zeros = exp as usize - frac_digits.len().min(exp as usize);
    digits.push_str(&"0".repeat(zeros));
    digits
}

/// Insert `,` as a thousands separator into a nonnegative integer digit string.
fn group_thousands(int_part: &str) -> String {
    let bytes = int_part.as_bytes();
    let n = bytes.len();
    let mut out = String::with_capacity(n + n / 3);
    for (i, &b) in bytes.iter().enumerate() {
        if i > 0 && (n - i) % 3 == 0 {
            out.push(',');
        }
        out.push(b as char);
    }
    out
}

/// `Number.prototype.toFixed(f)`: fixed-point with `f` fractional digits, rounding
/// half away from zero on the actual IEEE-754 value (so `(1.005).toFixed(2)` is
/// `"1.00"` because 1.005 is really 1.00499…). The sign of a negative input is
/// preserved even when the rounded magnitude is zero: `(-0.4).toFixed(0) === "-0"`.
///
/// The rounding is done on the value's EXACT decimal expansion (Rust's fixed
/// formatting is exact), not on `x * 10^f` — the latter loses precision for large
/// magnitudes (`(9.999999e20).toFixed(4)` must keep every integer digit).
fn to_fixed(n: f64, f: usize) -> String {
    if !n.is_finite() {
        return host::fmt_number(n);
    }
    // Spec: for |x| ≥ 10^21, toFixed falls back to ToString(x).
    if n.abs() >= 1e21 {
        return host::fmt_number(n);
    }
    let neg = n < 0.0;
    // Exact decimal with guard digits past the rounding position; then round the
    // digit string half-away-from-zero (nonneg operand ⇒ round-half-up).
    let full = format!("{:.*}", f + 25, n.abs());
    let mut body = round_decimal_string(&full, f);
    if neg {
        body.insert(0, '-'); // JS keeps the sign even for "-0" / "-0.00".
    }
    body
}

/// Round the exact decimal string `s` (`"int.frac"`, nonnegative) to `f`
/// fractional digits, half away from zero, propagating carry across the point.
fn round_decimal_string(s: &str, f: usize) -> String {
    let (int_part, frac_part) = s.split_once('.').unwrap_or((s, ""));
    let mut digits: Vec<u8> = int_part
        .bytes()
        .chain(frac_part.bytes())
        .map(|b| b - b'0')
        .collect();
    let point = int_part.len(); // digits before the decimal point
    let keep = point + f; // number of leading digits to keep

    // Round up if the first dropped digit is ≥ 5 (exact-half ⇒ up).
    if digits.get(keep).map(|&d| d >= 5).unwrap_or(false) {
        let mut i = keep;
        loop {
            if i == 0 {
                digits.insert(0, 1);
                // A new leading digit shifts the decimal point right by one.
                return assemble_decimal(&digits, point + 1, f);
            }
            i -= 1;
            if digits[i] == 9 {
                digits[i] = 0;
            } else {
                digits[i] += 1;
                break;
            }
        }
    }
    assemble_decimal(&digits, point, f)
}

/// Reassemble `digits` into `"int.frac"` keeping `f` fractional digits, given that
/// `point` digits precede the decimal point.
fn assemble_decimal(digits: &[u8], point: usize, f: usize) -> String {
    let int_str: String = digits[..point].iter().map(|d| (d + b'0') as char).collect();
    let int_str = int_str.trim_start_matches('0');
    let int_str = if int_str.is_empty() { "0" } else { int_str };
    if f == 0 {
        return int_str.to_string();
    }
    let frac: String = digits[point..point + f]
        .iter()
        .map(|d| (d + b'0') as char)
        .collect();
    format!("{int_str}.{frac}")
}

/// Round the nonnegative finite `a` to `p` significant decimal digits, half away
/// from zero, returning the `p` digits and the decimal exponent `e` such that the
/// value is `0.d…d × 10^(e+1)` (i.e. `d.d…d e±e`). Rust's `{:.*e}` rounds half to
/// EVEN (`(2.5)` at 1 digit would give "2"), but JS rounds half up ("3"), so the
/// exact digits are taken with guard positions and rounded here.
fn round_significant(a: f64, p: usize) -> (String, i32) {
    let sci = format!("{a:.*e}", p - 1 + 25);
    let (mant, exp_str) = sci.split_once('e').expect("LowerExp always has 'e'");
    let mut e: i32 = exp_str.parse().expect("LowerExp exponent is an integer");
    let all: Vec<u8> = mant
        .chars()
        .filter(|c| c.is_ascii_digit())
        .map(|c| c as u8 - b'0')
        .collect();
    let mut s: String = all[..p].iter().map(|d| (d + b'0') as char).collect();
    if all.get(p).map(|&d| d >= 5).unwrap_or(false) {
        // Round the p-digit mantissa up, propagating carry; a carry out of the
        // leading digit (`9.99 → 10`) bumps the decimal exponent by one.
        let mut d: Vec<u8> = all[..p].to_vec();
        let mut i = p;
        loop {
            if i == 0 {
                d.insert(0, 1);
                d.truncate(p);
                e += 1;
                break;
            }
            i -= 1;
            if d[i] == 9 {
                d[i] = 0;
            } else {
                d[i] += 1;
                break;
            }
        }
        s = d.iter().map(|x| (x + b'0') as char).collect();
    }
    (s, e)
}

/// `Number.prototype.toExponential(f)`: one digit before the point and `f` after,
/// with a signed decimal exponent (`(100).toExponential(2) === "1.00e+2"`). With
/// `f` omitted, as many digits as uniquely identify the value are used
/// (`(123456).toExponential() === "1.23456e+5"`). Rounding is half away from zero
/// on the exact value, matching `toPrecision`.
fn to_exponential(n: f64, f: Option<usize>) -> String {
    if !n.is_finite() {
        return host::fmt_number(n);
    }
    let neg = n < 0.0;
    let a = n.abs();
    let (s, e) = if a == 0.0 {
        // Zero has no significant digits: emit "0" padded to the requested width.
        ("0".repeat(f.unwrap_or(0) + 1), 0)
    } else {
        match f {
            Some(f) => round_significant(a, f + 1),
            None => {
                // Shortest round-tripping digits (Rust's `{:e}` is shortest).
                let sci = format!("{a:e}");
                let (mant, exp_str) = sci.split_once('e').expect("LowerExp always has 'e'");
                let digits: String = mant.chars().filter(|c| c.is_ascii_digit()).collect();
                let trimmed = digits.trim_end_matches('0');
                let digits = if trimmed.is_empty() { "0" } else { trimmed };
                (digits.to_string(), exp_str.parse().unwrap_or(0))
            }
        }
    };
    let sign = if e >= 0 { '+' } else { '-' };
    let mag = e.abs();
    let body = if s.len() == 1 {
        format!("{s}e{sign}{mag}")
    } else {
        format!("{}.{}e{sign}{mag}", &s[..1], &s[1..])
    };
    if neg {
        format!("-{body}")
    } else {
        body
    }
}

/// `Number.prototype.toPrecision(p)`: `p` significant digits, switching to
/// exponential form when the decimal exponent `e` satisfies `e < -6` or `e ≥ p`
/// (ECMAScript Number.prototype.toPrecision). Trailing zeros are significant and
/// retained (`(100).toPrecision(5) === "100.00"`).
fn to_precision(n: f64, p: usize) -> String {
    if !n.is_finite() {
        return host::fmt_number(n);
    }
    if n == 0.0 {
        return if p == 1 {
            "0".into()
        } else {
            format!("0.{}", "0".repeat(p - 1))
        };
    }
    let neg = n < 0.0;
    let (s, e) = round_significant(n.abs(), p);
    let pp = p as i32;

    let body = if e < -6 || e >= pp {
        // Exponential: first digit, optional '.rest', signed exponent.
        let sign = if e >= 0 { '+' } else { '-' };
        let mag = e.abs();
        if p == 1 {
            format!("{s}e{sign}{mag}")
        } else {
            format!("{}.{}e{sign}{mag}", &s[..1], &s[1..])
        }
    } else if e >= 0 {
        // e in 0..p-1: (e+1) integer digits, then any remaining as fraction.
        let ip = (e + 1) as usize;
        if ip == p {
            s
        } else {
            format!("{}.{}", &s[..ip], &s[ip..])
        }
    } else {
        // -6 ≤ e < 0: "0." then (−e−1) zeros then all p digits.
        format!("0.{}{}", "0".repeat((-e - 1) as usize), s)
    };
    if neg {
        format!("-{body}")
    } else {
        body
    }
}

/// `Number.prototype.toString(radix)` for radix 2..=36 (radix 10 goes through
/// `fmt_number`). Faithful port of V8's `DoubleToRadixCString`: the integer part
/// is emitted exact, and fractional digits are produced up to the input double's
/// precision (terminating via a ULP-sized `delta`), with round-half-to-even and
/// carry-over back into already-written digits (and into the integer part).
fn to_radix(n: f64, radix: u32) -> String {
    if !n.is_finite() {
        return host::fmt_number(n);
    }
    let digits = b"0123456789abcdefghijklmnopqrstuvwxyz";
    let rf = radix as f64;
    let neg = n < 0.0;
    let value = n.abs();

    let mut integer = value.floor();
    let mut fraction = value - integer;

    // Fraction digits, most-significant first.
    let mut frac: Vec<u8> = Vec::new();
    // Only compute fractional digits down to the input double's precision.
    let mut delta = 0.5 * (next_up(value) - value);
    delta = delta.max(next_up(0.0));
    if fraction >= delta {
        loop {
            // Shift up by one digit.
            fraction *= rf;
            delta *= rf;
            let digit = fraction as usize;
            frac.push(digits[digit]);
            fraction -= digit as f64;
            // Round to even.
            if (fraction > 0.5 || (fraction == 0.5 && (digit & 1) == 1)) && fraction + delta > 1.0 {
                // Carry-over: back-trace already-written fraction digits.
                loop {
                    match frac.pop() {
                        None => {
                            // Carried past the point into the integer part.
                            integer += 1.0;
                            break;
                        }
                        Some(c) => {
                            let d = if c > b'9' {
                                (c - b'a' + 10) as u32
                            } else {
                                (c - b'0') as u32
                            };
                            if d + 1 < radix {
                                frac.push(digits[(d + 1) as usize]);
                                break;
                            }
                            // digit was radix-1: drop it and keep carrying.
                        }
                    }
                }
                break;
            }
            if fraction < delta {
                break;
            }
        }
    }

    // Integer digits, least-significant first (reversed at the end).
    let mut int_out: Vec<u8> = Vec::new();
    // For magnitudes ≥ 2^53, `fmod` loses low bits: pre-fill trailing zeros.
    while v8_exponent(integer / rf) > 0 {
        integer /= rf;
        int_out.push(b'0');
    }
    loop {
        let remainder = integer % rf;
        int_out.push(digits[remainder as usize]);
        integer = (integer - remainder) / rf;
        if integer <= 0.0 {
            break;
        }
    }
    int_out.reverse();

    let mut out: Vec<u8> = Vec::new();
    if neg {
        out.push(b'-');
    }
    out.extend_from_slice(&int_out);
    if !frac.is_empty() {
        out.push(b'.');
        out.extend_from_slice(&frac);
    }
    String::from_utf8(out).unwrap()
}

/// Next representable f64 above `x` (`x` finite, `x ≥ 0`) — V8's `NextDouble`.
fn next_up(x: f64) -> f64 {
    f64::from_bits(x.to_bits() + 1)
}

/// V8's `Double::Exponent`: the binary exponent of the significand-scaled value
/// (`> 0` iff |x| ≥ 2^53). Used to detect integers past `fmod`'s exact range.
fn v8_exponent(x: f64) -> i32 {
    let biased = ((x.to_bits() >> 52) & 0x7ff) as i32;
    if biased == 0 {
        -1074 // denormal
    } else {
        biased - 1075
    }
}

// ══ Map / Set / Symbol / generator methods ═══════════════════════════════════

/// `Map.prototype.set` step 6 and `Set.prototype.add` step 4: a key of `-0` is
/// STORED as `+0`. `map_key` already treats the two as one key (SameValueZero),
/// but the value kept alongside it is what iteration and `console.log` report,
/// and node shows `0` there — `new Map().set(-0, 1)` renders `Map(1) { 0 => 1 }`.
fn normalize_zero_key(v: Value) -> Value {
    match v {
        Value::Float(f) if f == 0.0 && f.is_sign_negative() => Value::Float(0.0),
        other => other,
    }
}

fn map_method(recv: &Value, name: &str, args: Vec<Value>) -> Result<Value, String> {
    match name {
        "get" => {
            let key = with_host(|h| host::map_key(h, &arg0(&args)));
            Ok(with_host(|h| match h.get(recv) {
                Some(JsObj::Map { entries, .. }) => entries
                    .get(&key)
                    .map(|(_, v)| v.clone())
                    .unwrap_or(Value::Undef),
                _ => Value::Undef,
            }))
        }
        "set" => {
            let kv = normalize_zero_key(arg0(&args));
            let vv = args.get(1).cloned().unwrap_or(Value::Undef);
            reject_non_object_weak_key(recv, &kv, "WeakMap")?;
            let key = with_host(|h| host::map_key(h, &kv));
            with_host(|h| {
                if let Some(JsObj::Map { entries, .. }) = h.get_mut(recv) {
                    entries.insert(key, (kv, vv));
                }
            });
            Ok(recv.clone())
        }
        "has" => {
            let key = with_host(|h| host::map_key(h, &arg0(&args)));
            Ok(Value::Bool(with_host(
                |h| matches!(h.get(recv), Some(JsObj::Map { entries, .. }) if entries.contains_key(&key)),
            )))
        }
        "delete" => {
            let key = with_host(|h| host::map_key(h, &arg0(&args)));
            Ok(Value::Bool(with_host(|h| match h.get_mut(recv) {
                Some(JsObj::Map { entries, .. }) => entries.shift_remove(&key).is_some(),
                _ => false,
            })))
        }
        "clear" => {
            with_host(|h| {
                if let Some(JsObj::Map { entries, .. }) = h.get_mut(recv) {
                    entries.clear();
                }
            });
            Ok(Value::Undef)
        }
        "forEach" => {
            let cb = arg0(&args);
            let pairs: Vec<(Value, Value)> = with_host(|h| match h.get(recv) {
                Some(JsObj::Map { entries, .. }) => entries.values().cloned().collect(),
                _ => Vec::new(),
            });
            for (k, v) in pairs {
                host::invoke(&cb, vec![v, k, recv.clone()], None)?;
            }
            Ok(Value::Undef)
        }
        "keys" | "values" | "entries" | "@@iterator" => {
            let items: Vec<Value> = with_host(|h| {
                let pairs: Vec<(Value, Value)> = match h.get(recv) {
                    Some(JsObj::Map { entries, .. }) => entries.values().cloned().collect(),
                    _ => Vec::new(),
                };
                pairs
                    .into_iter()
                    .map(|(k, v)| match name {
                        "keys" => k,
                        "values" => v,
                        _ => h.new_array(vec![k, v]), // entries + @@iterator
                    })
                    .collect()
            });
            Ok(with_host(|h| h.alloc(JsObj::Iter { items, idx: 0 })))
        }
        _ => Err(host::type_error(&format!("map.{name} is not a function"))),
    }
}

/// A weak collection can only hold objects (and unregistered symbols) — a
/// primitive key is a `TypeError`, which is how packages probe for weak support.
fn reject_non_object_weak_key(recv: &Value, key: &Value, kind: &str) -> Result<(), String> {
    let weak = with_host(|h| {
        matches!(
            h.get(recv),
            Some(JsObj::Map { weak: true, .. }) | Some(JsObj::Set { weak: true, .. })
        )
    });
    if !weak {
        return Ok(());
    }
    let is_object = with_host(|h| match key {
        Value::Obj(_) => !h.is_null(key) && h.as_str(key).is_none() && h.as_bigint(key).is_none(),
        _ => false,
    });
    if is_object {
        return Ok(());
    }
    Err(host::type_error(if kind == "WeakMap" {
        "Invalid value used as weak map key"
    } else {
        "Invalid value used in weak set"
    }))
}

fn set_method(recv: &Value, name: &str, args: Vec<Value>) -> Result<Value, String> {
    match name {
        "add" => {
            let vv = normalize_zero_key(arg0(&args));
            reject_non_object_weak_key(recv, &vv, "WeakSet")?;
            let key = with_host(|h| host::map_key(h, &vv));
            with_host(|h| {
                if let Some(JsObj::Set { entries, .. }) = h.get_mut(recv) {
                    entries.insert(key, vv);
                }
            });
            Ok(recv.clone())
        }
        "has" => {
            let key = with_host(|h| host::map_key(h, &arg0(&args)));
            Ok(Value::Bool(with_host(
                |h| matches!(h.get(recv), Some(JsObj::Set { entries, .. }) if entries.contains_key(&key)),
            )))
        }
        "delete" => {
            let key = with_host(|h| host::map_key(h, &arg0(&args)));
            Ok(Value::Bool(with_host(|h| match h.get_mut(recv) {
                Some(JsObj::Set { entries, .. }) => entries.shift_remove(&key).is_some(),
                _ => false,
            })))
        }
        "clear" => {
            with_host(|h| {
                if let Some(JsObj::Set { entries, .. }) = h.get_mut(recv) {
                    entries.clear();
                }
            });
            Ok(Value::Undef)
        }
        "forEach" => {
            let cb = arg0(&args);
            let vals: Vec<Value> = with_host(|h| match h.get(recv) {
                Some(JsObj::Set { entries, .. }) => entries.values().cloned().collect(),
                _ => Vec::new(),
            });
            for v in vals {
                host::invoke(&cb, vec![v.clone(), v, recv.clone()], None)?;
            }
            Ok(Value::Undef)
        }
        "keys" | "values" | "entries" | "@@iterator" => {
            let items: Vec<Value> = with_host(|h| {
                let vals: Vec<Value> = match h.get(recv) {
                    Some(JsObj::Set { entries, .. }) => entries.values().cloned().collect(),
                    _ => Vec::new(),
                };
                if name == "entries" {
                    vals.into_iter()
                        .map(|v| h.new_array(vec![v.clone(), v]))
                        .collect()
                } else {
                    vals
                }
            });
            Ok(with_host(|h| h.alloc(JsObj::Iter { items, idx: 0 })))
        }
        _ => Err(host::type_error(&format!("set.{name} is not a function"))),
    }
}

fn generator_method(recv: &Value, name: &str, args: Vec<Value>) -> Result<Value, String> {
    // An `async function*` object's methods return PROMISES of the record, and
    // its body has to be driven through the await-aware stepper (a plain
    // `gen_resume` would surface an internal `await` suspension as a bogus yield).
    if host::is_async_generator(recv) {
        // All three go through `[[AsyncGeneratorQueue]]` (ECMA-262 27.6.3.6):
        // `.return`/`.throw` must wait behind a `.next()` that is still
        // suspended on an internal `await`, or that `.next()` would report
        // `{done: true}` for a value the body had not yet reached. An uncaught
        // `.throw(e)` rejects the returned promise; it does not throw here.
        return match name {
            "next" => Ok(host::async_gen_enqueue(
                recv,
                host::GenReq::Next(arg0(&args)),
            )),
            "return" => Ok(host::async_gen_enqueue(
                recv,
                host::GenReq::Return(arg0(&args)),
            )),
            "throw" => Ok(host::async_gen_enqueue(
                recv,
                host::GenReq::Throw(arg0(&args)),
            )),
            "@@asyncIterator" => Ok(recv.clone()),
            _ => Err(host::type_error(&format!(
                "asyncGenerator.{name} is not a function"
            ))),
        };
    }
    match name {
        "next" => {
            let send = arg0(&args);
            match host::gen_resume(recv, send)? {
                host::GenStep::Yield(v) => Ok(iter_result(v, false)),
                host::GenStep::Done(v) => Ok(iter_result(v, true)),
            }
        }
        "return" => {
            // Resume with an injected return so any pending `finally` runs; the
            // completion may itself be a `finally` yield (not-done) or the value.
            match host::gen_return(recv, arg0(&args))? {
                host::GenStep::Yield(v) => Ok(iter_result(v, false)),
                host::GenStep::Done(v) => Ok(iter_result(v, true)),
            }
        }
        "throw" => {
            // Inject a throw at the suspension point: an enclosing `try/catch` in
            // the body can handle it (and any `finally` runs); otherwise it
            // propagates to the caller.
            match host::gen_throw(recv, arg0(&args))? {
                host::GenStep::Yield(v) => Ok(iter_result(v, false)),
                host::GenStep::Done(v) => Ok(iter_result(v, true)),
            }
        }
        _ => Err(host::type_error(&format!(
            "generator.{name} is not a function"
        ))),
    }
}

/// A `{ value, done }` iterator-result object.
fn iter_result(value: Value, done: bool) -> Value {
    with_host(|h| {
        let mut m: IndexMap<String, Value> = IndexMap::new();
        m.insert("value".into(), value);
        m.insert("done".into(), Value::Bool(done));
        h.new_object(m)
    })
}

/// Built-in iterator object (`arr.values()`, `arr[Symbol.iterator]()`): a lazy
/// cursor over a materialized item list.
fn iter_method(recv: &Value, name: &str, args: Vec<Value>) -> Result<Value, String> {
    match name {
        "next" => {
            let step = with_host(|h| {
                if let Some(JsObj::Iter { items, idx }) = h.get_mut(recv) {
                    if *idx < items.len() {
                        let v = items[*idx].clone();
                        *idx += 1;
                        return Some(v);
                    }
                }
                None
            });
            Ok(match step {
                Some(v) => iter_result(v, false),
                None => iter_result(Value::Undef, true),
            })
        }
        "return" => {
            // Exhaust the cursor and report done.
            with_host(|h| {
                if let Some(JsObj::Iter { items, idx }) = h.get_mut(recv) {
                    *idx = items.len();
                }
            });
            Ok(iter_result(arg0(&args), true))
        }
        // An iterator is its own iterable.
        "@@iterator" => Ok(recv.clone()),
        _ => Err(host::type_error(&format!(
            "iterator.{name} is not a function"
        ))),
    }
}

fn symbol_method(recv: &Value, name: &str, _args: Vec<Value>) -> Result<Value, String> {
    match name {
        "toString" => Ok(with_host(|h| {
            let s = h.str_of(recv);
            h.new_str(s)
        })),
        _ => Err(host::type_error(&format!(
            "symbol.{name} is not a function"
        ))),
    }
}

// ══ Object.* prototype helpers, `in`, deep clone ═════════════════════════════

fn object_create(args: Vec<Value>) -> Result<Value, String> {
    let proto = arg0(&args);
    // 20.1.2.2 step 1: the prototype must be an Object or exactly `null`.
    // `undefined` is NOT accepted — measured on node v26.7.0,
    // `Object.create(undefined)` is
    // `TypeError: Object prototype may only be an Object or null: undefined`,
    // where node-js quietly built a normal object.
    reject_bad_prototype(&proto)?;
    let obj = with_host(|h| h.new_object(IndexMap::new()));
    // `set_proto` records a null proto as an explicit null-prototype object.
    with_host(|h| h.set_proto(&obj, proto));
    // Optional second arg: a property-descriptor map.
    if let Some(descs) = args.get(1).filter(|d| !matches!(d, Value::Undef)) {
        let entries: Vec<(String, Value)> = with_host(|h| match h.get(descs) {
            Some(JsObj::Object(p)) => p.iter().map(|(k, v)| (k.clone(), v.clone())).collect(),
            _ => Vec::new(),
        });
        for (k, d) in entries {
            apply_descriptor(&obj, &k, &d);
        }
    }
    Ok(obj)
}

/// The enumerable method names of a builtin `<Ctor>.prototype` namespace that
/// supports being copied via `mixin`/`getOwnPropertyNames`. Currently only
/// `EventEmitter.prototype` (the one express mixes onto its app function).
fn builtin_proto_method_names(ns: &str) -> Option<&'static [&'static str]> {
    match ns {
        "EventEmitter.prototype" => Some(crate::stdlib::events::METHODS),
        _ => None,
    }
}

/// The own SYMBOL-keyed property keys of `v` as symbol values. A Proxy's come
/// from its `ownKeys` trap (the symbol half of the same list the string keys are
/// filtered out of); every other receiver answers from its property map.
fn proxy_or_own_symbol_keys(v: &Value) -> Result<Vec<Value>, String> {
    if let Some(keys) = crate::proxy::own_keys(v)? {
        return Ok(keys
            .iter()
            .filter(|k| host::is_symbol_key(k))
            .map(|k| crate::proxy::key_value(k))
            .collect());
    }
    Ok(with_host(|h| h.own_symbol_keys(v)))
}

/// `[[DefineOwnProperty]]` reachable from `crate::proxy`'s no-trap forward.
pub fn define_property_pub(obj: &Value, key: Value, desc: Value) -> Result<Value, String> {
    object_define_property(vec![obj.clone(), key, desc])
}

/// `[[GetOwnProperty]]` reachable from `crate::proxy`'s no-trap forward.
pub fn own_descriptor_pub(obj: &Value, key: Value) -> Result<Value, String> {
    object_get_own_descriptor(vec![obj.clone(), key])
}

fn object_define_property(args: Vec<Value>) -> Result<Value, String> {
    let obj = arg0(&args);
    // A Proxy defines through its `defineProperty` trap; the target it forwards
    // to is where the ordinary path below finally runs.
    if with_host(|h| h.kind_of(&obj)) == Some(ObjKind::Proxy) {
        let key = with_host(|h| h.property_key(&args.get(1).cloned().unwrap_or(Value::Undef)));
        let desc = args.get(2).cloned().unwrap_or(Value::Undef);
        if !with_host(|h| is_object_like(h, &desc)) {
            return Err(host::type_error(&format!(
                "Property description must be an object: {}",
                with_host(|h| h.str_of(&desc))
            )));
        }
        crate::proxy::define_property(&obj, &key, &desc)?;
        return Ok(obj);
    }
    // 20.1.2.4 steps 1-3, both of which node-js skipped entirely: a non-object
    // target and a non-object descriptor each throw before anything is written.
    if !with_host(|h| is_object_like(h, &obj)) {
        return Err(host::type_error(
            "Object.defineProperty called on non-object",
        ));
    }
    let desc = args.get(2).cloned().unwrap_or(Value::Undef);
    if !with_host(|h| is_object_like(h, &desc)) {
        return Err(host::type_error(&format!(
            "Property description must be an object: {}",
            with_host(|h| h.str_of(&desc))
        )));
    }
    let key = with_host(|h| h.property_key(&args.get(1).cloned().unwrap_or(Value::Undef)));
    apply_descriptor(&obj, &key, &desc);
    Ok(obj)
}

/// Whether `v` is an Object in the language sense — anything `typeof` calls
/// `"object"` (bar `null`) or `"function"`. Used by the argument checks that
/// distinguish "an object" from a primitive.
fn is_object_like(h: &host::JsHost, v: &Value) -> bool {
    matches!(v, Value::Obj(_)) && !h.is_null(v) && !host::is_primitive(h, v)
}

/// `RequireObjectCoercible(v)` — 7.2.1. The check in front of every `ToObject`,
/// which node-js was missing on the whole `Object.keys`/`values`/`entries`/
/// `getOwnPropertyNames`/`getOwnPropertySymbols`/`getOwnPropertyDescriptor`/
/// `assign` family: each returned an empty result for `null` where node v26.7.0
/// throws `TypeError: Cannot convert undefined or null to object`. A PRIMITIVE
/// is coercible and keeps working (`Object.keys(1)` is `[]`).
fn require_object_coercible(v: &Value) -> Result<(), String> {
    if with_host(|h| matches!(v, Value::Undef) || h.is_null(v)) {
        return Err(host::type_error(
            "Cannot convert undefined or null to object",
        ));
    }
    Ok(())
}

/// 10.1.2 / 20.1.2.2 step 1: reject a `[[Prototype]]` that is neither an Object
/// nor `null`, with V8's wording. Measured on node v26.7.0:
/// `Object.create("s")` is
/// `TypeError: Object prototype may only be an Object or null: s`.
fn reject_bad_prototype(proto: &Value) -> Result<(), String> {
    if with_host(|h| h.is_null(proto) || is_object_like(h, proto)) {
        return Ok(());
    }
    Err(host::type_error(&format!(
        "Object prototype may only be an Object or null: {}",
        with_host(|h| h.str_of(proto))
    )))
}

/// Apply a `{ value | get | set }` descriptor object to `obj[key]`.
///
/// Per ECMAScript `ToPropertyDescriptor`, an omitted `writable`/`enumerable`/
/// `configurable` field defaults to **false** — which is why a `defineProperty`
/// data property is invisible to `Object.keys` unless the caller opts in. That
/// asymmetry against plain assignment is the whole reason the attribute table
/// exists.
fn apply_descriptor(obj: &Value, key: &str, desc: &Value) {
    let (value, get, set, attrs) = with_host(|h| match h.get(desc) {
        Some(JsObj::Object(p)) => {
            let flag = |n: &str| p.get(n).map(|v| h.truthy(v)).unwrap_or(false);
            (
                p.get("value").cloned(),
                p.get("get").cloned(),
                p.get("set").cloned(),
                host::PropAttrs {
                    writable: flag("writable"),
                    enumerable: flag("enumerable"),
                    configurable: flag("configurable"),
                },
            )
        }
        _ => (None, None, None, host::PropAttrs::default()),
    });
    with_host(|h| h.set_prop_attrs(obj, key, attrs));
    if get.is_some() || set.is_some() {
        with_host(|h| h.set_accessor(obj, key, get, set));
    } else if let Some(v) = value {
        // A function/class receiver stores its own props in the fn-prop side table
        // (express `mixin(app, proto)` defines methods onto the `app` *function*).
        if matches!(
            with_host(|h| h.get(obj).cloned()),
            Some(JsObj::Func(_)) | Some(JsObj::Class(_))
        ) {
            with_host(|h| h.set_fn_prop(obj, key, v));
        } else if let (Some(ObjKind::Array), Ok(i)) =
            (with_host(|h| h.kind_of(obj)), key.parse::<usize>())
        {
            // An array's index keys ARE its elements, and defining one past the
            // end grows the array with holes in between (10.4.2.1). This whole
            // branch used to be missing: `Object.defineProperty(arr, 1, {value})`
            // wrote into the ordinary property map an array does not have, so it
            // was a silent no-op.
            with_host(|h| {
                let old = match h.get(obj) {
                    Some(JsObj::Array(items)) => items.len(),
                    _ => 0,
                };
                if let Some(JsObj::Array(items)) = h.get_mut(obj) {
                    if i >= old {
                        items.resize(i + 1, Value::Undef);
                    }
                    items[i] = v;
                }
                if i > old {
                    h.mark_hole_range(obj, old..i);
                }
                h.clear_hole(obj, i);
            });
        } else {
            with_host(|h| {
                if let Some(JsObj::Object(p)) = h.get_mut(obj) {
                    p.insert(key.to_string(), v);
                    host::canonicalize_own_keys(p);
                }
            });
        }
    }
}

/// `Object.defineProperties(obj, descriptorMap)`.
fn object_define_properties(args: Vec<Value>) -> Result<Value, String> {
    let obj = arg0(&args);
    let descs = args.get(1).cloned().unwrap_or(Value::Undef);
    let entries: Vec<(String, Value)> = with_host(|h| match h.get(&descs) {
        Some(JsObj::Object(p)) => p.iter().map(|(k, v)| (k.clone(), v.clone())).collect(),
        _ => Vec::new(),
    });
    for (k, d) in entries {
        apply_descriptor(&obj, &k, &d);
    }
    Ok(obj)
}

fn object_get_own_descriptor(args: Vec<Value>) -> Result<Value, String> {
    let obj = arg0(&args);
    require_object_coercible(&obj)?;
    let key = with_host(|h| h.property_key(&args.get(1).cloned().unwrap_or(Value::Undef)));
    if with_host(|h| h.kind_of(&obj)) == Some(ObjKind::Proxy) {
        return Ok(crate::proxy::get_own_descriptor(&obj, &key)?.unwrap_or(Value::Undef));
    }
    // A method read off an enumerable builtin prototype (`EventEmitter.prototype`)
    // yields a `{ value: <method thunk> }` data descriptor so `mixin` can copy it.
    if let Some(JsObj::Builtin(ns)) = with_host(|h| h.get(&obj).cloned()) {
        if let Some(names) = builtin_proto_method_names(&ns) {
            if names.contains(&key.as_str()) {
                return Ok(with_host(|h| {
                    let thunk = h.alloc(JsObj::Builtin(format!(
                        "@proto:{}:{key}",
                        ns.trim_end_matches(".prototype")
                    )));
                    let mut m: IndexMap<String, Value> = IndexMap::new();
                    m.insert("value".into(), thunk);
                    m.insert("writable".into(), Value::Bool(true));
                    m.insert("enumerable".into(), Value::Bool(true));
                    m.insert("configurable".into(), Value::Bool(true));
                    h.new_object(m)
                }));
            }
        }
    }
    // Accessor descriptor?
    if let Some((get, set)) = with_host(|h| h.own_accessor(&obj, &key)) {
        return Ok(with_host(|h| {
            let a = h.prop_attrs(&obj, &key);
            let mut m: IndexMap<String, Value> = IndexMap::new();
            m.insert("get".into(), get.unwrap_or(Value::Undef));
            m.insert("set".into(), set.unwrap_or(Value::Undef));
            m.insert("enumerable".into(), Value::Bool(a.enumerable));
            m.insert("configurable".into(), Value::Bool(a.configurable));
            h.new_object(m)
        }));
    }
    let val = with_host(|h| match h.get(&obj) {
        // A Buffer's own properties are exactly its byte indices, read out of the
        // hidden `@@bytes` slot; `length`/`byteLength` are internal bookkeeping
        // that V8 keeps on the prototype, so they own no descriptor.
        Some(JsObj::Object(p))
            if p.get("@@native").map(|t| h.str_of(t)).as_deref() == Some("Buffer") =>
        {
            match (
                p.get("@@bytes").and_then(|b| h.get(b)),
                key.parse::<usize>(),
            ) {
                (Some(JsObj::Array(items)), Ok(i)) => items.get(i).cloned(),
                _ => None,
            }
        }
        Some(JsObj::Object(p)) => p.get(&key).cloned(),
        // An array's index keys read the elements; `length` is the exotic own
        // property; anything else is an ordinary own key in the side table.
        Some(JsObj::Array(items)) => match key.parse::<usize>() {
            // An ELIDED index owns no property at all, so it has no descriptor.
            Ok(i) if h.is_hole(&obj, i) => None,
            Ok(i) => items.get(i).cloned(),
            Err(_) if key == "length" => Some(Value::Float(items.len() as f64)),
            Err(_) => h.fn_prop(&obj, &key),
        },
        // A function/class own prop lives in the fn-prop side table.
        Some(JsObj::Func(_)) | Some(JsObj::Class(_)) => h.fn_prop(&obj, &key),
        _ => None,
    });
    match val {
        Some(v) => Ok(with_host(|h| {
            let a = h.prop_attrs(&obj, &key);
            let mut m: IndexMap<String, Value> = IndexMap::new();
            m.insert("value".into(), v);
            m.insert("writable".into(), Value::Bool(a.writable));
            m.insert("enumerable".into(), Value::Bool(a.enumerable));
            m.insert("configurable".into(), Value::Bool(a.configurable));
            h.new_object(m)
        })),
        None => Ok(Value::Undef),
    }
}

/// `Object.getOwnPropertyDescriptors(obj)` — the descriptor of every own string
/// key, keyed by name. `Object.create(proto, getOwnPropertyDescriptors(src))` is
/// the standard "clone with accessors intact" idiom, so this must agree
/// key-for-key with `getOwnPropertyNames`.
fn object_get_own_descriptors(args: Vec<Value>) -> Result<Value, String> {
    let obj = arg0(&args);
    let names = object_keys(vec![obj.clone()], 3)?;
    let keys: Vec<String> = with_host(|h| match h.get(&names) {
        Some(JsObj::Array(items)) => items.iter().map(|k| h.str_of(k)).collect(),
        _ => Vec::new(),
    });
    let mut out: IndexMap<String, Value> = IndexMap::new();
    for k in keys {
        let ks = with_host(|h| h.new_str(k.clone()));
        let d = object_get_own_descriptor(vec![obj.clone(), ks])?;
        if !matches!(d, Value::Undef) {
            out.insert(k, d);
        }
    }
    Ok(with_host(|h| h.new_object(out)))
}

/// `key in obj` respecting the prototype chain. Reports a `Result` because a
/// Proxy's `has` trap is user code and may throw.
pub fn has_property(obj: &Value, key: &str) -> Result<bool, String> {
    if let Some(b) = crate::proxy::has(obj, key)? {
        return Ok(b);
    }
    Ok(has_property_ordinary(obj, key))
}

/// `[[HasProperty]]` for every non-Proxy receiver.
fn has_property_ordinary(obj: &Value, key: &str) -> bool {
    // `key in <builtin namespace/prototype>`: membership matches what a property
    // read would yield. `String.prototype.indexOf` (and the rest of the builtin
    // prototype methods) resolve as callable thunks via `namespace_property`, so
    // `'indexOf' in String.prototype` must report true (get-intrinsic probes this
    // with the `in` operator before reading the intrinsic).
    if let Some(JsObj::Builtin(ns)) = with_host(|h| h.get(obj).cloned()) {
        return !matches!(namespace_property(&ns, key), Value::Undef);
    }
    // An integer index of a typed array / Buffer is an own property, and lives
    // in the hidden element array rather than the property map — the same
    // question `hasOwnProperty` answers, through the same helper. Only a hit
    // short-circuits: a non-index key like `'length'` must still fall through
    // to the ordinary chain lookup below.
    if crate::stdlib::typedarray::has_index(obj, key) == Some(true) {
        return true;
    }
    if with_host(|h| host::lookup_chain(h, obj, key)).is_some() {
        return true;
    }
    if with_host(|h| host::lookup_accessor(h, obj, key)).is_some() {
        return true;
    }
    with_host(|h| match h.get(obj) {
        Some(JsObj::Object(p)) => p.contains_key(key),
        Some(JsObj::Array(items)) => {
            key == "length"
                || key
                    .parse::<usize>()
                    .map(|i| i < items.len() && !h.is_hole(obj, i))
                    .unwrap_or(false)
                // A non-index own property (`arr.foo`, `arr[sym]`) lives in the
                // side table, and `in` must see it.
                || h.fn_prop(obj, key).is_some()
        }
        Some(JsObj::Func(_)) | Some(JsObj::Class(_)) => h.fn_prop(obj, key).is_some(),
        _ => false,
    })
}

/// `structuredClone` — a deep copy of plain data (objects/arrays/primitives).
/// `structuredClone` — the HTML structured-clone algorithm's shape: a deep copy
/// that preserves the *reference graph*. Two properties pointing at the same
/// object clone to two properties pointing at the same clone, and a cycle clones
/// to a cycle instead of recursing forever. `seen` maps each source heap index
/// to its clone, which is what buys both.
pub(crate) fn deep_clone(v: &Value) -> Value {
    deep_clone_seen(v, &mut std::collections::HashMap::new())
}

fn deep_clone_seen(v: &Value, seen: &mut std::collections::HashMap<u32, Value>) -> Value {
    let idx = match v {
        Value::Obj(i) => *i,
        _ => return v.clone(),
    };
    if let Some(done) = seen.get(&idx) {
        return done.clone();
    }
    match with_host(|h| h.get(v).cloned()) {
        Some(JsObj::Array(items)) => {
            // Register the (empty) clone BEFORE recursing so a self-reference
            // resolves to it.
            let out = with_host(|h| h.new_array(Vec::new()));
            seen.insert(idx, out.clone());
            let cloned: Vec<Value> = items.iter().map(|x| deep_clone_seen(x, seen)).collect();
            with_host(|h| {
                if let Some(JsObj::Array(a)) = h.get_mut(&out) {
                    *a = cloned;
                }
                // A sparse source clones to an equally sparse array: the clone
                // walks own properties, so a hole is nothing to copy.
                h.copy_holes(v, &out, Some);
            });
            out
        }
        Some(JsObj::Object(props)) => {
            let out = with_host(|h| h.new_object(IndexMap::new()));
            seen.insert(idx, out.clone());
            let cloned: IndexMap<String, Value> = props
                .iter()
                .map(|(k, val)| (k.clone(), deep_clone_seen(val, seen)))
                .collect();
            with_host(|h| {
                if let Some(JsObj::Object(p)) = h.get_mut(&out) {
                    *p = cloned;
                }
                // A native exotic (Buffer, typed array, …) keeps its prototype so
                // the clone passes the same brand checks as the source.
                if let Some(p) = h.proto_of(v) {
                    h.set_proto(&out, p);
                }
                h.copy_prop_attrs(v, &out);
            });
            out
        }
        // Map/Set are structured types: clone the entries, keep the kind.
        Some(JsObj::Map { entries, weak }) => {
            let out = with_host(|h| {
                h.alloc(JsObj::Map {
                    entries: IndexMap::new(),
                    weak,
                })
            });
            seen.insert(idx, out.clone());
            let pairs: Vec<(Value, Value)> = entries.values().cloned().collect();
            for (k, val) in pairs {
                let ck = deep_clone_seen(&k, seen);
                let cv = deep_clone_seen(&val, seen);
                let _ = map_method(&out, "set", vec![ck, cv]);
            }
            out
        }
        Some(JsObj::Set { entries, weak }) => {
            let out = with_host(|h| {
                h.alloc(JsObj::Set {
                    entries: IndexMap::new(),
                    weak,
                })
            });
            seen.insert(idx, out.clone());
            let vals: Vec<Value> = entries.values().cloned().collect();
            for x in vals {
                let cx = deep_clone_seen(&x, seen);
                let _ = set_method(&out, "add", vec![cx]);
            }
            out
        }
        // Strings/BigInts/RegExps/dates are immutable-enough to share, and a
        // function is not cloneable at all (Node throws DataCloneError; node-js
        // passes it through rather than inventing that error class).
        _ => v.clone(),
    }
}

// ══ Promises, timers, microtasks (event-loop-driven) ═════════════════════════

/// A short `Name: message` string for an error value (used when an await
/// rejection unwinds as a thrown error).
pub fn error_string(h: &host::JsHost, v: &Value) -> String {
    if let Some(JsObj::Object(props)) = h.get(v) {
        let name = props
            .get("name")
            .map(|x| h.str_of(x))
            .or_else(|| host::lookup_chain(h, v, "name").map(|x| h.str_of(&x)))
            .unwrap_or_else(|| "Error".into());
        if let Some(m) = props.get("message") {
            return format!("{name}: {}", h.str_of(m));
        }
        return name;
    }
    h.str_of(v)
}

fn make_builtin(name: String) -> Value {
    with_host(|h| h.alloc(JsObj::Builtin(name)))
}

/// `[[GetPrototypeOf]]` (10.1.1) — the answer `Object.getPrototypeOf`,
/// `Reflect.getPrototypeOf` and a `__proto__` READ all have to agree on.
///
/// `__proto__` used to answer from `JsHost::proto_of` alone, which records only
/// an EXPLICIT link, so an object on the default prototype reported `null`:
/// `({}).__proto__ === Object.prototype` was false while
/// `Object.getPrototypeOf({}) === Object.prototype` was true. One function, so
/// the three cannot drift apart again.
pub fn prototype_of(v: &Value) -> Value {
    // Constructor-side inheritance: `Buffer extends Uint8Array`, so
    // `Object.getPrototypeOf(Buffer)` is the `Uint8Array` constructor itself,
    // not `Function.prototype`. This is the class-side half of the subclass
    // link — the instance-side half is `Buffer.prototype`'s `[[Prototype]]`.
    if matches!(with_host(|h| h.get(v).cloned()), Some(JsObj::Builtin(ref n)) if n == "Buffer") {
        return with_host(|h| h.alloc(JsObj::Builtin("Uint8Array".into())));
    }
    // Constructor-side inheritance for a `class B extends A` (ClassDefinition
    // 15.7.14 step 6.d: the constructor's `[[Prototype]]` is the parent
    // CONSTRUCTOR, not `Function.prototype`). Statics already resolved through
    // `ClassVal.parent`, but the link itself was invisible, so
    // `Object.getPrototypeOf(B) === A` read false and any library walking the
    // constructor chain — rather than calling a static — saw a base class.
    // A base class keeps the default answer below (`Function.prototype`).
    if let Some(JsObj::Class(c)) = with_host(|h| h.get(v).cloned()) {
        if let Some(parent) = c.parent {
            return parent;
        }
    }
    // `Object.create(null)` and friends really do have a null prototype.
    if with_host(|h| h.has_null_proto(v)) {
        return with_host(|h| h.null());
    }
    if let Some(p) = with_host(|h| h.proto_of(v)) {
        return p;
    }
    // A builtin exotic with no explicit `[[Prototype]]` link reports its
    // constructor's prototype namespace (`Object.getPrototypeOf([]) ===
    // Array.prototype`), which `strict_eq` compares by name. A plain object
    // reports the one real `Object.prototype` object.
    with_host(|h| {
        h.ensure_native_protos();
        match default_ctor_name(h, v) {
            Some("Object") => h.object_proto(),
            Some(c) => h.alloc(JsObj::Builtin(format!("{c}.prototype"))),
            None => h.null(),
        }
    })
}

/// `new Promise((resolve, reject) => …)` — run the executor synchronously with
/// internal resolve/reject functions.
fn new_promise(executor: Value) -> Result<Value, String> {
    let p = with_host(|h| h.new_promise());
    let id = with_host(|h| h.promise_id(&p).unwrap());
    let res = make_builtin(format!("@@presolve:{id}"));
    let rej = make_builtin(format!("@@preject:{id}"));
    if let Err(e) = host::invoke(&executor, vec![res, rej], None) {
        // A throw in the executor rejects the promise.
        let ev = host::take_exc_or_error(&e);
        host::reject_promise_val(id, ev);
    }
    Ok(p)
}

fn promise_resolve(v: Value) -> Result<Value, String> {
    Ok(host::promise_of(&v))
}
fn promise_reject(v: Value) -> Result<Value, String> {
    let p = with_host(|h| h.new_promise());
    let id = with_host(|h| h.promise_id(&p).unwrap());
    host::reject_promise_val(id, v);
    Ok(p)
}

/// `Promise.withResolvers()` — a fresh pending promise paired with its own
/// resolve/reject continuations (the same `@@presolve`/`@@preject` thunks the
/// executor receives), returned as a plain `{ promise, resolve, reject }` object.
fn promise_with_resolvers() -> Result<Value, String> {
    let p = with_host(|h| h.new_promise());
    let id = with_host(|h| h.promise_id(&p).unwrap());
    let resolve = make_builtin(format!("@@presolve:{id}"));
    let reject = make_builtin(format!("@@preject:{id}"));
    let mut props: IndexMap<String, Value> = IndexMap::new();
    props.insert("promise".into(), p);
    props.insert("resolve".into(), resolve);
    props.insert("reject".into(), reject);
    Ok(with_host(|h| h.new_object(props)))
}

#[derive(Clone, Copy)]
enum AllMode {
    All,
    AllSettled,
}

/// `Promise.all` / `Promise.allSettled`.
fn promise_all(args: Vec<Value>, mode: AllMode) -> Result<Value, String> {
    let items = host::iter_all(&arg0(&args))?;
    let result = with_host(|h| h.new_promise());
    let rid = with_host(|h| h.promise_id(&result).unwrap());
    let n = items.len();
    if n == 0 {
        let empty = with_host(|h| h.new_array(Vec::new()));
        host::resolve_promise_val(rid, empty);
        return Ok(result);
    }
    // Shared mutable accumulator via Rc<RefCell<…>>.
    let slots = std::rc::Rc::new(std::cell::RefCell::new(vec![Value::Undef; n]));
    let remaining = std::rc::Rc::new(std::cell::RefCell::new(n));
    for (i, it) in items.into_iter().enumerate() {
        let ap = host::promise_of(&it);
        let aid = with_host(|h| h.promise_id(&ap).unwrap());
        let slots = slots.clone();
        let remaining = remaining.clone();
        host::subscribe_native(
            aid,
            Box::new(move |state, val| {
                let settled = match mode {
                    AllMode::All => {
                        if state == host::PromiseState::Rejected {
                            host::reject_promise_val(rid, val);
                            return Ok(());
                        }
                        val
                    }
                    AllMode::AllSettled => with_host(|h| {
                        let mut m: IndexMap<String, Value> = IndexMap::new();
                        if state == host::PromiseState::Rejected {
                            m.insert("status".into(), h.new_str("rejected"));
                            m.insert("reason".into(), val);
                        } else {
                            m.insert("status".into(), h.new_str("fulfilled"));
                            m.insert("value".into(), val);
                        }
                        h.new_object(m)
                    }),
                };
                slots.borrow_mut()[i] = settled;
                let mut r = remaining.borrow_mut();
                *r -= 1;
                if *r == 0 {
                    let arr = with_host(|h| h.new_array(slots.borrow().clone()));
                    host::resolve_promise_val(rid, arr);
                }
                Ok(())
            }),
        );
    }
    Ok(result)
}

/// `Promise.race` (first to settle wins) / `Promise.any` (first to fulfill wins).
fn promise_race(args: Vec<Value>, any: bool) -> Result<Value, String> {
    let items = host::iter_all(&arg0(&args))?;
    let result = with_host(|h| h.new_promise());
    let rid = with_host(|h| h.promise_id(&result).unwrap());
    let n = items.len();
    let errors = std::rc::Rc::new(std::cell::RefCell::new(vec![Value::Undef; n]));
    let remaining = std::rc::Rc::new(std::cell::RefCell::new(n));
    for (i, it) in items.into_iter().enumerate() {
        let ap = host::promise_of(&it);
        let aid = with_host(|h| h.promise_id(&ap).unwrap());
        let errors = errors.clone();
        let remaining = remaining.clone();
        host::subscribe_native(
            aid,
            Box::new(move |state, val| {
                if any {
                    if state == host::PromiseState::Fulfilled {
                        host::resolve_promise_val(rid, val);
                    } else {
                        errors.borrow_mut()[i] = val;
                        let mut r = remaining.borrow_mut();
                        *r -= 1;
                        if *r == 0 {
                            // All rejected → AggregateError carrying every reason.
                            let reasons = with_host(|h| h.new_array(errors.borrow().clone()));
                            let msg = with_host(|h| h.new_str("All promises were rejected"));
                            let agg = make_error("AggregateError", &[reasons, msg]);
                            host::reject_promise_val(rid, agg);
                        }
                    }
                } else if state == host::PromiseState::Rejected {
                    host::reject_promise_val(rid, val);
                } else {
                    host::resolve_promise_val(rid, val);
                }
                Ok(())
            }),
        );
    }
    Ok(result)
}

/// `.then` / `.catch` / `.finally` on a promise.
fn promise_method(recv: &Value, name: &str, args: Vec<Value>) -> Result<Value, String> {
    match name {
        "then" => Ok(host::promise_then(
            recv,
            args.first().cloned().unwrap_or(Value::Undef),
            args.get(1).cloned().unwrap_or(Value::Undef),
        )),
        "catch" => Ok(host::promise_then(
            recv,
            Value::Undef,
            args.first().cloned().unwrap_or(Value::Undef),
        )),
        "finally" => {
            let cb = arg0(&args);
            let i = match cb {
                Value::Obj(i) => i,
                _ => 0,
            };
            let pass = make_builtin(format!("@@finpass:{i}"));
            let throw = make_builtin(format!("@@finthrow:{i}"));
            Ok(host::promise_then(recv, pass, throw))
        }
        _ => Err(host::type_error(&format!(
            "promise.{name} is not a function"
        ))),
    }
}

fn enqueue_microtask(next_tick: bool, cb: Value, args: Vec<Value>) {
    with_host(|h| {
        if next_tick {
            h.queue_nexttick(cb, args);
        } else {
            h.queue_micro(cb, args);
        }
    });
}

/// `setTimeout`/`setInterval`/`setImmediate` — register a macrotask and return
/// the handle object Node returns (`Timeout` for the first two, `Immediate` for
/// the third), carrying `ref`/`unref`/`hasRef`/`refresh`.
///
/// `setInterval` schedules a *repeating* timer: the loop re-arms it each time it
/// fires, so it runs until cleared and — being referenced — holds the process
/// open exactly as in Node.
fn schedule_timer(name: &str, args: Vec<Value>) -> Value {
    let cb = arg0(&args);
    let delay = if name == "setImmediate" {
        -1.0 // before any 0ms timeout
    } else {
        args.get(1)
            .map(|d| with_host(|h| h.to_number(d)))
            .unwrap_or(0.0)
            .max(0.0)
    };
    let extra = if name == "setImmediate" {
        args.get(1..).map(|s| s.to_vec()).unwrap_or_default()
    } else {
        args.get(2..).map(|s| s.to_vec()).unwrap_or_default()
    };
    // Node clamps a sub-1ms interval to 1ms, so `setInterval(fn, 0)` yields a
    // ~1000Hz timer rather than a busy loop that starves the rest of the queue.
    let interval = (name == "setInterval").then(|| delay.max(1.0));
    let id = with_host(|h| h.add_timer(delay, cb, extra, interval));
    let tag = if name == "setImmediate" {
        "Immediate"
    } else {
        "Timeout"
    };
    crate::stdlib::timers::new_handle(id, tag)
}

/// `clearTimeout`/`clearInterval`/`clearImmediate` — cancel by handle object or
/// by the bare id it coerces to (code that stored `+timer` still works).
fn clear_timer(v: &Value) {
    let id =
        crate::stdlib::timers::handle_id(v).unwrap_or_else(|| with_host(|h| h.to_number(v)) as u64);
    with_host(|h| h.cancel_timer(id));
}
