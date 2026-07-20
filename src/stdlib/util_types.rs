//! Node `util.types` — runtime type-tag predicates.
//!
//! Every function here answers "what internal kind is this value?" without any
//! user-observable coercion, mirroring Node's `require('node:util/types')`. Each
//! predicate inspects the argument's `JsObj` heap variant or its hidden
//! `@@native` tag and returns a plain boolean, so classification reuses the exact
//! machinery the rest of the runtime already relies on:
//!   - `Map`/`Set` carry a `weak` flag → the weak/non-weak split;
//!   - `RegExp`/`Promise`/`Generator` are first-class heap variants;
//!   - `Date`/`ArrayBuffer`/`TypedArray`/`Buffer` are plain objects wearing a
//!     `@@native` tag (see `date.rs`/`typedarray.rs`/`buffer.rs`);
//!   - async/generator functions are read off the shared `FuncDef` flags;
//!   - `isNativeError` defers to the real `host::instance_of` against `Error`.
//!
//! Deviations from V8, kept honest (node-js is not V8):
//!   - `isProxy` is always `false` — there is no `Proxy` in node-js.
//!   - No boxed primitives exist (`Number(x)` yields a primitive, never an
//!     object wrapper), so `isBoxedPrimitive` and the `is{Number,String,…}Object`
//!     family are all `false`.
//!   - `isArgumentsObject` is `false`: `arguments` is materialised as a plain
//!     array (see `host.rs`), indistinguishable from any other array here.
//!   - No `SharedArrayBuffer`/`DataView`/`BigInt64Array`, module-namespace, or
//!     external objects → those predicates are `false`.

use crate::host::{with_host, JsObj};
use fusevm::Value;

pub const METHODS: &[&str] = &[
    "isDate",
    "isRegExp",
    "isMap",
    "isSet",
    "isWeakMap",
    "isWeakSet",
    "isPromise",
    "isArrayBuffer",
    "isSharedArrayBuffer",
    "isAnyArrayBuffer",
    "isTypedArray",
    "isDataView",
    "isUint8Array",
    "isUint8ClampedArray",
    "isUint16Array",
    "isUint32Array",
    "isInt8Array",
    "isInt16Array",
    "isInt32Array",
    "isFloat32Array",
    "isFloat64Array",
    "isBigInt64Array",
    "isBigUint64Array",
    "isAsyncFunction",
    "isGeneratorFunction",
    "isGeneratorObject",
    "isProxy",
    "isNativeError",
    "isBoxedPrimitive",
    "isArgumentsObject",
    "isNumberObject",
    "isStringObject",
    "isBooleanObject",
    "isSymbolObject",
    "isBigIntObject",
    "isModuleNamespaceObject",
    "isExternal",
];

pub fn call(method: &str, args: &[Value]) -> Option<Result<Value, String>> {
    let v = args.first().cloned().unwrap_or(Value::Undef);
    let b = |x: bool| Some(Ok(Value::Bool(x)));
    match method {
        "isDate" => b(super::native_tag(&v).as_deref() == Some("Date")),
        "isRegExp" => b(with_host(|h| matches!(h.get(&v), Some(JsObj::RegExp(_))))),
        "isMap" => b(with_host(|h| matches!(h.get(&v), Some(JsObj::Map { weak: false, .. })))),
        "isSet" => b(with_host(|h| matches!(h.get(&v), Some(JsObj::Set { weak: false, .. })))),
        "isWeakMap" => b(with_host(|h| matches!(h.get(&v), Some(JsObj::Map { weak: true, .. })))),
        "isWeakSet" => b(with_host(|h| matches!(h.get(&v), Some(JsObj::Set { weak: true, .. })))),
        "isPromise" => b(with_host(|h| matches!(h.get(&v), Some(JsObj::Promise { .. })))),

        // `ArrayBuffer` is a `@@native`-tagged byte container; node-js has no
        // `SharedArrayBuffer`, so `isAnyArrayBuffer` collapses onto it.
        "isArrayBuffer" => b(super::native_tag(&v).as_deref() == Some("ArrayBuffer")),
        "isSharedArrayBuffer" => b(false),
        "isAnyArrayBuffer" => b(super::native_tag(&v).as_deref() == Some("ArrayBuffer")),
        "isDataView" => b(false),

        // Typed arrays carry `@@native = "TypedArray"` + a `@@kind`; a Node
        // `Buffer` is a `Uint8Array` subclass, so it answers to both `isTypedArray`
        // and `isUint8Array`.
        "isTypedArray" => b(ta_kind(&v).is_some()),
        "isUint8Array" => b(ta_kind(&v).as_deref() == Some("Uint8Array")),
        "isUint8ClampedArray" => b(ta_kind(&v).as_deref() == Some("Uint8ClampedArray")),
        "isUint16Array" => b(ta_kind(&v).as_deref() == Some("Uint16Array")),
        "isUint32Array" => b(ta_kind(&v).as_deref() == Some("Uint32Array")),
        "isInt8Array" => b(ta_kind(&v).as_deref() == Some("Int8Array")),
        "isInt16Array" => b(ta_kind(&v).as_deref() == Some("Int16Array")),
        "isInt32Array" => b(ta_kind(&v).as_deref() == Some("Int32Array")),
        "isFloat32Array" => b(ta_kind(&v).as_deref() == Some("Float32Array")),
        "isFloat64Array" => b(ta_kind(&v).as_deref() == Some("Float64Array")),
        // No BigInt-backed typed arrays in node-js.
        "isBigInt64Array" => b(false),
        "isBigUint64Array" => b(false),

        "isAsyncFunction" => b(func_flag(&v, FuncFlag::Async)),
        "isGeneratorFunction" => b(func_flag(&v, FuncFlag::Generator)),
        "isGeneratorObject" => b(with_host(|h| matches!(h.get(&v), Some(JsObj::Generator { .. })))),

        // No Proxy in node-js; there is nothing that could report `true`.
        "isProxy" => b(false),
        // Reuse the vetted prototype-chain walk: any instance whose chain reaches
        // `Error.prototype` is a native error.
        "isNativeError" => b(is_native_error(&v)),

        // node-js never boxes primitives, so every wrapper-object predicate is
        // structurally `false`.
        "isBoxedPrimitive"
        | "isNumberObject"
        | "isStringObject"
        | "isBooleanObject"
        | "isSymbolObject"
        | "isBigIntObject" => b(false),

        // `arguments` is a plain array here (indistinguishable from any array),
        // and there are no module-namespace / external (N-API) objects.
        "isArgumentsObject" | "isModuleNamespaceObject" | "isExternal" => b(false),

        _ => None,
    }
}

/// The typed-array kind of `v` (`"Uint8Array"`/…/`"Float64Array"`), or `None` if
/// it is not a typed array. A native `Buffer` reports `Uint8Array` (Node models
/// `Buffer` as a `Uint8Array` subclass).
fn ta_kind(v: &Value) -> Option<String> {
    match super::native_tag(v).as_deref() {
        Some("TypedArray") => with_host(|h| match h.get(v) {
            Some(JsObj::Object(p)) => p.get("@@kind").map(|k| h.str_of(k)),
            _ => None,
        }),
        Some("Buffer") => Some("Uint8Array".to_string()),
        _ => None,
    }
}

/// Which `FuncDef` flag a function predicate is asking about.
enum FuncFlag {
    Async,
    Generator,
}

/// True if `v` is a closure whose template carries the requested flag. Extract the
/// `def_id` first (immutable borrow), then read the shared `funcs` table — never
/// nesting two `with_host` borrows.
fn func_flag(v: &Value, flag: FuncFlag) -> bool {
    let def_id = with_host(|h| match h.get(v) {
        Some(JsObj::Func(f)) => Some(f.def_id),
        _ => None,
    });
    with_host(|h| {
        def_id
            .and_then(|id| h.funcs.get(id))
            .map(|d| match flag {
                FuncFlag::Async => d.is_async,
                FuncFlag::Generator => d.is_generator,
            })
            .unwrap_or(false)
    })
}

/// True if `v`'s prototype chain reaches `Error.prototype` — i.e. it is an
/// instance of one of the built-in error constructors.
fn is_native_error(v: &Value) -> bool {
    if !matches!(v, Value::Obj(_)) {
        return false;
    }
    let err_ctor = with_host(|h| h.alloc(JsObj::Builtin("Error".into())));
    crate::host::instance_of(v, &err_ctor).unwrap_or(false)
}
