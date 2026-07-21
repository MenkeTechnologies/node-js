//! Builtin op handlers (compiler-emitted `CallBuiltin` ids) plus the JS standard
//! library (`console`, `Math`, `JSON`, `Object`, array/string methods) reachable
//! from the host. Handlers pop their arguments off the VM operand stack and
//! return the result value, which the VM pushes back.

use crate::host::{self, ops, with_host, FuncVal, JsObj};
use fusevm::{NumOp, Value, VM};
use indexmap::IndexMap;

/// Register every node-js builtin id on a VM.
pub fn install(vm: &mut VM) {
    vm.register_builtin(ops::GETLOCAL, b_getlocal);
    vm.register_builtin(ops::SETLOCAL, b_setlocal);
    vm.register_builtin(ops::DECLARE, b_declare);
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
}

/// `ITER_CLOSE`: close the iterator on the stack (a for-of `break`). A generator
/// runs its pending `finally`; a user iterator object gets its `.return()` called
/// if present; a plain materialized iterator just drops. Returns `undefined`.
fn b_iter_close(vm: &mut VM, _: u8) -> Value {
    let it = vm.pop();
    if with_host(|h| h.is_generator_val(&it)) {
        // Ignore the close outcome; a `finally` may print/yield but the loop is
        // done. Preserve any error it raises (uncaught finally throw propagates).
        if let Err(e) = host::gen_return(&it, Value::Undef) {
            return abort(vm, e);
        }
        return Value::Undef;
    }
    // A user iterator object with a `.return()` method (iterator protocol close).
    if matches!(with_host(|h| h.get(&it).cloned()), Some(JsObj::Object(_))) {
        if let Some(f) = with_host(|h| host::lookup_chain(h, &it, "return")) {
            if with_host(|h| host::is_callable(h, &f)) {
                if let Err(e) = host::invoke(&f, Vec::new(), Some(it.clone())) {
                    return abort(vm, e);
                }
            }
        }
    }
    Value::Undef
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
    with_host(|h| h.set_fn_prop(&strings, "raw", raw_arr));
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
    let thunk = vm.pop();
    let name = sval(&vm.pop());
    let class_val = vm.pop();
    host::define_field(&class_val, &name, thunk);
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
    for (name, thunk) in fields {
        match host::invoke(&thunk, Vec::new(), Some(this.clone())) {
            Ok(val) => with_host(|h| {
                if let Some(JsObj::Object(props)) = h.get_mut(&this) {
                    props.insert(name, val);
                }
            }),
            Err(e) => return abort(vm, e),
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

fn b_yield(vm: &mut VM, _: u8) -> Value {
    let v = vm.pop();
    match host::gen_yield(v) {
        Ok(sent) => {
            // A `.return()`/`.throw()` injected on resume sets a pending Return
            // signal (or error); halt the chunk so the body unwinds through any
            // enclosing `try/finally`, exactly like a source `return`/`throw`.
            if with_host(|h| h.error.is_some() || h.signal.is_some()) {
                vm.ip = vm.chunk.ops.len();
            }
            sent
        }
        Err(e) => abort(vm, e),
    }
}

fn b_propkey(vm: &mut VM, _: u8) -> Value {
    let v = vm.pop();
    let k = with_host(|h| h.property_key(&v));
    with_host(|h| h.new_str(k))
}

fn b_new_target(_vm: &mut VM, _: u8) -> Value {
    with_host(|h| h.current_new_target().unwrap_or(Value::Undef))
}

/// `a / b` with JS/IEEE-754 semantics. fusevm's native `Op::Div` returns `Undef`
/// for a zero divisor (so a frontend whose `/` differs must lower to a builtin —
/// its own documented guidance), but JavaScript requires `x/0 === ±Infinity` and
/// `0/0 === NaN`, so `/` is lowered here instead. Non-number operands are coerced
/// via `ToNumber`, exactly as the numeric hook's `arith(Div)` path does.
fn b_div(vm: &mut VM, _: u8) -> Value {
    let b = vm.pop();
    let a = vm.pop();
    let r = with_host(|h| h.arith(NumOp::Div, &a, &b));
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

fn b_getlocal(vm: &mut VM, _: u8) -> Value {
    let name = sval(&vm.pop());
    if let Some(v) = with_host(|h| h.read_name(&name)) {
        return v;
    }
    // Globals bound lazily: numeric sentinels + builtin namespaces.
    match name.as_str() {
        "undefined" => return Value::Undef,
        "NaN" => return Value::Float(f64::NAN),
        "Infinity" => return Value::Float(f64::INFINITY),
        "globalThis" => return with_host(|h| h.new_object(IndexMap::new())),
        _ => {}
    }
    if is_namespace(&name) || is_known_builtin(&name) {
        return with_host(|h| h.alloc(JsObj::Builtin(name.clone())));
    }
    abort(vm, host::ref_error(&name))
}

fn b_setlocal(vm: &mut VM, _: u8) -> Value {
    let val = vm.pop();
    let name = sval(&vm.pop());
    with_host(|h| h.set_name(&name, val.clone()));
    val
}

fn b_declare(vm: &mut VM, _: u8) -> Value {
    let val = vm.pop();
    let name = sval(&vm.pop());
    with_host(|h| h.declare_name(&name, val.clone()));
    val
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
pub fn get_property(recv: &Value, name: &str) -> Result<Value, String> {
    if with_host(|h| h.is_nullish(recv)) {
        return Err(host::type_error(&format!(
            "Cannot read properties of {} (reading '{name}')",
            with_host(|h| h.str_of(recv))
        )));
    }
    // Accessor (own or inherited getter) takes precedence over the chain walk.
    if let Some((getter, _)) = with_host(|h| host::lookup_accessor(h, recv, name)) {
        return match getter {
            Some(g) => host::invoke(&g, Vec::new(), Some(recv.clone())),
            None => Ok(Value::Undef), // set-only property reads as undefined
        };
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
    let obj = with_host(|h| h.get(recv).cloned());
    Ok(match obj {
        Some(JsObj::Object(props)) => {
            // Typed-array element read (`ta[i]`): elements live in a hidden
            // `@@elems`, not as own numeric props, so intercept integer keys.
            if !name.is_empty()
                && name.bytes().all(|b| b.is_ascii_digit())
                && matches!(props.get("@@native"), Some(v) if with_host(|h| h.str_of(v)) == "TypedArray")
            {
                if let Some(v) = crate::stdlib::typedarray::elem_get(recv, name) {
                    return Ok(v);
                }
            }
            if let Some(v) = props.get(name) {
                v.clone()
            } else if let Some(v) = with_host(|h| host::lookup_chain(h, recv, name)) {
                // A method / data property inherited from the prototype chain.
                v
            } else if name == "__proto__" {
                with_host(|h| h.proto_of(recv)).unwrap_or_else(|| with_host(|h| h.null()))
            } else if crate::stdlib::native_tag(recv)
                .map(|tag| crate::stdlib::instance_has_method(&tag, name))
                .unwrap_or(false)
            {
                // A native instance method read as a property (`server.listen`) →
                // a bound method, dispatched via `instance_call` when invoked.
                bound_method(recv, name)
            } else if is_object_method(name) {
                bound_method(recv, name)
            } else {
                Value::Undef
            }
        }
        Some(JsObj::Class(_)) | Some(JsObj::Func(_)) | Some(JsObj::BoundFunc { .. }) => {
            function_property(recv, name)
        }
        Some(JsObj::Symbol { desc, .. }) => match name {
            "description" => match desc {
                Some(d) => with_host(|h| h.new_str(d)),
                None => Value::Undef,
            },
            "toString" => bound_method(recv, name),
            _ => Value::Undef,
        },
        Some(JsObj::BigInt(_)) => {
            if matches!(
                name,
                "toString" | "valueOf" | "toLocaleString" | "constructor"
            ) {
                bound_method(recv, name)
            } else {
                Value::Undef
            }
        }
        Some(JsObj::RegExp(r)) => crate::regexp::regexp_property(&r, name).unwrap_or_else(|| {
            if crate::regexp::is_regexp_method(name) {
                bound_method(recv, name)
            } else {
                Value::Undef
            }
        }),
        Some(JsObj::Map { entries, .. }) => match name {
            "size" => Value::Float(entries.len() as f64),
            "@@iterator" => bound_method(recv, name),
            _ if is_map_method(name) => bound_method(recv, name),
            _ => Value::Undef,
        },
        Some(JsObj::Set { entries, .. }) => match name {
            "size" => Value::Float(entries.len() as f64),
            "@@iterator" => bound_method(recv, name),
            _ if is_set_method(name) => bound_method(recv, name),
            _ => Value::Undef,
        },
        Some(JsObj::Generator { .. }) => {
            if is_generator_method(name) {
                bound_method(recv, name)
            } else {
                Value::Undef
            }
        }
        Some(JsObj::Promise { .. }) => {
            if matches!(name, "then" | "catch" | "finally") {
                bound_method(recv, name)
            } else {
                Value::Undef
            }
        }
        Some(JsObj::Iter { .. }) => {
            if matches!(name, "next" | "return" | "@@iterator") {
                bound_method(recv, name)
            } else {
                Value::Undef
            }
        }
        Some(JsObj::Array(items)) => {
            if name == "length" {
                Value::Float(items.len() as f64)
            } else if let Ok(i) = name.parse::<usize>() {
                items.get(i).cloned().unwrap_or(Value::Undef)
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
        Some(JsObj::Str(s)) => {
            if name == "length" {
                Value::Float(s.chars().count() as f64)
            } else if let Ok(i) = name.parse::<usize>() {
                match s.chars().nth(i) {
                    Some(c) => with_host(|h| h.new_str(c.to_string())),
                    None => Value::Undef,
                }
            } else if name == "@@iterator" || is_string_method(name) {
                bound_method(recv, name)
            } else {
                Value::Undef
            }
        }
        Some(JsObj::Builtin(ns)) => namespace_property(&ns, name),
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
                Some("TextEncoder") => Some("TextEncoder"),
                Some("TextDecoder") => Some("TextDecoder"),
                Some("EventEmitter") => Some("EventEmitter"),
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

/// The builtin globals that are constructor *functions* (callable via `new`), so
/// `Ctor.name` is the constructor name. Excludes the non-callable namespaces
/// (`Math`, `JSON`, `console`, `Reflect`, `process`), whose `.name` is
/// `undefined` in Node.
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
            | "WeakRef"
            | "TextEncoder"
            | "TextDecoder"
            | "IncomingMessage"
            | "ServerResponse"
            | "EventEmitter"
            | "Buffer"
            | "URL"
            | "URLSearchParams"
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
            | "valueOf"
            | "constructor"
    )
}

pub fn is_object_builtin_method(name: &str) -> bool {
    matches!(
        name,
        "hasOwnProperty" | "isPrototypeOf" | "propertyIsEnumerable" | "toString" | "valueOf"
    )
}

/// Dispatch an `Object.prototype` builtin method on an object/instance.
pub fn object_builtin_method(recv: &Value, name: &str, args: Vec<Value>) -> Result<Value, String> {
    match name {
        "hasOwnProperty" => {
            let k = with_host(|h| h.property_key(&arg0(&args)));
            // A builtin namespace/prototype receiver (`Map.prototype`) reports
            // ownership via `has_property` (its methods resolve as thunks).
            if matches!(with_host(|h| h.get(recv).cloned()), Some(JsObj::Builtin(_))) {
                return Ok(Value::Bool(has_property(recv, &k)));
            }
            let has = with_host(|h| match h.get(recv) {
                Some(JsObj::Object(p)) => p.contains_key(&k),
                Some(JsObj::Array(items)) => {
                    k == "length" || k.parse::<usize>().map(|i| i < items.len()).unwrap_or(false)
                }
                _ => false,
            });
            Ok(Value::Bool(has))
        }
        "isPrototypeOf" => {
            let target = arg0(&args);
            let mut cur = with_host(|h| h.proto_of(&target));
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
            let has =
                with_host(|h| matches!(h.get(recv), Some(JsObj::Object(p)) if p.contains_key(&k)));
            Ok(Value::Bool(has))
        }
        "toString" => Ok(with_host(|h| {
            // An instance with a custom `toString` up the chain is handled by
            // call_method before reaching here; this is the default.
            let s = h.str_of(recv);
            h.new_str(s)
        })),
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
    if matches!(with_host(|h| h.get(recv).cloned()), Some(JsObj::Class(_))) {
        if let Some(v) = with_host(|h| h.class_static(recv, name)) {
            return v;
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
    // Arrows / classes: no auto prototype (classes set their own).
    let is_arrow =
        matches!(with_host(|h| h.get(recv).cloned()), Some(JsObj::Func(f)) if f.is_arrow);
    if is_arrow {
        return Value::Undef;
    }
    if !matches!(with_host(|h| h.get(recv).cloned()), Some(JsObj::Func(_))) {
        return Value::Undef;
    }
    with_host(|h| {
        let proto = h.new_object(IndexMap::new());
        if let Some(JsObj::Object(p)) = h.get_mut(&proto) {
            p.insert("constructor".to_string(), recv.clone());
        }
        h.set_fn_prop(recv, "prototype", proto.clone());
        proto
    })
}

/// A property on a builtin namespace object (`Math.PI`, `Number.MAX_SAFE_INTEGER`,
/// `console.log`).
fn namespace_property(ns: &str, name: &str) -> Value {
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
        ("Number", "MIN_VALUE") => Some(f64::MIN_POSITIVE),
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
    // The well-known `Symbol.iterator` symbol (used as a computed method key).
    if ns == "Symbol" && name == "iterator" {
        return with_host(|h| h.well_known_iterator());
    }
    if ns == "Symbol" && name == "asyncIterator" {
        return with_host(|h| h.well_known_async_iterator());
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
        return with_host(|h| h.alloc(JsObj::Builtin(format!("{ns}.prototype"))));
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
    if ctor == "Object" && method == "toString" {
        return Ok(with_host(|h| h.new_str(object_tag(h, recv))));
    }
    // `EventEmitter.prototype.<m>` mixed onto a receiver (express's `app`): run the
    // emitter method directly against `recv` (routing back through `call_method`
    // would re-resolve the mixed-in thunk and recurse).
    if ctor == "EventEmitter" {
        return crate::stdlib::events::instance_call(recv, method, args);
    }
    host::call_method(recv, method, args)
}

/// The `Object.prototype.toString` brand tag for `v` (`[object Array]` etc.).
fn object_tag(h: &host::JsHost, v: &Value) -> String {
    let tag = match v {
        Value::Undef => "Undefined",
        Value::Bool(_) => "Boolean",
        Value::Int(_) | Value::Float(_) => "Number",
        Value::Str(_) => "String",
        Value::Obj(_) => match h.get(v) {
            Some(JsObj::Null) => "Null",
            Some(JsObj::Str(_)) => "String",
            Some(JsObj::Array(_)) => "Array",
            Some(JsObj::Func(_))
            | Some(JsObj::Class(_))
            | Some(JsObj::Builtin(_))
            | Some(JsObj::BoundFunc { .. })
            | Some(JsObj::BoundMethod { .. }) => "Function",
            Some(JsObj::RegExp(_)) => "RegExp",
            _ => "Object",
        },
        // node-js only produces the Value variants above; fusevm's shell-oriented
        // variants never arise here.
        _ => "Object",
    };
    format!("[object {tag}]")
}

fn b_setattr(vm: &mut VM, _: u8) -> Value {
    let val = vm.pop();
    let name = sval(&vm.pop());
    let recv = vm.pop();
    set_property(&recv, &name, val.clone());
    val
}

fn set_property(recv: &Value, name: &str, val: Value) {
    // `obj.__proto__ = p` re-links the prototype.
    if name == "__proto__" && matches!(with_host(|h| h.get(recv).cloned()), Some(JsObj::Object(_)))
    {
        with_host(|h| h.set_proto(recv, val));
        return;
    }
    // An inherited/own setter accessor intercepts the write.
    if let Some((_, Some(setter))) = with_host(|h| host::lookup_accessor(h, recv, name)) {
        let _ = host::invoke(&setter, vec![val], Some(recv.clone()));
        return;
    }
    // A set-only-elsewhere getter (accessor with no setter): ignore the write.
    if let Some((Some(_), None)) = with_host(|h| host::lookup_accessor(h, recv, name)) {
        return;
    }
    // Writing `name`/`prototype`/statics on a function value.
    if matches!(
        with_host(|h| h.get(recv).cloned()),
        Some(JsObj::Func(_)) | Some(JsObj::Class(_))
    ) {
        with_host(|h| h.set_fn_prop(recv, name, val));
        return;
    }
    // Writing a static onto a builtin namespace/ctor (`Error.prepareStackTrace`).
    // Each bare reference is a fresh `Builtin` handle, so route to the stable
    // per-namespace side table rather than the per-index `fn_props`.
    if let Some(JsObj::Builtin(ns)) = with_host(|h| h.get(recv).cloned()) {
        with_host(|h| h.set_builtin_static(&ns, name, val));
        return;
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
                        n as usize
                    } else {
                        0
                    };
                }
            });
            return;
        }
    }
    // Typed-array element write (`ta[i] = v`): coerce + store into `@@elems`.
    if !name.is_empty() && name.bytes().all(|b| b.is_ascii_digit()) {
        let is_ta = matches!(
            with_host(|h| h.get(recv).cloned()),
            Some(JsObj::Object(ref p)) if p.get("@@native").map(|v| with_host(|h| h.str_of(v))).as_deref() == Some("TypedArray")
        );
        if is_ta && crate::stdlib::typedarray::elem_set(recv, name, &val) {
            return;
        }
    }
    // An arbitrary own prop on an array (e.g. exec-result `.index`/`.input`).
    if matches!(with_host(|h| h.get(recv).cloned()), Some(JsObj::Array(_)))
        && name != "length"
        && name.parse::<usize>().is_err()
    {
        with_host(|h| h.set_fn_prop(recv, name, val));
        return;
    }
    with_host(|h| match h.get_mut(recv) {
        Some(JsObj::Object(props)) => {
            props.insert(name.to_string(), val);
        }
        Some(JsObj::Array(items)) => {
            if name == "length" {
                let n = h_val_to_len(&val);
                items.resize(n, Value::Undef);
            } else if let Ok(i) = name.parse::<usize>() {
                if i >= items.len() {
                    items.resize(i + 1, Value::Undef);
                }
                items[i] = val;
            }
        }
        _ => {}
    });
}

fn h_val_to_len(v: &Value) -> usize {
    match v {
        Value::Float(f) if f.is_finite() && *f >= 0.0 => *f as usize,
        Value::Int(n) if *n >= 0 => *n as usize,
        _ => 0,
    }
}

fn b_getitem(vm: &mut VM, _: u8) -> Value {
    let idx = vm.pop();
    let recv = vm.pop();
    let key = with_host(|h| h.property_key(&idx));
    match get_property(&recv, &key) {
        Ok(v) => v,
        Err(e) => abort(vm, e),
    }
}

fn b_setitem(vm: &mut VM, _: u8) -> Value {
    let val = vm.pop();
    let idx = vm.pop();
    let recv = vm.pop();
    let key = with_host(|h| h.property_key(&idx));
    set_property(&recv, &key, val.clone());
    val
}

fn b_delitem(vm: &mut VM, _: u8) -> Value {
    let idx = vm.pop();
    let recv = vm.pop();
    let key = with_host(|h| h.str_of(&idx));
    with_host(|h| match h.get_mut(&recv) {
        Some(JsObj::Object(props)) => {
            props.shift_remove(&key);
        }
        Some(JsObj::Array(items)) => {
            if let Ok(i) = key.parse::<usize>() {
                if i < items.len() {
                    items[i] = Value::Undef;
                }
            }
        }
        _ => {}
    });
    Value::Bool(true)
}

fn b_delprop_name(vm: &mut VM, _: u8) -> Value {
    let name = sval(&vm.pop());
    let recv = vm.pop();
    with_host(|h| {
        if let Some(JsObj::Object(props)) = h.get_mut(&recv) {
            props.shift_remove(&name);
        }
    });
    Value::Bool(true)
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
        let spread = matches!(flat[i], Value::Int(1));
        if spread {
            let src = flat[i + 1].clone();
            let entries = with_host(|h| match h.get(&src) {
                Some(JsObj::Object(m)) => m
                    .iter()
                    .map(|(k, v)| (k.clone(), v.clone()))
                    .collect::<Vec<_>>(),
                Some(JsObj::Array(items)) => items
                    .iter()
                    .enumerate()
                    .map(|(idx, v)| (idx.to_string(), v.clone()))
                    .collect::<Vec<_>>(),
                _ => Vec::new(),
            });
            for (k, v) in entries {
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
    let is_arrow = with_host(|h| h.funcs.get(def_id).map(|d| d.is_arrow).unwrap_or(false));
    with_host(|h| {
        let env = h.current_env_capture();
        let this = h.current_this();
        h.alloc(JsObj::Func(FuncVal {
            def_id,
            env: Some(env),
            this,
            is_arrow,
            home_class: None,
        }))
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
        "globalThis" => "object".to_string(),
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
    let r = with_host(|h| h.bitwise(tag, &a, &b));
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
    with_host(|h| match tag {
        host::unop::POS => Value::Float(h.to_number(&v)),
        host::unop::BITNOT => {
            let n = h.to_number(&v);
            let i = if n.is_finite() {
                n.trunc() as i64 as i32
            } else {
                0
            };
            Value::Float(!i as f64)
        }
        _ => Value::Undef,
    })
}

// ── membership ────────────────────────────────────────────────────────────────

fn b_contains(vm: &mut VM, _: u8) -> Value {
    let container = vm.pop();
    let key = vm.pop();
    // `x in y` requires y to be an object.
    if !matches!(container, Value::Obj(_)) {
        return abort(vm, host::type_error("Cannot use 'in' operator to search"));
    }
    let k = with_host(|h| h.property_key(&key));
    Value::Bool(has_property(&container, &k))
}

// ── control ───────────────────────────────────────────────────────────────────

fn b_sig_return(vm: &mut VM, _: u8) -> Value {
    let v = vm.pop();
    with_host(|h| h.signal = Some(host::Signal::Return(v.clone())));
    vm.ip = vm.chunk.ops.len();
    v
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
    let td = match with_host(|h| h.try_def(id)) {
        Some(t) => t,
        None => return abort(vm, "internal: unknown try id".into()),
    };
    let mut pending: Option<String> = None;

    let body_res = host::run_chunk_on(td.block.clone());
    let signal_after = with_host(|h| h.signal.is_some());
    if let Err(e) = body_res {
        if signal_after {
            pending = Some(e);
        } else if let Some((bind, hbody)) = &td.handler {
            // Bind the thrown value (or a synthesized error) to the catch param.
            let thrown =
                with_host(|h| h.exc.clone()).unwrap_or_else(|| with_host(|h| synth_error(h, &e)));
            with_host(|h| {
                h.error = None;
                h.exc = None;
            });
            if let Some(name) = bind {
                with_host(|h| h.declare_name(name, thrown));
            }
            if let Err(e2) = host::run_chunk_on(hbody.clone()) {
                pending = Some(e2);
            }
        } else {
            pending = Some(e);
        }
    }

    // finally always runs; a finally error/signal supersedes.
    if let Some(fin) = &td.finalizer {
        let sig_before = with_host(|h| h.signal.take());
        match host::run_chunk_on(fin.clone()) {
            Ok(_) => {
                if with_host(|h| h.signal.is_none()) {
                    with_host(|h| h.signal = sig_before);
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
    let (name, message) = match e.split_once(": ") {
        Some((n, m)) if host::ERROR_NAMES.contains(&n) => (n.to_string(), m.to_string()),
        _ => ("Error".to_string(), e.to_string()),
    };
    let mut props: IndexMap<String, Value> = IndexMap::new();
    let mv = h.new_str(message.clone());
    props.insert("message".into(), mv);
    let stack = if message.is_empty() {
        format!("{name}\n    at <anonymous>")
    } else {
        format!("{name}: {message}\n    at <anonymous>")
    };
    let sv = h.new_str(stack);
    props.insert("stack".into(), sv);
    let obj = h.new_object(props);
    if let Some(p) = host::error_proto_of(h, &name) {
        h.set_proto(&obj, p);
    }
    obj
}

// ── iteration ─────────────────────────────────────────────────────────────────

fn b_getiter(vm: &mut VM, _: u8) -> Value {
    let v = vm.pop();
    // A generator is its own iterator (resumed lazily by FORITER).
    if with_host(|h| h.is_generator_val(&v)) {
        return v;
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
    let mut i = 0;
    while i + 1 < flat.len() {
        let spread = matches!(flat[i], Value::Int(1));
        let val = flat[i + 1].clone();
        if spread {
            match host::iter_all(&val) {
                Ok(items) => out.extend(items),
                Err(e) => return abort(vm, e),
            }
        } else {
            out.push(val);
        }
        i += 2;
    }
    with_host(|h| h.new_array(out))
}

// ── calls ──────────────────────────────────────────────────────────────────────

fn b_call(vm: &mut VM, argc: u8) -> Value {
    let mut args = pop_n(vm, argc as usize);
    let name = sval(&args.remove(0));
    let r = host::call_named(&name, args);
    finish(vm, r)
}

fn b_call_method(vm: &mut VM, argc: u8) -> Value {
    let mut args = pop_n(vm, argc as usize);
    let recv = args.remove(0);
    let name = sval(&args.remove(0));
    let r = host::call_method(&recv, &name, args);
    finish(vm, r)
}

fn b_call_value(vm: &mut VM, argc: u8) -> Value {
    let mut args = pop_n(vm, argc as usize);
    let callable = args.remove(0);
    let r = host::invoke(&callable, args, None);
    finish(vm, r)
}

fn b_new(vm: &mut VM, argc: u8) -> Value {
    let mut args = pop_n(vm, argc as usize);
    let ctor = args.remove(0);
    let r = host::construct(&ctor, args);
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
pub fn numeric_hook(op: NumOp, a: &Value, b: &Value) -> Result<Value, String> {
    with_host(|h| h.arith(op, a, b))
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
    "WeakRef",
    "TextEncoder",
    "TextDecoder",
    "queueMicrotask",
    "setTimeout",
    "setInterval",
    "setImmediate",
    "clearTimeout",
    "clearInterval",
    "structuredClone",
    "require",
    // CommonJS loader dispatch targets referenced by per-module `require`
    // closures (see `module.rs`); never written by user code.
    "__cjs_require",
    "__cjs_resolve",
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
    "Object.defineProperty",
    "Object.getOwnPropertyDescriptor",
    "Object.hasOwn",
    "Array.isArray",
    "Array.from",
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
    "Reflect.ownKeys",
    "Reflect.has",
    "Reflect.get",
    "Reflect.set",
    "Reflect.getPrototypeOf",
    "Promise.resolve",
    "Promise.reject",
    "Promise.all",
    "Promise.allSettled",
    "Promise.race",
    "Promise.any",
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
            None => Err(format!("Error: Cannot find module '{spec}'")),
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
            None => Err(format!("Error: Cannot find module '{spec}'")),
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
        set_property(&target, "stack", stack);
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
        // `eval` exists as a global (libraries like get-intrinsic capture it as an
        // intrinsic) but node-js has no runtime source evaluator; calling it is
        // unsupported. A non-string argument is returned unchanged, as in JS.
        "eval" => match arg0(&args) {
            v @ (Value::Undef | Value::Bool(_) | Value::Int(_) | Value::Float(_)) => Ok(v),
            _ => Err(host::type_error("eval is not supported in node-js")),
        },
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
                host::to_string_value(&args[0])
            }
        }
        "Number" => Ok(Value::Float(if args.is_empty() {
            0.0
        } else {
            with_host(|h| h.to_number(&args[0]))
        })),
        "BigInt" => bigint_ctor(&arg0(&args)),
        "RegExp" => regexp_ctor(&args),
        "BigInt.asIntN" | "BigInt.asUintN" => bigint_as_n(name.ends_with("asUintN"), &args),
        "Boolean" => Ok(Value::Bool(with_host(|h| h.truthy(&arg0(&args))))),
        "String.fromCharCode" => Ok(with_host(|h| {
            let s: String = args
                .iter()
                .filter_map(|a| char::from_u32(h.to_number(a) as u32))
                .collect();
            h.new_str(s)
        })),
        "String.raw" => string_raw(&args),
        // `Array(5)` === `new Array(5)` (length-5 empty), but `Array.of(5)` is `[5]`.
        "Array" => construct_builtin("Array", args),
        "Array.of" => Ok(with_host(|h| h.new_array(args))),
        "Array.isArray" => Ok(Value::Bool(matches!(
            with_host(|h| h.get(&arg0(&args)).cloned()),
            Some(JsObj::Array(_))
        ))),
        "Array.from" => array_from(args),
        "Object" => Ok(object_call(args)),
        "Object.keys" => object_keys(args, 0),
        "Object.values" => object_keys(args, 1),
        "Object.entries" => object_keys(args, 2),
        "Object.assign" => object_assign(args),
        "Object.freeze" => Ok(arg0(&args)),
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
        "Object.getPrototypeOf" | "Reflect.getPrototypeOf" => Ok(with_host(|h| {
            h.proto_of(&arg0(&args)).unwrap_or_else(|| h.null())
        })),
        "Object.setPrototypeOf" => {
            let obj = arg0(&args);
            let proto = args.get(1).cloned().unwrap_or(Value::Undef);
            with_host(|h| h.set_proto(&obj, proto));
            Ok(obj)
        }
        "Object.create" => object_create(args),
        "Object.getOwnPropertyNames" => object_keys(args, 0),
        // `Object.hasOwn(obj, key)` — the static form of `hasOwnProperty`.
        "Object.hasOwn" => {
            let obj = arg0(&args);
            let key = args.get(1).cloned().unwrap_or(Value::Undef);
            object_builtin_method(&obj, "hasOwnProperty", vec![key])
        }
        "Object.defineProperty" => object_define_property(args),
        "Object.getOwnPropertyDescriptor" => object_get_own_descriptor(args),
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
        "Symbol.keyFor" => Ok(with_host(|h| match h.get(&arg0(&args)) {
            Some(JsObj::Symbol { desc, .. }) => {
                desc.clone().map(|d| h.new_str(d)).unwrap_or(Value::Undef)
            }
            _ => Value::Undef,
        })),
        "Map" | "WeakMap" | "Set" | "WeakSet" | "Promise" => construct_builtin(name, args),
        "Reflect.ownKeys" => object_keys(args, 0),
        "Reflect.has" => {
            let obj = arg0(&args);
            let k = with_host(|h| h.property_key(&args.get(1).cloned().unwrap_or(Value::Undef)));
            Ok(Value::Bool(has_property(&obj, &k)))
        }
        "Reflect.get" => {
            let obj = arg0(&args);
            let k = with_host(|h| h.property_key(&args.get(1).cloned().unwrap_or(Value::Undef)));
            get_property(&obj, &k)
        }
        "Reflect.set" => {
            let obj = arg0(&args);
            let k = with_host(|h| h.property_key(&args.get(1).cloned().unwrap_or(Value::Undef)));
            let v = args.get(2).cloned().unwrap_or(Value::Undef);
            set_property(&obj, &k, v);
            Ok(Value::Bool(true))
        }
        "JSON.stringify" => json_stringify(args),
        "JSON.parse" => json_parse(args),
        "structuredClone" => Ok(deep_clone(&arg0(&args))),
        "queueMicrotask" | "process.nextTick" => {
            let cb = arg0(&args);
            let rest = args.get(1..).map(|s| s.to_vec()).unwrap_or_default();
            enqueue_microtask(name == "process.nextTick", cb, rest);
            Ok(Value::Undef)
        }
        "setTimeout" | "setInterval" | "setImmediate" => Ok(schedule_timer(name, args)),
        "clearTimeout" | "clearInterval" => {
            clear_timer(&arg0(&args));
            Ok(Value::Undef)
        }
        "Promise.resolve" => promise_resolve(arg0(&args)),
        "Promise.reject" => promise_reject(arg0(&args)),
        "Promise.all" => promise_all(args, AllMode::All),
        "Promise.allSettled" => promise_all(args, AllMode::AllSettled),
        "Promise.race" => promise_race(args, false),
        "Promise.any" => promise_race(args, true),
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
            // Exact for the integer f64 range; larger integers round-trip via the
            // decimal string.
            match BigInt::parse_bytes(host::fmt_number(*f).as_bytes(), 10) {
                Some(b) => b,
                None => return Err(host::type_error("Cannot convert value to a BigInt")),
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
            _ => return Err(host::type_error("Cannot convert value to a BigInt")),
        },
        _ => return Err(host::type_error("Cannot convert value to a BigInt")),
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
            // new Array(n) -> length-n array; new Array(a, b) -> [a, b].
            if args.len() == 1 {
                if let Value::Float(f) = args[0] {
                    if f.fract() == 0.0 && f >= 0.0 {
                        return Ok(with_host(|h| h.new_array(vec![Value::Undef; f as usize])));
                    }
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
        "RegExp" => regexp_ctor(&args),
        "BigInt" => Err(host::type_error("BigInt is not a constructor")),
        "Error" => Ok(make_error(name, &args)),
        n if host::ERROR_NAMES.contains(&n) => Ok(make_error(name, &args)),
        _ => Err(host::type_error(&format!("{name} is not a constructor"))),
    }
}

fn make_error(name: &str, args: &[Value]) -> Value {
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
        let stack = match &msg {
            Some(m) if !m.is_empty() => format!("{name}: {m}\n    at <anonymous>"),
            _ => format!("{name}\n    at <anonymous>"),
        };
        let sv = h.new_str(stack);
        props.insert("stack".into(), sv);
        let e = h.new_object(props);
        if let Some(p) = host::error_proto_of(h, name) {
            h.set_proto(&e, p);
        }
        e
    })
}

fn print_line(args: &[Value], stderr: bool) {
    // Node's console.log(...args) === util.format(...args): printf-style
    // substitution when the first arg is a format string, else inspect-and-join.
    let line: String = crate::stdlib::util::format(args);
    if stderr {
        eprintln!("{line}");
    } else {
        println!("{line}");
    }
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

fn parse_int(args: &[Value]) -> f64 {
    let s = with_host(|h| h.str_of(&arg0(args)));
    let radix = args
        .get(1)
        .map(|r| with_host(|h| h.to_number(r)) as u32)
        .filter(|r| (2..=36).contains(r));
    let t = s.trim();
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
    let n = i64::from_str_radix(&valid, radix)
        .map(|n| n as f64)
        .unwrap_or(f64::NAN);
    if neg {
        -n
    } else {
        n
    }
}

fn parse_float(args: &[Value]) -> f64 {
    let s = with_host(|h| h.str_of(&arg0(args)));
    let t = s.trim_start();
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
    // Longest numeric prefix.
    let mut end = 0;
    let bytes = t.as_bytes();
    let mut seen_dot = false;
    let mut seen_e = false;
    for (i, &c) in bytes.iter().enumerate() {
        match c {
            b'0'..=b'9' => end = i + 1,
            b'+' | b'-' if i == 0 || bytes[i - 1] == b'e' || bytes[i - 1] == b'E' => end = i + 1,
            b'.' if !seen_dot && !seen_e => {
                seen_dot = true;
                end = i + 1;
            }
            b'e' | b'E' if !seen_e && i > 0 => {
                seen_e = true;
                end = i + 1;
            }
            _ => break,
        }
    }
    t[..end].parse::<f64>().unwrap_or(f64::NAN)
}

fn math_fn(fname: &str, args: &[Value]) -> Result<Value, String> {
    let x = arg_num(args, 0);
    let r = match fname {
        "floor" => x.floor(),
        "ceil" => x.ceil(),
        "round" => {
            // JS rounds half up toward +Infinity, but preserves the sign of a
            // zero result: Math.round(-0.5) === -0, Math.round(-0.4) === -0.
            let r = (x + 0.5).floor();
            if r == 0.0 && x.is_sign_negative() {
                -0.0
            } else {
                r
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
        "pow" => x.powf(arg_num(args, 1)),
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
                    if n > m {
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
                    if n < m {
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
    }
    let entries: Vec<(String, Value)> = with_host(|h| match h.get(&v) {
        Some(JsObj::Object(props)) => props
            .iter()
            .filter(|(k, _)| !k.starts_with("@@") && !k.starts_with('#'))
            .map(|(k, val)| (k.clone(), val.clone()))
            .collect(),
        Some(JsObj::Array(items)) => items
            .iter()
            .enumerate()
            .map(|(i, val)| (i.to_string(), val.clone()))
            .collect(),
        _ => Vec::new(),
    });
    Ok(with_host(|h| {
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
    }))
}

fn object_assign(args: Vec<Value>) -> Result<Value, String> {
    let target = arg0(&args);
    for src in args.iter().skip(1) {
        let entries: Vec<(String, Value)> = with_host(|h| match h.get(src) {
            Some(JsObj::Object(p)) => p.iter().map(|(k, v)| (k.clone(), v.clone())).collect(),
            _ => Vec::new(),
        });
        with_host(|h| {
            if let Some(JsObj::Object(p)) = h.get_mut(&target) {
                for (k, v) in entries {
                    p.insert(k, v);
                }
            }
        });
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
    let v = arg0(&args);
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
                    None => props
                        .iter()
                        .filter(|(k, _)| !k.starts_with("@@") && !k.starts_with('#'))
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
    let v = p.parse_value()?;
    p.skip_ws();
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
            _ => Err("SyntaxError: Unexpected token in JSON".into()),
        }
    }
    fn expect_lit(&mut self, lit: &str) -> Result<(), String> {
        for ch in lit.chars() {
            if self.peek() != Some(ch) {
                return Err("SyntaxError: Unexpected token in JSON".into());
            }
            self.pos += 1;
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
    fn parse_number(&mut self) -> Result<Value, String> {
        let start = self.pos;
        while matches!(self.peek(), Some(c) if c.is_ascii_digit() || c == '-' || c == '+' || c == '.' || c == 'e' || c == 'E')
        {
            self.pos += 1;
        }
        let s: String = self.chars[start..self.pos].iter().collect();
        s.parse::<f64>()
            .map(Value::Float)
            .map_err(|_| "SyntaxError: bad number in JSON".into())
    }
    fn parse_string(&mut self) -> Result<String, String> {
        self.pos += 1; // opening quote
        let mut out = String::new();
        loop {
            match self.peek() {
                None => return Err("SyntaxError: unterminated string in JSON".into()),
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
                _ => return Err("SyntaxError: bad array in JSON".into()),
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
            let key = self.parse_string()?;
            self.skip_ws();
            if self.peek() != Some(':') {
                return Err("SyntaxError: expected ':' in JSON".into());
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
                _ => return Err("SyntaxError: bad object in JSON".into()),
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
            | "valueOf"
            | "match"
            | "matchAll"
            | "search"
            | "normalize"
            | "localeCompare"
    )
}

/// Whether `v` is a `RegExp` value (drives the regex path of `match`/`replace`/…).
fn is_regexp_arg(v: &Value) -> bool {
    matches!(with_host(|h| h.get(v).cloned()), Some(JsObj::RegExp(_)))
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
    matches!(name, "toFixed" | "toString" | "toPrecision" | "valueOf")
}

/// Dispatch `recv.name(args)` for the built-in prototype methods.
pub fn call_type_method(recv: &Value, name: &str, args: Vec<Value>) -> Result<Value, String> {
    let obj = with_host(|h| h.get(recv).cloned());
    match obj {
        Some(JsObj::Array(_)) => array_method(recv, name, args),
        Some(JsObj::Str(s)) => string_method(&s, name, args),
        Some(JsObj::Map { .. }) => map_method(recv, name, args),
        Some(JsObj::Set { .. }) => set_method(recv, name, args),
        Some(JsObj::Generator { .. }) => generator_method(recv, name, args),
        Some(JsObj::Promise { .. }) => promise_method(recv, name, args),
        Some(JsObj::Iter { .. }) => iter_method(recv, name, args),
        Some(JsObj::Symbol { .. }) => symbol_method(recv, name, args),
        Some(JsObj::BigInt(b)) => bigint_method(&b, name, args),
        Some(JsObj::RegExp(_)) => crate::regexp::regexp_method(recv, name, args),
        Some(JsObj::Func(_)) | Some(JsObj::Class(_)) | Some(JsObj::BoundFunc { .. }) => {
            match function_builtin_method(recv, name, &args)? {
                Some(v) => Ok(v),
                None => Err(host::type_error(&format!("{name} is not a function"))),
            }
        }
        Some(JsObj::Object(props)) => {
            if let Some(f) = props.get(name).cloned() {
                host::invoke(&f, args, Some(recv.clone()))
            } else if name == "hasOwnProperty" {
                let k = with_host(|h| h.str_of(&arg0(&args)));
                Ok(Value::Bool(props.contains_key(&k)))
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
            Err(host::type_error(&format!("{} is not a function", name)))
        }
    }
}

fn array_items(recv: &Value) -> Vec<Value> {
    with_host(|h| match h.get(recv) {
        Some(JsObj::Array(items)) => items.clone(),
        _ => Vec::new(),
    })
}

fn array_method(recv: &Value, name: &str, args: Vec<Value>) -> Result<Value, String> {
    match name {
        "push" => {
            with_host(|h| {
                if let Some(JsObj::Array(items)) = h.get_mut(recv) {
                    items.extend(args.iter().cloned());
                }
            });
            Ok(Value::Float(array_items(recv).len() as f64))
        }
        "pop" => Ok(with_host(|h| {
            if let Some(JsObj::Array(items)) = h.get_mut(recv) {
                items.pop().unwrap_or(Value::Undef)
            } else {
                Value::Undef
            }
        })),
        "shift" => Ok(with_host(|h| {
            if let Some(JsObj::Array(items)) = h.get_mut(recv) {
                if items.is_empty() {
                    Value::Undef
                } else {
                    items.remove(0)
                }
            } else {
                Value::Undef
            }
        })),
        "unshift" => {
            with_host(|h| {
                if let Some(JsObj::Array(items)) = h.get_mut(recv) {
                    for (i, a) in args.iter().enumerate() {
                        items.insert(i, a.clone());
                    }
                }
            });
            Ok(Value::Float(array_items(recv).len() as f64))
        }
        "join" => {
            let sep = if args.is_empty() {
                ",".to_string()
            } else {
                with_host(|h| h.str_of(&args[0]))
            };
            let items = array_items(recv);
            let s = with_host(|h| {
                items
                    .iter()
                    .map(|x| match x {
                        Value::Undef => String::new(),
                        _ if h.is_null(x) => String::new(),
                        _ => h.str_of(x),
                    })
                    .collect::<Vec<_>>()
                    .join(&sep)
            });
            Ok(with_host(|h| h.new_str(s)))
        }
        "indexOf" => {
            let items = array_items(recv);
            let target = arg0(&args);
            let idx = with_host(|h| items.iter().position(|x| h.strict_eq(x, &target)));
            Ok(Value::Float(idx.map(|i| i as f64).unwrap_or(-1.0)))
        }
        "lastIndexOf" => {
            let items = array_items(recv);
            let target = arg0(&args);
            let idx = with_host(|h| items.iter().rposition(|x| h.strict_eq(x, &target)));
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
            Ok(with_host(|h| h.new_array(items[lo..hi].to_vec())))
        }
        "concat" => {
            let mut out = array_items(recv);
            for a in &args {
                match with_host(|h| h.get(a).cloned()) {
                    Some(JsObj::Array(items)) => out.extend(items),
                    _ => out.push(a.clone()),
                }
            }
            Ok(with_host(|h| h.new_array(out)))
        }
        "reverse" => {
            with_host(|h| {
                if let Some(JsObj::Array(items)) = h.get_mut(recv) {
                    items.reverse();
                }
            });
            Ok(recv.clone())
        }
        "fill" => {
            // fill(value[, start[, end]]) — negative indices count from the end.
            let val = arg0(&args);
            let len = array_items(recv).len() as i64;
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
            });
            Ok(recv.clone())
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
            with_host(|h| {
                if let Some(JsObj::Array(a)) = h.get_mut(recv) {
                    for (k, v) in slice.into_iter().enumerate() {
                        if target + k < a.len() {
                            a[target + k] = v;
                        }
                    }
                }
            });
            Ok(recv.clone())
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
        "map" => {
            let items = array_items(recv);
            let cb = arg0(&args);
            let mut out = Vec::with_capacity(items.len());
            for (i, it) in items.iter().enumerate() {
                out.push(host::invoke(
                    &cb,
                    vec![it.clone(), Value::Float(i as f64), recv.clone()],
                    None,
                )?);
            }
            Ok(with_host(|h| h.new_array(out)))
        }
        "flatMap" => {
            let items = array_items(recv);
            let cb = arg0(&args);
            let mut out = Vec::new();
            for (i, it) in items.iter().enumerate() {
                let r = host::invoke(
                    &cb,
                    vec![it.clone(), Value::Float(i as f64), recv.clone()],
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
            let cb = arg0(&args);
            let mut out = Vec::new();
            for (i, it) in items.iter().enumerate() {
                let keep = host::invoke(
                    &cb,
                    vec![it.clone(), Value::Float(i as f64), recv.clone()],
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
            let cb = arg0(&args);
            for (i, it) in items.iter().enumerate() {
                host::invoke(
                    &cb,
                    vec![it.clone(), Value::Float(i as f64), recv.clone()],
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
                    vec![it.clone(), Value::Float(i as f64), recv.clone()],
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
                    vec![it.clone(), Value::Float(i as f64), recv.clone()],
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
            let cb = arg0(&args);
            for (i, it) in items.iter().enumerate() {
                let m = host::invoke(
                    &cb,
                    vec![it.clone(), Value::Float(i as f64), recv.clone()],
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
            let cb = arg0(&args);
            for (i, it) in items.iter().enumerate() {
                let m = host::invoke(
                    &cb,
                    vec![it.clone(), Value::Float(i as f64), recv.clone()],
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
            let cb = arg0(&args);
            let mut acc;
            let mut start = 0;
            if args.len() >= 2 {
                acc = args[1].clone();
            } else if !items.is_empty() {
                acc = items[0].clone();
                start = 1;
            } else {
                return Err(host::type_error(
                    "Reduce of empty array with no initial value",
                ));
            }
            for (i, it) in items.iter().enumerate().skip(start) {
                acc = host::invoke(
                    &cb,
                    vec![acc, it.clone(), Value::Float(i as f64), recv.clone()],
                    None,
                )?;
            }
            Ok(acc)
        }
        "reduceRight" => {
            let items = array_items(recv);
            let cb = arg0(&args);
            let n = items.len();
            let mut acc;
            let mut i = n; // one past the next index to process (walking down)
            if args.len() >= 2 {
                acc = args[1].clone();
            } else if n > 0 {
                acc = items[n - 1].clone();
                i = n - 1;
            } else {
                return Err(host::type_error(
                    "Reduce of empty array with no initial value",
                ));
            }
            while i > 0 {
                i -= 1;
                acc = host::invoke(
                    &cb,
                    vec![acc, items[i].clone(), Value::Float(i as f64), recv.clone()],
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
                    vec![items[i].clone(), Value::Float(i as f64), recv.clone()],
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
                    vec![items[i].clone(), Value::Float(i as f64), recv.clone()],
                    None,
                )?;
                if with_host(|h| h.truthy(&m)) {
                    return Ok(Value::Float(i as f64));
                }
            }
            Ok(Value::Float(-1.0))
        }
        "sort" => {
            let mut items = array_items(recv);
            sort_values(&mut items, args.first())?;
            with_host(|h| {
                if let Some(JsObj::Array(a)) = h.get_mut(recv) {
                    *a = items;
                }
            });
            Ok(recv.clone())
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
            flatten_into(array_items(recv), depth, &mut out);
            Ok(with_host(|h| h.new_array(out)))
        }
        "keys" => {
            let n = array_items(recv).len();
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
        "toString" => {
            let s = with_host(|h| h.str_of(recv));
            Ok(with_host(|h| h.new_str(s)))
        }
        _ => Err(host::type_error(&format!("{name} is not a function"))),
    }
}

/// In-place sort of `items` (shared by `sort` and `toSorted`). Uses insertion
/// sort so the fallible JS comparator can be called; default order is by the
/// string form of each element. Propagates a comparator error.
fn sort_values(items: &mut [Value], cmp: Option<&Value>) -> Result<(), String> {
    for i in 1..items.len() {
        let mut j = i;
        while j > 0 {
            let order = match cmp {
                Some(cb) => {
                    let v = host::invoke(cb, vec![items[j - 1].clone(), items[j].clone()], None)?;
                    with_host(|h| h.to_number(&v))
                }
                None => {
                    let a = with_host(|h| h.str_of(&items[j - 1]));
                    let b = with_host(|h| h.str_of(&items[j]));
                    if a > b {
                        1.0
                    } else {
                        -1.0
                    }
                }
            };
            if order > 0.0 {
                items.swap(j - 1, j);
                j -= 1;
            } else {
                break;
            }
        }
    }
    Ok(())
}

/// Recursively flatten `items` up to `depth` levels into `out`. `depth` is an
/// f64 so `Infinity` (full flatten) and finite counts share one path.
fn flatten_into(items: Vec<Value>, depth: f64, out: &mut Vec<Value>) {
    for it in items {
        let inner = if depth > 0.0 {
            match with_host(|h| h.get(&it).cloned()) {
                Some(JsObj::Array(inner)) => Some(inner),
                _ => None,
            }
        } else {
            None
        };
        match inner {
            Some(inner) => flatten_into(inner, depth - 1.0, out),
            None => out.push(it),
        }
    }
}

fn array_splice(recv: &Value, args: Vec<Value>) -> Result<Value, String> {
    let len = array_items(recv).len();
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
    let removed = with_host(|h| {
        if let Some(JsObj::Array(items)) = h.get_mut(recv) {
            let removed: Vec<Value> = items.splice(start..start + delete, inserts).collect();
            removed
        } else {
            Vec::new()
        }
    });
    Ok(with_host(|h| h.new_array(removed)))
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
    let chars: Vec<char> = s.chars().collect();
    match name {
        "@@iterator" => {
            let items: Vec<Value> = chars.iter().map(|c| new_s(c.to_string())).collect();
            Ok(with_host(|h| h.alloc(JsObj::Iter { items, idx: 0 })))
        }
        "toUpperCase" => Ok(new_s(s.to_uppercase())),
        "toLowerCase" => Ok(new_s(s.to_lowercase())),
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
        "normalize" => Ok(new_s(s.to_string())),
        "trim" => Ok(new_s(s.trim().to_string())),
        "trimStart" => Ok(new_s(s.trim_start().to_string())),
        "trimEnd" => Ok(new_s(s.trim_end().to_string())),
        "toString" | "valueOf" => Ok(new_s(s.to_string())),
        "charAt" => {
            let i = arg_num(&args, 0) as usize;
            Ok(new_s(
                chars.get(i).map(|c| c.to_string()).unwrap_or_default(),
            ))
        }
        "at" => {
            let mut i = arg_num(&args, 0) as i64;
            if i < 0 {
                i += chars.len() as i64;
            }
            if i >= 0 && (i as usize) < chars.len() {
                Ok(new_s(chars[i as usize].to_string()))
            } else {
                Ok(Value::Undef)
            }
        }
        "charCodeAt" | "codePointAt" => {
            let i = arg_num(&args, 0) as usize;
            match chars.get(i) {
                Some(c) => Ok(Value::Float(*c as u32 as f64)),
                None => Ok(Value::Float(f64::NAN)),
            }
        }
        "indexOf" => {
            let needle = with_host(|h| h.str_of(&arg0(&args)));
            Ok(Value::Float(byte_to_char_index(s, s.find(&needle))))
        }
        "lastIndexOf" => {
            let needle = with_host(|h| h.str_of(&arg0(&args)));
            Ok(Value::Float(byte_to_char_index(s, s.rfind(&needle))))
        }
        "includes" => {
            let needle = with_host(|h| h.str_of(&arg0(&args)));
            Ok(Value::Bool(s.contains(&needle)))
        }
        "startsWith" => {
            let needle = with_host(|h| h.str_of(&arg0(&args)));
            Ok(Value::Bool(s.starts_with(&needle)))
        }
        "endsWith" => {
            let needle = with_host(|h| h.str_of(&arg0(&args)));
            Ok(Value::Bool(s.ends_with(&needle)))
        }
        "slice" => {
            let (lo, hi) = slice_bounds(&args, chars.len());
            Ok(new_s(chars[lo..hi].iter().collect()))
        }
        "substring" => {
            let mut a = arg_num(&args, 0).max(0.0) as usize;
            let mut b = if args.len() < 2 || matches!(args[1], Value::Undef) {
                chars.len()
            } else {
                (arg_num(&args, 1).max(0.0) as usize).min(chars.len())
            };
            a = a.min(chars.len());
            if a > b {
                std::mem::swap(&mut a, &mut b);
            }
            Ok(new_s(chars[a..b].iter().collect()))
        }
        "substr" => {
            // A negative start counts from the end: max(len + start, 0).
            let len = chars.len() as i64;
            let mut start = arg_num(&args, 0) as i64;
            if start < 0 {
                start = (len + start).max(0);
            }
            let start = (start as usize).min(chars.len());
            let count = if args.len() >= 2 {
                arg_num(&args, 1).max(0.0) as usize
            } else {
                chars.len()
            };
            let end = (start + count).min(chars.len());
            Ok(new_s(chars[start..end].iter().collect()))
        }
        "repeat" => {
            let n = arg_num(&args, 0);
            if n < 0.0 || !n.is_finite() {
                return Err(host::type_error("Invalid count value"));
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
        "padStart" => Ok(new_s(pad(s, &args, true))),
        "padEnd" => Ok(new_s(pad(s, &args, false))),
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
                Ok(Value::Float(byte_to_char_index(s, s.find(&needle))))
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
                    chars.iter().map(|c| new_s(c.to_string())).collect()
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

fn byte_to_char_index(s: &str, byte: Option<usize>) -> f64 {
    match byte {
        Some(b) => s[..b].chars().count() as f64,
        None => -1.0,
    }
}

fn pad(s: &str, args: &[Value], start: bool) -> String {
    let target = arg_num(args, 0) as usize;
    let cur = s.chars().count();
    if cur >= target {
        return s.to_string();
    }
    let filler = if args.len() >= 2 {
        with_host(|h| h.str_of(&args[1]))
    } else {
        " ".to_string()
    };
    if filler.is_empty() {
        return s.to_string();
    }
    let need = target - cur;
    let fill_chars: Vec<char> = filler.chars().collect();
    let padding: String = (0..need)
        .map(|i| fill_chars[i % fill_chars.len()])
        .collect();
    if start {
        format!("{padding}{s}")
    } else {
        format!("{s}{padding}")
    }
}

/// `BigInt.prototype` methods: `toString([radix])`, `valueOf`, `toLocaleString`.
fn bigint_method(b: &num_bigint::BigInt, name: &str, args: Vec<Value>) -> Result<Value, String> {
    match name {
        "toString" => {
            let radix = args.first().map(|_| arg_num(&args, 0) as u32).unwrap_or(10);
            if !(2..=36).contains(&radix) {
                return Err("RangeError: toString() radix must be between 2 and 36".into());
            }
            Ok(new_s(b.to_str_radix(radix)))
        }
        "toLocaleString" => Ok(new_s(b.to_string())),
        "valueOf" => Ok(with_host(|h| h.new_bigint(b.clone()))),
        _ => Err(host::type_error(&format!("{name} is not a function"))),
    }
}

fn number_method(n: f64, name: &str, args: Vec<Value>) -> Result<Value, String> {
    match name {
        "toFixed" => {
            let digits = arg_num(&args, 0).max(0.0) as usize;
            Ok(new_s(to_fixed(n, digits)))
        }
        "toString" => {
            let radix = args.first().map(|_| arg_num(&args, 0) as u32).unwrap_or(10);
            if radix == 10 || !(2..=36).contains(&radix) {
                Ok(new_s(host::fmt_number(n)))
            } else {
                Ok(new_s(to_radix(n, radix)))
            }
        }
        "toPrecision" => {
            if args.is_empty() {
                Ok(new_s(host::fmt_number(n)))
            } else {
                let p = arg_num(&args, 0) as usize;
                Ok(new_s(to_precision(n, p.max(1))))
            }
        }
        "valueOf" => Ok(Value::Float(n)),
        _ => Err(host::type_error(&format!("{name} is not a function"))),
    }
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
    let a = n.abs();
    // Take the EXACT digits with guard positions past the p-th, then round to p
    // significant digits half away from zero — Rust's `{:.*e}` rounds half to
    // EVEN (`(2.5).toPrecision(1)` would give "2"), but JS rounds half up ("3").
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
            if fraction > 0.5 || (fraction == 0.5 && (digit & 1) == 1) {
                if fraction + delta > 1.0 {
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
            let kv = arg0(&args);
            let vv = args.get(1).cloned().unwrap_or(Value::Undef);
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

fn set_method(recv: &Value, name: &str, args: Vec<Value>) -> Result<Value, String> {
    match name {
        "add" => {
            let vv = arg0(&args);
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
    let obj = with_host(|h| h.new_object(IndexMap::new()));
    // `set_proto` records a null proto as an explicit null-prototype object;
    // undefined leaves the object with the default (bare-object) prototype.
    if !matches!(proto, Value::Undef) {
        with_host(|h| h.set_proto(&obj, proto));
    }
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

fn object_define_property(args: Vec<Value>) -> Result<Value, String> {
    let obj = arg0(&args);
    let key = with_host(|h| h.property_key(&args.get(1).cloned().unwrap_or(Value::Undef)));
    let desc = args.get(2).cloned().unwrap_or(Value::Undef);
    apply_descriptor(&obj, &key, &desc);
    Ok(obj)
}

/// Apply a `{ value | get | set }` descriptor object to `obj[key]`.
fn apply_descriptor(obj: &Value, key: &str, desc: &Value) {
    let (value, get, set) = with_host(|h| match h.get(desc) {
        Some(JsObj::Object(p)) => (
            p.get("value").cloned(),
            p.get("get").cloned(),
            p.get("set").cloned(),
        ),
        _ => (None, None, None),
    });
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
        } else {
            with_host(|h| {
                if let Some(JsObj::Object(p)) = h.get_mut(obj) {
                    p.insert(key.to_string(), v);
                }
            });
        }
    }
}

fn object_get_own_descriptor(args: Vec<Value>) -> Result<Value, String> {
    let obj = arg0(&args);
    let key = with_host(|h| h.property_key(&args.get(1).cloned().unwrap_or(Value::Undef)));
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
            let mut m: IndexMap<String, Value> = IndexMap::new();
            m.insert("get".into(), get.unwrap_or(Value::Undef));
            m.insert("set".into(), set.unwrap_or(Value::Undef));
            m.insert("enumerable".into(), Value::Bool(true));
            m.insert("configurable".into(), Value::Bool(true));
            h.new_object(m)
        }));
    }
    let val = with_host(|h| match h.get(&obj) {
        Some(JsObj::Object(p)) => p.get(&key).cloned(),
        // A function/class own prop lives in the fn-prop side table.
        Some(JsObj::Func(_)) | Some(JsObj::Class(_)) => h.fn_prop(&obj, &key),
        _ => None,
    });
    match val {
        Some(v) => Ok(with_host(|h| {
            let mut m: IndexMap<String, Value> = IndexMap::new();
            m.insert("value".into(), v);
            m.insert("writable".into(), Value::Bool(true));
            m.insert("enumerable".into(), Value::Bool(true));
            m.insert("configurable".into(), Value::Bool(true));
            h.new_object(m)
        })),
        None => Ok(Value::Undef),
    }
}

/// `key in obj` respecting the prototype chain.
pub fn has_property(obj: &Value, key: &str) -> bool {
    // `key in <builtin namespace/prototype>`: membership matches what a property
    // read would yield. `String.prototype.indexOf` (and the rest of the builtin
    // prototype methods) resolve as callable thunks via `namespace_property`, so
    // `'indexOf' in String.prototype` must report true (get-intrinsic probes this
    // with the `in` operator before reading the intrinsic).
    if let Some(JsObj::Builtin(ns)) = with_host(|h| h.get(obj).cloned()) {
        return !matches!(namespace_property(&ns, key), Value::Undef);
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
                    .map(|i| i < items.len())
                    .unwrap_or(false)
        }
        _ => false,
    })
}

/// `structuredClone` — a deep copy of plain data (objects/arrays/primitives).
fn deep_clone(v: &Value) -> Value {
    match with_host(|h| h.get(v).cloned()) {
        Some(JsObj::Array(items)) => {
            let cloned: Vec<Value> = items.iter().map(deep_clone).collect();
            with_host(|h| h.new_array(cloned))
        }
        Some(JsObj::Object(props)) => {
            let cloned: IndexMap<String, Value> = props
                .iter()
                .map(|(k, val)| (k.clone(), deep_clone(val)))
                .collect();
            with_host(|h| h.new_object(cloned))
        }
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
    with_host(|h| h.promise_mark_handled(id)); // avoid spurious unhandled noise
    Ok(p)
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
                            // All rejected → AggregateError (simplified to an Error).
                            let agg = with_host(|h| {
                                synth_error(h, "AggregateError: All promises were rejected")
                            });
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

/// `setTimeout`/`setInterval`/`setImmediate` — register a macrotask. We do NOT
/// implement repeating intervals (each fires once) to keep output deterministic
/// and terminating; the delay orders timers on a virtual clock.
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
    let id = with_host(|h| h.add_timer(delay, cb, extra));
    Value::Float(id as f64)
}

fn clear_timer(v: &Value) {
    let id = with_host(|h| h.to_number(v)) as u64;
    with_host(|h| h.cancel_timer(id));
}
