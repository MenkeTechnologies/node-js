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

/// Read `recv.name` (also the computed-key path for string keys).
fn get_property(recv: &Value, name: &str) -> Result<Value, String> {
    if with_host(|h| h.is_nullish(recv)) {
        return Err(host::type_error(&format!(
            "Cannot read properties of {} (reading '{name}')",
            with_host(|h| h.str_of(recv))
        )));
    }
    let obj = with_host(|h| h.get(recv).cloned());
    Ok(match obj {
        Some(JsObj::Object(props)) => props.get(name).cloned().unwrap_or(Value::Undef),
        Some(JsObj::Array(items)) => {
            if name == "length" {
                Value::Float(items.len() as f64)
            } else if let Ok(i) = name.parse::<usize>() {
                items.get(i).cloned().unwrap_or(Value::Undef)
            } else if is_array_method(name) {
                bound_method(recv, name)
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
            } else if is_string_method(name) {
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

fn bound_method(recv: &Value, name: &str) -> Value {
    with_host(|h| {
        h.alloc(JsObj::BoundMethod {
            recv: recv.clone(),
            name: name.to_string(),
        })
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
    let qualified = format!("{ns}.{name}");
    if is_known_builtin(&qualified) {
        return with_host(|h| h.alloc(JsObj::Builtin(qualified)));
    }
    Value::Undef
}

fn b_setattr(vm: &mut VM, _: u8) -> Value {
    let val = vm.pop();
    let name = sval(&vm.pop());
    let recv = vm.pop();
    set_property(&recv, &name, val.clone());
    val
}

fn set_property(recv: &Value, name: &str, val: Value) {
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
    let key = with_host(|h| h.str_of(&idx));
    match get_property(&recv, &key) {
        Ok(v) => v,
        Err(e) => abort(vm, e),
    }
}

fn b_setitem(vm: &mut VM, _: u8) -> Value {
    let val = vm.pop();
    let idx = vm.pop();
    let recv = vm.pop();
    let key = with_host(|h| h.str_of(&idx));
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
    let mut i = 0;
    while i + 2 < flat.len() || (i + 2 == flat.len() && flat.len() % 3 == 0 && i < flat.len()) {
        if i + 2 >= flat.len() {
            break;
        }
        let spread = matches!(flat[i], Value::Int(1));
        if spread {
            let src = flat[i + 1].clone();
            let entries = with_host(|h| match h.get(&src) {
                Some(JsObj::Object(m)) => m.iter().map(|(k, v)| (k.clone(), v.clone())).collect::<Vec<_>>(),
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
            props.insert(key, flat[i + 2].clone());
        }
        i += 3;
    }
    with_host(|h| h.new_object(props))
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
    with_host(|h| {
        let s = h.str_of(&v);
        h.new_str(s)
    })
}

fn b_typeof(vm: &mut VM, _: u8) -> Value {
    let v = vm.pop();
    with_host(|h| {
        let t = h.type_of(&v);
        h.new_str(t)
    })
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
    let _ctor = vm.pop();
    let _obj = vm.pop();
    // Prototype chains are not modeled; report false (conservative).
    Value::Bool(false)
}

// ── bitwise / unary ───────────────────────────────────────────────────────────

fn b_binop(vm: &mut VM, _: u8) -> Value {
    let b = vm.pop();
    let a = vm.pop();
    let tag = match vm.pop() {
        Value::Int(n) => n,
        _ => 0,
    };
    with_host(|h| h.bitwise(tag, &a, &b))
}

fn b_unary(vm: &mut VM, _: u8) -> Value {
    let v = vm.pop();
    let tag = match vm.pop() {
        Value::Int(n) => n,
        _ => 0,
    };
    with_host(|h| match tag {
        host::unop::POS => Value::Float(h.to_number(&v)),
        host::unop::BITNOT => {
            let n = h.to_number(&v);
            let i = if n.is_finite() { n.trunc() as i64 as i32 } else { 0 };
            Value::Float(!i as f64)
        }
        _ => Value::Undef,
    })
}

// ── membership ────────────────────────────────────────────────────────────────

fn b_contains(vm: &mut VM, _: u8) -> Value {
    let container = vm.pop();
    let key = vm.pop();
    let k = with_host(|h| h.str_of(&key));
    Value::Bool(with_host(|h| match h.get(&container) {
        Some(JsObj::Object(props)) => props.contains_key(&k),
        Some(JsObj::Array(items)) => k.parse::<usize>().map(|i| i < items.len()).unwrap_or(false),
        _ => false,
    }))
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
        let name = props.get("name").map(|x| h.str_of(x)).unwrap_or_else(|| "Error".into());
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
            let thrown = with_host(|h| h.exc.clone()).unwrap_or_else(|| {
                with_host(|h| synth_error(h, &e))
            });
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

/// Synthesize an `Error`-shaped object from an internal error string.
fn synth_error(h: &mut host::JsHost, e: &str) -> Value {
    let (name, message) = match e.split_once(": ") {
        Some((n, m)) => (n.to_string(), m.to_string()),
        None => ("Error".to_string(), e.to_string()),
    };
    let mut props: IndexMap<String, Value> = IndexMap::new();
    let nv = h.new_str(name);
    let mv = h.new_str(message);
    props.insert("name".into(), nv);
    props.insert("message".into(), mv);
    h.new_object(props)
}

// ── iteration ─────────────────────────────────────────────────────────────────

fn b_getiter(vm: &mut VM, _: u8) -> Value {
    let v = vm.pop();
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
    let next = with_host(|h| {
        if let Some(JsObj::Iter { items, idx }) = h.get_mut(&it) {
            if *idx < items.len() {
                let v = items[*idx].clone();
                *idx += 1;
                return Some(v);
            }
        }
        None
    });
    match next {
        Some(v) => {
            vm.push(v);
            Value::Bool(true)
        }
        None => Value::Bool(false),
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
    let items = match with_host(|h| h.iter_vec(&iterable)) {
        Ok(v) => v,
        Err(e) => return abort(vm, e),
    };
    let ordered: Vec<Value> = if star < 0 {
        (0..count).map(|i| items.get(i).cloned().unwrap_or(Value::Undef)).collect()
    } else {
        let si = star as usize;
        let after = count.saturating_sub(si + 1);
        let rest_end = items.len().saturating_sub(after).max(si);
        let mut out: Vec<Value> = Vec::with_capacity(count);
        for i in 0..si {
            out.push(items.get(i).cloned().unwrap_or(Value::Undef));
        }
        let rest: Vec<Value> = items.get(si..rest_end).map(|s| s.to_vec()).unwrap_or_default();
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
            match with_host(|h| h.iter_vec(&val)) {
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
    let args = with_host(|h| h.iter_vec(&args_arr)).unwrap_or_default();
    let r = host::invoke(&callable, args, None);
    finish(vm, r)
}

fn b_apply_method(vm: &mut VM, _: u8) -> Value {
    let args_arr = vm.pop();
    let name = sval(&vm.pop());
    let recv = vm.pop();
    let args = with_host(|h| h.iter_vec(&args_arr)).unwrap_or_default();
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
        "console" | "Math" | "JSON" | "Object" | "Array" | "Number" | "String" | "Boolean"
    )
}

const GLOBAL_FUNCS: &[&str] = &[
    "parseInt",
    "parseFloat",
    "isNaN",
    "isFinite",
    "String",
    "Number",
    "Boolean",
    "Array",
    "Error",
    "TypeError",
    "RangeError",
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
    "Object.fromEntries",
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
];

pub fn is_known_builtin(name: &str) -> bool {
    GLOBAL_FUNCS.contains(&name) || NS_METHODS.contains(&name) || is_namespace(name)
}

/// Call a resolved builtin function (global or `namespace.method`).
pub fn call_builtin_function(name: &str, args: Vec<Value>) -> Result<Value, String> {
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
        "Number.isInteger" => Ok(Value::Bool(is_integer(arg0(&args)))),
        "Number.isSafeInteger" => Ok(Value::Bool(is_safe_integer(arg0(&args)))),
        "Number.isNaN" => Ok(Value::Bool(matches!(arg0(&args), Value::Float(f) if f.is_nan()))),
        "Number.isFinite" => Ok(Value::Bool(matches!(arg0(&args), Value::Float(f) if f.is_finite()) || matches!(arg0(&args), Value::Int(_)))),
        "String" => Ok(with_host(|h| {
            let s = if args.is_empty() { String::new() } else { h.str_of(&args[0]) };
            h.new_str(s)
        })),
        "Number" => Ok(Value::Float(if args.is_empty() { 0.0 } else { with_host(|h| h.to_number(&args[0])) })),
        "Boolean" => Ok(Value::Bool(with_host(|h| h.truthy(&arg0(&args))))),
        "String.fromCharCode" => Ok(with_host(|h| {
            let s: String = args
                .iter()
                .filter_map(|a| char::from_u32(h.to_number(a) as u32))
                .collect();
            h.new_str(s)
        })),
        "Array" | "Array.of" => Ok(with_host(|h| h.new_array(args))),
        "Array.isArray" => Ok(Value::Bool(matches!(with_host(|h| h.get(&arg0(&args)).cloned()), Some(JsObj::Array(_))))),
        "Array.from" => array_from(args),
        "Object.keys" => object_keys(args, 0),
        "Object.values" => object_keys(args, 1),
        "Object.entries" => object_keys(args, 2),
        "Object.assign" => object_assign(args),
        "Object.freeze" => Ok(arg0(&args)),
        "Object.fromEntries" => object_from_entries(args),
        "JSON.stringify" => json_stringify(args),
        "JSON.parse" => json_parse(args),
        "Error" | "TypeError" | "RangeError" => Ok(make_error(name, &args)),
        _ if name.starts_with("Math.") => math_fn(&name[5..], &args),
        _ => Err(host::type_error(&format!("{name} is not a function"))),
    }
}

/// Construct via `new` for the builtin constructors.
pub fn construct_builtin(name: &str, args: Vec<Value>) -> Result<Value, String> {
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
        "Object" => Ok(with_host(|h| h.new_object(IndexMap::new()))),
        "Error" | "TypeError" | "RangeError" => Ok(make_error(name, &args)),
        _ => Err(host::type_error(&format!("{name} is not a constructor"))),
    }
}

fn make_error(name: &str, args: &[Value]) -> Value {
    with_host(|h| {
        let mut props: IndexMap<String, Value> = IndexMap::new();
        let nv = h.new_str(name);
        props.insert("name".into(), nv);
        let msg = args.first().map(|a| h.str_of(a)).unwrap_or_default();
        let mv = h.new_str(msg);
        props.insert("message".into(), mv);
        h.new_object(props)
    })
}

fn print_line(args: &[Value], stderr: bool) {
    let line: String = with_host(|h| {
        args.iter().map(|a| h.console_format(a)).collect::<Vec<_>>().join(" ")
    });
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

fn parse_int(args: &[Value]) -> f64 {
    let s = with_host(|h| h.str_of(&arg0(args)));
    let radix = args.get(1).map(|r| with_host(|h| h.to_number(r)) as u32).filter(|r| (2..=36).contains(r));
    let t = s.trim();
    let (neg, digits) = match t.strip_prefix('-') {
        Some(rest) => (true, rest),
        None => (false, t.strip_prefix('+').unwrap_or(t)),
    };
    let (radix, digits) = match radix {
        Some(16) => (16u32, digits.strip_prefix("0x").or_else(|| digits.strip_prefix("0X")).unwrap_or(digits)),
        Some(r) => (r, digits),
        None => {
            if let Some(hex) = digits.strip_prefix("0x").or_else(|| digits.strip_prefix("0X")) {
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
    let n = i64::from_str_radix(&valid, radix).map(|n| n as f64).unwrap_or(f64::NAN);
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
    let inf_body = t.strip_prefix('+').or_else(|| t.strip_prefix('-')).unwrap_or(t);
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
    let entries: Vec<(String, Value)> = with_host(|h| match h.get(&v) {
        Some(JsObj::Object(props)) => props.iter().map(|(k, val)| (k.clone(), val.clone())).collect(),
        Some(JsObj::Array(items)) => items.iter().enumerate().map(|(i, val)| (i.to_string(), val.clone())).collect(),
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
    let items = with_host(|h| h.iter_vec(&arg0(&args))).unwrap_or_default();
    if let Some(cb) = args.get(1).cloned() {
        let mut out = Vec::with_capacity(items.len());
        for (i, it) in items.into_iter().enumerate() {
            out.push(host::invoke(&cb, vec![it, Value::Float(i as f64)], None)?);
        }
        return Ok(with_host(|h| h.new_array(out)));
    }
    Ok(with_host(|h| h.new_array(items)))
}

// ── JSON ──────────────────────────────────────────────────────────────────────

fn json_stringify(args: Vec<Value>) -> Result<Value, String> {
    let v = arg0(&args);
    let indent = match args.get(2) {
        Some(Value::Float(f)) => " ".repeat((*f as usize).min(10)),
        Some(other) => with_host(|h| h.as_str(other)).unwrap_or_default(),
        None => String::new(),
    };
    let s = with_host(|h| json_str(h, &v, &indent, 0));
    match s {
        Some(s) => Ok(with_host(|h| h.new_str(s))),
        None => Ok(Value::Undef),
    }
}

fn json_str(h: &host::JsHost, v: &Value, indent: &str, depth: usize) -> Option<String> {
    match v {
        Value::Undef => None,
        Value::Bool(b) => Some(if *b { "true".into() } else { "false".into() }),
        Value::Int(n) => Some(n.to_string()),
        Value::Float(f) => Some(if f.is_finite() { host::fmt_number(*f) } else { "null".into() }),
        Value::Str(s) => Some(json_quote(s)),
        Value::Obj(_) => match h.get(v) {
            Some(JsObj::Str(s)) => Some(json_quote(s)),
            Some(JsObj::Null) => Some("null".into()),
            Some(JsObj::Func(_)) | Some(JsObj::Builtin(_)) | Some(JsObj::BoundMethod { .. }) => None,
            Some(JsObj::Array(items)) => {
                if items.is_empty() {
                    return Some("[]".into());
                }
                let parts: Vec<String> = items
                    .iter()
                    .map(|x| json_str(h, x, indent, depth + 1).unwrap_or_else(|| "null".into()))
                    .collect();
                Some(wrap(&parts, "[", "]", indent, depth))
            }
            Some(JsObj::Object(props)) => {
                let parts: Vec<String> = props
                    .iter()
                    .filter_map(|(k, val)| {
                        json_str(h, val, indent, depth + 1).map(|vs| {
                            let sep = if indent.is_empty() { ":" } else { ": " };
                            format!("{}{sep}{vs}", json_quote(k))
                        })
                    })
                    .collect();
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
        format!("{open}\n{pad}{}\n{pad_close}{close}", parts.join(&format!(",\n{pad}")))
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
    let mut p = JsonParser { chars: s.chars().collect(), pos: 0 };
    p.skip_ws();
    let v = p.parse_value()?;
    p.skip_ws();
    Ok(v)
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
        while matches!(self.peek(), Some(' ') | Some('\n') | Some('\t') | Some('\r')) {
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
        while matches!(self.peek(), Some(c) if c.is_ascii_digit() || c == '-' || c == '+' || c == '.' || c == 'e' || c == 'E') {
            self.pos += 1;
        }
        let s: String = self.chars[start..self.pos].iter().collect();
        s.parse::<f64>().map(Value::Float).map_err(|_| "SyntaxError: bad number in JSON".into())
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
                            let h: String = self.chars[self.pos + 1..(self.pos + 5).min(self.chars.len())].iter().collect();
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
        "push" | "pop" | "shift" | "unshift" | "map" | "filter" | "forEach" | "join" | "slice"
            | "indexOf" | "lastIndexOf" | "includes" | "reduce" | "concat" | "reverse" | "sort"
            | "find" | "findIndex" | "some" | "every" | "flat" | "fill" | "splice" | "keys"
            | "values" | "entries" | "flatMap" | "at" | "toString"
    )
}
fn is_string_method(name: &str) -> bool {
    matches!(
        name,
        "toUpperCase" | "toLowerCase" | "charAt" | "charCodeAt" | "codePointAt" | "indexOf"
            | "lastIndexOf" | "includes" | "slice" | "substring" | "substr" | "split" | "trim"
            | "trimStart" | "trimEnd" | "replace" | "replaceAll" | "repeat" | "startsWith"
            | "endsWith" | "padStart" | "padEnd" | "concat" | "at" | "toString" | "valueOf"
    )
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
        Some(JsObj::Object(props)) => {
            if let Some(f) = props.get(name).cloned() {
                host::invoke(&f, args, Some(recv.clone()))
            } else if name == "hasOwnProperty" {
                let k = with_host(|h| h.str_of(&arg0(&args)));
                Ok(Value::Bool(props.contains_key(&k)))
            } else if name == "toString" {
                Ok(with_host(|h| h.new_str("[object Object]")))
            } else {
                Err(host::type_error(&format!(
                    "{} is not a function",
                    name
                )))
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
            let items = array_items(recv);
            let target = arg0(&args);
            Ok(Value::Bool(with_host(|h| items.iter().any(|x| h.strict_eq(x, &target)))))
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
            let val = arg0(&args);
            with_host(|h| {
                if let Some(JsObj::Array(items)) = h.get_mut(recv) {
                    for it in items.iter_mut() {
                        *it = val.clone();
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
                out.push(host::invoke(&cb, vec![it.clone(), Value::Float(i as f64), recv.clone()], None)?);
            }
            Ok(with_host(|h| h.new_array(out)))
        }
        "flatMap" => {
            let items = array_items(recv);
            let cb = arg0(&args);
            let mut out = Vec::new();
            for (i, it) in items.iter().enumerate() {
                let r = host::invoke(&cb, vec![it.clone(), Value::Float(i as f64), recv.clone()], None)?;
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
                let keep = host::invoke(&cb, vec![it.clone(), Value::Float(i as f64), recv.clone()], None)?;
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
                host::invoke(&cb, vec![it.clone(), Value::Float(i as f64), recv.clone()], None)?;
            }
            Ok(Value::Undef)
        }
        "find" => {
            let items = array_items(recv);
            let cb = arg0(&args);
            for (i, it) in items.iter().enumerate() {
                let m = host::invoke(&cb, vec![it.clone(), Value::Float(i as f64), recv.clone()], None)?;
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
                let m = host::invoke(&cb, vec![it.clone(), Value::Float(i as f64), recv.clone()], None)?;
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
                let m = host::invoke(&cb, vec![it.clone(), Value::Float(i as f64), recv.clone()], None)?;
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
                let m = host::invoke(&cb, vec![it.clone(), Value::Float(i as f64), recv.clone()], None)?;
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
                return Err(host::type_error("Reduce of empty array with no initial value"));
            }
            for (i, it) in items.iter().enumerate().skip(start) {
                acc = host::invoke(&cb, vec![acc, it.clone(), Value::Float(i as f64), recv.clone()], None)?;
            }
            Ok(acc)
        }
        "sort" => {
            let mut items = array_items(recv);
            let cmp = args.first().cloned();
            // Insertion sort so we can call the (fallible) JS comparator.
            let mut err: Option<String> = None;
            for i in 1..items.len() {
                let mut j = i;
                while j > 0 {
                    let order = match &cmp {
                        Some(cb) => {
                            match host::invoke(cb, vec![items[j - 1].clone(), items[j].clone()], None) {
                                Ok(v) => with_host(|h| h.to_number(&v)),
                                Err(e) => {
                                    err = Some(e);
                                    0.0
                                }
                            }
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
                    if err.is_some() {
                        break;
                    }
                    if order > 0.0 {
                        items.swap(j - 1, j);
                        j -= 1;
                    } else {
                        break;
                    }
                }
                if err.is_some() {
                    break;
                }
            }
            if let Some(e) = err {
                return Err(e);
            }
            with_host(|h| {
                if let Some(JsObj::Array(a)) = h.get_mut(recv) {
                    *a = items;
                }
            });
            Ok(recv.clone())
        }
        "flat" => {
            let items = array_items(recv);
            let mut out = Vec::new();
            for it in items {
                match with_host(|h| h.get(&it).cloned()) {
                    Some(JsObj::Array(inner)) => out.extend(inner),
                    _ => out.push(it),
                }
            }
            Ok(with_host(|h| h.new_array(out)))
        }
        "keys" => {
            let n = array_items(recv).len();
            let items: Vec<Value> = (0..n).map(|i| Value::Float(i as f64)).collect();
            Ok(with_host(|h| h.alloc(JsObj::Iter { items, idx: 0 })))
        }
        "values" => {
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
            Ok(with_host(|h| h.alloc(JsObj::Iter { items: pairs, idx: 0 })))
        }
        "splice" => array_splice(recv, args),
        "toString" => {
            let s = with_host(|h| h.str_of(recv));
            Ok(with_host(|h| h.new_str(s)))
        }
        _ => Err(host::type_error(&format!("{name} is not a function"))),
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
        "toUpperCase" => Ok(new_s(s.to_uppercase())),
        "toLowerCase" => Ok(new_s(s.to_lowercase())),
        "trim" => Ok(new_s(s.trim().to_string())),
        "trimStart" => Ok(new_s(s.trim_start().to_string())),
        "trimEnd" => Ok(new_s(s.trim_end().to_string())),
        "toString" | "valueOf" => Ok(new_s(s.to_string())),
        "charAt" => {
            let i = arg_num(&args, 0) as usize;
            Ok(new_s(chars.get(i).map(|c| c.to_string()).unwrap_or_default()))
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
        "replace" => {
            let from = with_host(|h| h.str_of(&arg0(&args)));
            let to = with_host(|h| h.str_of(&args.get(1).cloned().unwrap_or(Value::Undef)));
            Ok(new_s(s.replacen(&from, &to, 1)))
        }
        "replaceAll" => {
            let from = with_host(|h| h.str_of(&arg0(&args)));
            let to = with_host(|h| h.str_of(&args.get(1).cloned().unwrap_or(Value::Undef)));
            Ok(new_s(s.replace(&from, &to)))
        }
        "split" => {
            let parts: Vec<Value> = if args.is_empty() || matches!(args[0], Value::Undef) {
                vec![new_s(s.to_string())]
            } else {
                let sep = with_host(|h| h.str_of(&args[0]));
                if sep.is_empty() {
                    chars.iter().map(|c| new_s(c.to_string())).collect()
                } else {
                    s.split(&sep as &str).map(|p| new_s(p.to_string())).collect()
                }
            };
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
    let padding: String = (0..need).map(|i| fill_chars[i % fill_chars.len()]).collect();
    if start {
        format!("{padding}{s}")
    } else {
        format!("{s}{padding}")
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
    let mut digits: Vec<u8> = int_part.bytes().chain(frac_part.bytes()).map(|b| b - b'0').collect();
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
    let frac: String = digits[point..point + f].iter().map(|d| (d + b'0') as char).collect();
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
    let all: Vec<u8> = mant.chars().filter(|c| c.is_ascii_digit()).map(|c| c as u8 - b'0').collect();
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

fn to_radix(n: f64, radix: u32) -> String {
    if !n.is_finite() {
        return host::fmt_number(n);
    }
    let neg = n < 0.0;
    let mut i = n.abs().trunc() as u64;
    if i == 0 {
        return "0".into();
    }
    let digits = b"0123456789abcdefghijklmnopqrstuvwxyz";
    let mut out = Vec::new();
    while i > 0 {
        out.push(digits[(i % radix as u64) as usize]);
        i /= radix as u64;
    }
    if neg {
        out.push(b'-');
    }
    out.reverse();
    String::from_utf8(out).unwrap()
}
