//! ECMAScript modules (ECMA-262 §16.2) on the existing engine.
//!
//! The pipeline, in the spec's terms:
//!
//! * **ParseModule** — [`crate::parser::parse_module`] parses with the Module
//!   goal and [`analyze`] derives the module record's static fields: its
//!   `[[RequestedModules]]`, `[[ImportEntries]]`, and the local / indirect /
//!   star export entries (16.2.1.6.1), with the early errors that depend on
//!   them (duplicate exports, exporting an undeclared name, an import binding
//!   colliding with a declaration).
//! * **Link / Evaluate** — the module-record algorithms (`InnerModuleLinking`,
//!   `ResolveExport`, `GetExportedNames`, `InnerModuleEvaluation` with the
//!   asynchronous-evaluation machinery for top-level `await`) are ported
//!   step for step in `esm_loader.js`, which drives the records this file
//!   produces through the `@@esm.*` natives in [`native`].
//! * **InitializeEnvironment / ExecuteModule** — [`lower`] turns a module body
//!   into one ordinary function the compiler already knows how to build:
//!
//!   ```text
//!   (function [*] (%imp, %meta, %reg) {
//!       %reg([() => local₀, () => local₁, …]);   // the export bindings
//!       [yield;]                                 // only for a module in a cycle
//!       …body…
//!   })
//!   ```
//!
//!   The function's scope IS the module environment: its hoisted function
//!   declarations exist on entry and its `let`/`const`/`class` bindings sit in
//!   their temporal dead zone until evaluation reaches them. Each exported
//!   binding is handed to the loader as a getter closure over that scope, so a
//!   read is always the binding's CURRENT value (a live binding), and a read
//!   before initialisation throws the TDZ `ReferenceError`. Every reference to
//!   an imported name is rewritten to `%imp[k]()`, a call of the getter the
//!   loader resolved for import entry `k`; an assignment to one throws
//!   `TypeError: Assignment to constant variable.` (an import binding is
//!   immutable, 9.1.1.5.5). A module that is part of an import cycle is built
//!   as a GENERATOR instead, so the loader can run its prologue — creating the
//!   environment and registering its getters — before any module in the cycle
//!   evaluates, which is what lets a cycle call a hoisted function of a module
//!   that has not started yet. An acyclic module runs as a plain function, on
//!   the ordinary stack.

use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};

use crate::ast::*;

/// Hidden parameter names of the lowered module function. No source text can
/// spell a `%`, so they never collide with a user binding.
const IMP: &str = "%imp";
const META: &str = "%meta";
const REG: &str = "%reg";
/// The global the lowered `import(…)` calls (`builtins::global_binding`).
pub const IMPORT_FN: &str = "%esm.import";
/// Prefix a free reference to a CommonJS wrapper name gets in module code, so
/// that it resolves to nothing: `require` is a lazily-bound global of this
/// runtime, but in an ES module it is not defined at all. `arguments` is in
/// the list too: module code runs inside the lowered function, whose own
/// `arguments` the module's top level must not see.
pub const CJS_FREE_PREFIX: &str = "%esm.cjs.";
const CJS_NAMES: &[&str] = &[
    "require",
    "module",
    "exports",
    "__filename",
    "__dirname",
    "arguments",
];

/// One `[[ImportEntries]]` record (16.2.1.6, Table 61).
#[derive(Debug, Clone)]
pub struct ImportEntry {
    /// Index into [`ModuleInfo::requests`].
    pub request: usize,
    /// The imported name, or `None` for `import * as ns` (namespace-object).
    pub import_name: Option<String>,
    pub local: String,
    /// The declaration's `{ … }` clause (see [`ImportDecl::clause`]).
    pub clause: Span,
}

/// One indirect export (`export { a as b } from "m"`, `export * as ns from "m"`).
#[derive(Debug, Clone)]
pub struct IndirectExport {
    pub export_name: String,
    pub request: usize,
    /// `None` is `export * as ns`: the binding is the module's namespace.
    pub import_name: Option<String>,
}

/// The static half of a Source Text Module Record.
#[derive(Debug, Clone, Default)]
pub struct ModuleInfo {
    /// `[[RequestedModules]]`: (specifier, attributes), deduplicated, in source
    /// order of first appearance.
    pub requests: Vec<(String, Vec<(String, String)>)>,
    pub imports: Vec<ImportEntry>,
    /// `(exportName, localName)` — the order of the getters the prologue
    /// registers.
    pub local_exports: Vec<(String, String)>,
    pub indirect_exports: Vec<IndirectExport>,
    /// Request indices of `export * from "m"`.
    pub star_exports: Vec<usize>,
    /// `[[HasTLA]]`: an `await` (or `for await`) outside any function.
    pub has_tla: bool,
}

fn syntax(msg: impl std::fmt::Display) -> String {
    format!("SyntaxError: {msg}")
}

/// The names a statement list declares lexically at its own level (let,
/// const, class, and — in strict code — function declarations).
fn lexical_names(stmts: &[Stmt], out: &mut Vec<String>) {
    for s in stmts {
        match &s.kind {
            StmtKind::Decl { kind, decls } if *kind != DeclKind::Var => {
                for d in decls {
                    crate::compiler::pattern_names(&d.target, out);
                }
            }
            StmtKind::ClassDecl(c) => out.extend(c.name.clone()),
            StmtKind::FuncDecl { name, .. } => out.push(name.clone()),
            _ => {}
        }
    }
}

/// Every name a function body binds at its top: its `var`s (hoisted through
/// blocks) and its own lexical declarations.
fn body_names(stmts: &[Stmt], out: &mut Vec<String>) {
    for s in stmts {
        crate::compiler::collect_var_names(s, out);
    }
    lexical_names(stmts, out);
}

fn add_request(info: &mut ModuleInfo, spec: &str, attrs: &[(String, String)]) -> usize {
    if let Some(i) = info
        .requests
        .iter()
        .position(|(s, a)| s == spec && a.as_slice() == attrs)
    {
        return i;
    }
    info.requests.push((spec.to_string(), attrs.to_vec()));
    info.requests.len() - 1
}

/// ParseModule's static semantics for `stmts` (16.2.1.6.1), plus the module
/// early errors (16.2.1.1).
pub fn analyze(stmts: &[Stmt]) -> Result<ModuleInfo, String> {
    let mut info = ModuleInfo::default();
    // Declared names of the module scope, for the export/import checks.
    let mut declared: Vec<String> = Vec::new();
    for s in stmts {
        match &s.kind {
            StmtKind::Export(ExportDecl::Decl(d))
            | StmtKind::Export(ExportDecl::DefaultDecl(d)) => {
                crate::compiler::collect_var_names(d, &mut declared);
                lexical_names(std::slice::from_ref(&**d), &mut declared);
            }
            StmtKind::Export(_) | StmtKind::Import(_) => {}
            _ => {
                crate::compiler::collect_var_names(s, &mut declared);
                lexical_names(std::slice::from_ref(s), &mut declared);
            }
        }
    }
    // Import entries first: an `export { x }` of an imported `x` re-exports it.
    for s in stmts {
        if let StmtKind::Import(d) = &s.kind {
            let req = add_request(&mut info, &d.specifier, &d.attributes);
            for spec in &d.specs {
                let (import_name, local) = match spec {
                    ImportSpec::Default(l) => (Some("default".to_string()), l.clone()),
                    ImportSpec::Namespace(l) => (None, l.clone()),
                    ImportSpec::Named { imported, local } => {
                        (Some(imported.clone()), local.clone())
                    }
                };
                if declared.contains(&local) || info.imports.iter().any(|e| e.local == local) {
                    return Err(syntax(format!(
                        "Identifier '{local}' has already been declared"
                    )));
                }
                info.imports.push(ImportEntry {
                    request: req,
                    import_name,
                    local,
                    clause: d.clause,
                });
            }
        }
    }
    // Two `export default` declarations both bind `*default*`, which V8
    // reports as a redeclaration of its internal name.
    let mut default_seen = false;
    let default_decl = |seen: &mut bool| -> Result<(), String> {
        if std::mem::replace(seen, true) {
            return Err(syntax("Identifier '.default' has already been declared"));
        }
        Ok(())
    };
    let mut exported: Vec<String> = Vec::new();
    let mut note_export = |name: &str| -> Result<(), String> {
        if exported.iter().any(|e| e == name) {
            return Err(syntax(format!("Duplicate export of '{name}'")));
        }
        exported.push(name.to_string());
        Ok(())
    };
    for s in stmts {
        let StmtKind::Export(e) = &s.kind else {
            continue;
        };
        match e {
            ExportDecl::Decl(d) => {
                let mut names = Vec::new();
                crate::compiler::collect_var_names(d, &mut names);
                lexical_names(std::slice::from_ref(&**d), &mut names);
                for n in names {
                    note_export(&n)?;
                    info.local_exports.push((n.clone(), n));
                }
            }
            ExportDecl::DefaultDecl(d) => {
                default_decl(&mut default_seen)?;
                note_export("default")?;
                let mut names = Vec::new();
                lexical_names(std::slice::from_ref(&**d), &mut names);
                let local = names.pop().unwrap_or_else(|| "default".to_string());
                info.local_exports.push(("default".into(), local));
            }
            ExportDecl::DefaultExpr(_) => {
                default_decl(&mut default_seen)?;
                note_export("default")?;
                info.local_exports
                    .push(("default".into(), "default".into()));
            }
            ExportDecl::Named {
                specs, from: None, ..
            } => {
                for sp in specs {
                    note_export(&sp.exported)?;
                    match info.imports.iter().find(|i| i.local == sp.local) {
                        // Re-exporting a named import is an indirect export of
                        // the binding it imports; a namespace import is a local
                        // binding of its own (16.2.1.6.1 step 10.a).
                        Some(ImportEntry {
                            request,
                            import_name: Some(name),
                            ..
                        }) => info.indirect_exports.push(IndirectExport {
                            export_name: sp.exported.clone(),
                            request: *request,
                            import_name: Some(name.clone()),
                        }),
                        Some(_) => info
                            .local_exports
                            .push((sp.exported.clone(), sp.local.clone())),
                        None if declared.contains(&sp.local) => info
                            .local_exports
                            .push((sp.exported.clone(), sp.local.clone())),
                        None => {
                            return Err(syntax(format!(
                                "Export '{}' is not defined in module",
                                sp.local
                            )))
                        }
                    }
                }
            }
            ExportDecl::Named {
                specs,
                from: Some(from),
                attributes,
            } => {
                let req = add_request(&mut info, from, attributes);
                for sp in specs {
                    note_export(&sp.exported)?;
                    info.indirect_exports.push(IndirectExport {
                        export_name: sp.exported.clone(),
                        request: req,
                        import_name: Some(sp.local.clone()),
                    });
                }
            }
            ExportDecl::Star {
                alias,
                from,
                attributes,
            } => {
                let req = add_request(&mut info, from, attributes);
                match alias {
                    Some(a) => {
                        note_export(a)?;
                        info.indirect_exports.push(IndirectExport {
                            export_name: a.clone(),
                            request: req,
                            import_name: None,
                        });
                    }
                    None => info.star_exports.push(req),
                }
            }
        }
    }
    info.has_tla = stmts.iter().any(stmt_has_tla);
    Ok(info)
}

/// Whether evaluating `s` at the module top level can `await` — an `await`
/// or `for await` that is not inside a nested function.
fn stmt_has_tla(s: &Stmt) -> bool {
    let mut found = false;
    let mut probe = AwaitProbe { found: &mut found };
    probe.stmt(s);
    found
}

struct AwaitProbe<'a> {
    found: &'a mut bool,
}

impl AwaitProbe<'_> {
    fn stmt(&mut self, s: &Stmt) {
        if *self.found {
            return;
        }
        match &s.kind {
            StmtKind::ForOf { is_await: true, .. } => *self.found = true,
            StmtKind::FuncDecl { .. } => {}
            _ => walk_stmt_children(s, &mut |n| match n {
                Node::S(st) => self.stmt(st),
                Node::E(e) => self.expr(e),
            }),
        }
    }
    fn expr(&mut self, e: &Expr) {
        if *self.found {
            return;
        }
        match e {
            Expr::Await(_) => *self.found = true,
            // A nested function's `await` belongs to it.
            Expr::Function { .. } => {}
            // Class bodies are functions too, but a computed key or the
            // heritage runs in the enclosing scope.
            Expr::Class(c) => self.class_outer(c),
            _ => walk_expr_children(e, &mut |x| self.expr(x)),
        }
    }
    fn class_outer(&mut self, c: &ClassNode) {
        if let Some(p) = &c.parent {
            self.expr(p);
        }
        for m in &c.members {
            if m.computed {
                self.expr(&m.key);
            }
        }
    }
}

/// Visit the direct child statements and expressions of `s` (read-only).
/// A child of a statement, as [`walk_stmt_children`] hands it over.
enum Node<'a> {
    S(&'a Stmt),
    E(&'a Expr),
}

fn walk_stmt_children<'a>(s: &'a Stmt, f: &mut dyn FnMut(Node<'a>)) {
    match &s.kind {
        StmtKind::Expr(e) | StmtKind::Throw(e) => f(Node::E(e)),
        StmtKind::Return(e) => e.iter().for_each(|e| f(Node::E(e))),
        StmtKind::Decl { decls, .. } => {
            for d in decls {
                f(Node::E(&d.target));
                if let Some(i) = &d.init {
                    f(Node::E(i));
                }
            }
        }
        StmtKind::Block(b) => b.iter().for_each(|s| f(Node::S(s))),
        StmtKind::FuncDecl { .. }
        | StmtKind::Break(_)
        | StmtKind::Continue(_)
        | StmtKind::Empty => {}
        StmtKind::ClassDecl(c) => {
            if let Some(p) = &c.parent {
                f(Node::E(p));
            }
            for m in &c.members {
                if m.computed {
                    f(Node::E(&m.key));
                }
            }
        }
        StmtKind::If { test, cons, alt } => {
            f(Node::E(test));
            f(Node::S(cons));
            if let Some(a) = alt {
                f(Node::S(a));
            }
        }
        StmtKind::While { test, body } | StmtKind::DoWhile { body, test } => {
            f(Node::E(test));
            f(Node::S(body));
        }
        StmtKind::With { object, body } => {
            f(Node::E(object));
            f(Node::S(body));
        }
        StmtKind::For {
            init,
            test,
            update,
            body,
        } => {
            if let Some(i) = init {
                f(Node::S(i));
            }
            test.iter().for_each(|e| f(Node::E(e)));
            update.iter().for_each(|e| f(Node::E(e)));
            f(Node::S(body));
        }
        StmtKind::ForOf {
            target, iter, body, ..
        } => {
            f(Node::E(target));
            f(Node::E(iter));
            f(Node::S(body));
        }
        StmtKind::ForIn {
            target,
            object,
            body,
            ..
        } => {
            f(Node::E(target));
            f(Node::E(object));
            f(Node::S(body));
        }
        StmtKind::Switch { disc, cases } => {
            f(Node::E(disc));
            for c in cases {
                c.test.iter().for_each(|e| f(Node::E(e)));
                c.body.iter().for_each(|s| f(Node::S(s)));
            }
        }
        StmtKind::Labeled { body, .. } => f(Node::S(body)),
        StmtKind::Try {
            block,
            handler,
            finalizer,
        } => {
            block.iter().for_each(|s| f(Node::S(s)));
            if let Some((p, b)) = handler {
                p.iter().for_each(|e| f(Node::E(e)));
                b.iter().for_each(|s| f(Node::S(s)));
            }
            if let Some(fin) = finalizer {
                fin.iter().for_each(|s| f(Node::S(s)));
            }
        }
        StmtKind::Import(_) => {}
        StmtKind::Export(e) => match e {
            ExportDecl::Decl(d) | ExportDecl::DefaultDecl(d) => f(Node::S(d)),
            ExportDecl::DefaultExpr(x) => f(Node::E(x)),
            _ => {}
        },
    }
}

/// Visit the direct child expressions of `e` (read-only, not into bodies of
/// nested functions' statements).
fn walk_expr_children(e: &Expr, ex: &mut dyn FnMut(&Expr)) {
    match e {
        Expr::Template { exprs, .. } => exprs.iter().for_each(&mut *ex),
        Expr::TaggedTemplate { tag, exprs, .. } => {
            ex(tag);
            exprs.iter().for_each(&mut *ex);
        }
        Expr::Yield { arg, .. } => arg.iter().for_each(|x| ex(x)),
        Expr::Await(x) | Expr::Spread(x) | Expr::Unary(_, x) => ex(x),
        Expr::Array(items) | Expr::Sequence(items) => items.iter().for_each(&mut *ex),
        Expr::Object(props) => {
            for p in props {
                match p {
                    Prop::KeyValue {
                        key,
                        value,
                        computed,
                    } => {
                        if *computed {
                            ex(key);
                        }
                        ex(value);
                    }
                    Prop::Spread(x) => ex(x),
                    Prop::Accessor { key, computed, .. } => {
                        if *computed {
                            ex(key);
                        }
                    }
                }
            }
        }
        Expr::Logical(_, l, r) | Expr::Binary(_, l, r) => {
            ex(l);
            ex(r);
        }
        Expr::Conditional { test, cons, alt } => {
            ex(test);
            ex(cons);
            ex(alt);
        }
        Expr::Assign { target, value, .. } => {
            ex(target);
            ex(value);
        }
        Expr::Update { target, .. } => ex(target),
        Expr::Call { func, args, .. } => {
            ex(func);
            args.iter().for_each(&mut *ex);
        }
        Expr::New { callee, args } => {
            ex(callee);
            args.iter().for_each(&mut *ex);
        }
        Expr::Member { object, .. } => ex(object),
        Expr::Index { object, index, .. } => {
            ex(object);
            ex(index);
        }
        Expr::Class(_) | Expr::Function { .. } => {}
        Expr::Null
        | Expr::Undefined
        | Expr::True
        | Expr::False
        | Expr::Number(_)
        | Expr::BigInt(_)
        | Expr::Regex(..)
        | Expr::Str(_)
        | Expr::Ident(_)
        | Expr::This
        | Expr::Super
        | Expr::NewTarget
        | Expr::Hole => {}
    }
}

// ── lowering ────────────────────────────────────────────────────────────────

/// `(() => { throw new TypeError("Assignment to constant variable."); })()` —
/// what a write to an import binding becomes (SetMutableBinding on an
/// immutable binding, 9.1.1.5.5 / 9.1.1.1.5).
fn const_assignment_throw() -> Expr {
    let throw = Stmt::from(StmtKind::Throw(Expr::New {
        callee: Box::new(Expr::Ident("TypeError".into())),
        args: vec![Expr::Str("Assignment to constant variable.".into())],
    }));
    Expr::Call {
        func: Box::new(arrow(FnBody::Block(vec![throw]))),
        args: Vec::new(),
        optional: false,
    }
}

/// `({ set v(_) { throw new TypeError("Assignment to constant variable.") } }).v`
/// — a reference to stand in for an import binding as a destructuring or
/// `for`-head target: writing it throws what writing the binding must.
fn const_assignment_target() -> Expr {
    let Expr::Call { func, .. } = const_assignment_throw() else {
        unreachable!("const_assignment_throw builds a call")
    };
    let Expr::Function { body, .. } = *func else {
        unreachable!("the callee is an arrow")
    };
    let setter = Expr::Function {
        params: vec![Param {
            pattern: Expr::Ident("_".into()),
            default: None,
            rest: false,
        }],
        body,
        is_arrow: false,
        name: None,
        is_generator: false,
        is_async: false,
        is_method: true,
        span: (0, 0),
    };
    Expr::Member {
        object: Box::new(Expr::Object(vec![Prop::Accessor {
            key: Expr::Str("v".into()),
            computed: false,
            is_getter: false,
            func: setter,
        }])),
        property: "v".into(),
        optional: false,
    }
}

fn arrow(body: FnBody) -> Expr {
    Expr::Function {
        params: Vec::new(),
        body,
        is_arrow: true,
        name: None,
        is_generator: false,
        is_async: false,
        is_method: false,
        span: (0, 0),
    }
}

/// Rewrites one module body: import references, `import.meta`, `import(…)`,
/// and free CommonJS names, honouring every nested scope that shadows them.
struct Rewriter<'a> {
    /// Import local name → its index in `%imp`.
    imports: HashMap<String, usize>,
    /// The CommonJS wrapper names the module does not itself declare.
    cjs_free: Vec<&'static str>,
    /// Names bound by the scopes enclosing the current point, innermost last.
    scopes: Vec<HashSet<String>>,
    referrer: &'a str,
}

impl Rewriter<'_> {
    fn shadowed(&self, n: &str) -> bool {
        self.scopes.iter().any(|s| s.contains(n))
    }
    fn import_index(&self, n: &str) -> Option<usize> {
        self.imports.get(n).copied().filter(|_| !self.shadowed(n))
    }
    fn import_read(k: usize) -> Expr {
        Expr::Call {
            func: Box::new(Expr::Index {
                object: Box::new(Expr::Ident(IMP.into())),
                index: Box::new(Expr::Number(k as f64)),
                optional: false,
            }),
            args: Vec::new(),
            optional: false,
        }
    }
    fn with_scope<T>(&mut self, names: Vec<String>, f: impl FnOnce(&mut Self) -> T) -> T {
        self.scopes.push(names.into_iter().collect());
        let out = f(self);
        self.scopes.pop();
        out
    }
    fn stmts(&mut self, stmts: &mut [Stmt]) {
        for s in stmts {
            self.stmt(s);
        }
    }

    fn block(&mut self, stmts: &mut [Stmt]) {
        let mut names = Vec::new();
        lexical_names(stmts, &mut names);
        self.with_scope(names, |r| r.stmts(stmts));
    }

    fn stmt(&mut self, s: &mut Stmt) {
        match &mut s.kind {
            StmtKind::Expr(e) | StmtKind::Throw(e) => self.expr(e),
            StmtKind::Return(e) => {
                if let Some(e) = e {
                    self.expr(e);
                }
            }
            StmtKind::Decl { decls, .. } => {
                for d in decls {
                    self.binding_pattern(&mut d.target);
                    if let Some(i) = &mut d.init {
                        self.expr(i);
                    }
                }
            }
            StmtKind::Block(b) => self.block(b),
            StmtKind::FuncDecl { params, body, .. } => self.function(params, body, None, false),
            StmtKind::ClassDecl(c) => self.class(c),
            StmtKind::If { test, cons, alt } => {
                self.expr(test);
                self.stmt(cons);
                if let Some(a) = alt {
                    self.stmt(a);
                }
            }
            StmtKind::While { test, body } | StmtKind::DoWhile { body, test } => {
                self.expr(test);
                self.stmt(body);
            }
            StmtKind::With { object, body } => {
                self.expr(object);
                self.stmt(body);
            }
            StmtKind::For {
                init,
                test,
                update,
                body,
            } => {
                let mut names = Vec::new();
                if let Some(i) = init {
                    lexical_names(std::slice::from_ref(&**i), &mut names);
                }
                self.with_scope(names, |r| {
                    if let Some(i) = init {
                        r.stmt(i);
                    }
                    if let Some(t) = test {
                        r.expr(t);
                    }
                    if let Some(u) = update {
                        r.expr(u);
                    }
                    r.stmt(body);
                });
            }
            StmtKind::ForOf {
                decl_kind,
                target,
                iter,
                body,
                ..
            }
            | StmtKind::ForIn {
                decl_kind,
                target,
                object: iter,
                body,
                ..
            } => {
                let mut names = Vec::new();
                if matches!(decl_kind, Some(DeclKind::Let) | Some(DeclKind::Const)) {
                    crate::compiler::pattern_names(target, &mut names);
                }
                let declares = decl_kind.is_some();
                self.with_scope(names, |r| {
                    r.expr(iter);
                    if declares {
                        r.binding_pattern(target);
                    } else {
                        r.assignment_pattern(target);
                    }
                    r.stmt(body);
                });
            }
            StmtKind::Switch { disc, cases } => {
                self.expr(disc);
                let mut names = Vec::new();
                for c in cases.iter() {
                    lexical_names(&c.body, &mut names);
                }
                self.with_scope(names, |r| {
                    for c in cases {
                        if let Some(t) = &mut c.test {
                            r.expr(t);
                        }
                        r.stmts(&mut c.body);
                    }
                });
            }
            StmtKind::Labeled { body, .. } => self.stmt(body),
            StmtKind::Try {
                block,
                handler,
                finalizer,
            } => {
                self.block(block);
                if let Some((param, body)) = handler {
                    let mut names = Vec::new();
                    if let Some(p) = param {
                        crate::compiler::pattern_names(p, &mut names);
                    }
                    lexical_names(body, &mut names);
                    self.with_scope(names, |r| {
                        if let Some(p) = param {
                            r.binding_pattern(p);
                        }
                        r.stmts(body);
                    });
                }
                if let Some(f) = finalizer {
                    self.block(f);
                }
            }
            StmtKind::Break(_) | StmtKind::Continue(_) | StmtKind::Empty => {}
            StmtKind::Import(_) | StmtKind::Export(_) => {}
        }
    }

    /// A function's parameters and body, in a scope binding its parameters,
    /// its `var`s and top-level declarations, its own name (a named function
    /// expression) and, unless it is an arrow, `arguments`.
    fn function(
        &mut self,
        params: &mut [Param],
        body: &mut [Stmt],
        own: Option<&str>,
        arrow: bool,
    ) {
        let mut names: Vec<String> = own.map(str::to_string).into_iter().collect();
        if !arrow {
            names.push("arguments".into());
        }
        for p in params.iter() {
            crate::compiler::pattern_names(&p.pattern, &mut names);
        }
        body_names(body, &mut names);
        self.with_scope(names, |r| {
            for p in params.iter_mut() {
                r.binding_pattern(&mut p.pattern);
                if let Some(d) = &mut p.default {
                    r.expr(d);
                }
            }
            r.stmts(body);
        });
    }

    fn class(&mut self, c: &mut ClassNode) {
        let names: Vec<String> = c.name.clone().into_iter().collect();
        self.with_scope(names, |r| {
            if let Some(p) = &mut c.parent {
                r.expr(p);
            }
            for m in &mut c.members {
                if m.computed {
                    r.expr(&mut m.key);
                }
                match m.kind {
                    MemberKind::Field => {
                        if let Some(i) = &mut m.field_init {
                            r.with_scope(vec!["arguments".into()], |r| r.expr(i));
                        }
                    }
                    _ => r.function(&mut m.params, &mut m.body, None, false),
                }
            }
        });
    }

    /// A declaration target: its names are bindings, but default values and
    /// computed keys inside it are expressions.
    fn binding_pattern(&mut self, target: &mut Expr) {
        match target {
            Expr::Ident(_) => {}
            Expr::Array(items) => items.iter_mut().for_each(|i| self.binding_pattern(i)),
            Expr::Object(props) => {
                for p in props {
                    match p {
                        Prop::KeyValue {
                            key,
                            value,
                            computed,
                        } => {
                            if *computed {
                                self.expr(key);
                            }
                            self.binding_pattern(value);
                        }
                        Prop::Spread(e) => self.binding_pattern(e),
                        Prop::Accessor { .. } => {}
                    }
                }
            }
            Expr::Assign { target, value, .. } => {
                self.binding_pattern(target);
                self.expr(value);
            }
            Expr::Spread(inner) => self.binding_pattern(inner),
            other => self.expr(other),
        }
    }

    /// An assignment target pattern: member targets and defaults are
    /// expressions; a plain name is a reference (checked by the caller).
    fn assignment_pattern(&mut self, target: &mut Expr) {
        match target {
            // An import binding inside a destructuring target or a `for`
            // head is written through a reference whose PutValue throws, so
            // the elements before it are still assigned, in order.
            Expr::Ident(n) if self.import_index(n).is_some() => *target = const_assignment_target(),
            Expr::Ident(_) => self.expr(target),
            Expr::Array(items) => items.iter_mut().for_each(|i| self.assignment_pattern(i)),
            Expr::Object(props) => {
                for p in props {
                    match p {
                        Prop::KeyValue {
                            key,
                            value,
                            computed,
                        } => {
                            if *computed {
                                self.expr(key);
                            }
                            self.assignment_pattern(value);
                        }
                        Prop::Spread(e) => self.assignment_pattern(e),
                        Prop::Accessor { .. } => {}
                    }
                }
            }
            Expr::Assign { target, value, .. } => {
                self.assignment_pattern(target);
                self.expr(value);
            }
            Expr::Spread(inner) => self.assignment_pattern(inner),
            other => self.expr(other),
        }
    }

    fn expr(&mut self, e: &mut Expr) {
        match e {
            Expr::Ident(n) => {
                if let Some(k) = self.import_index(n) {
                    *e = Self::import_read(k);
                } else if self.cjs_free.contains(&n.as_str()) && !self.shadowed(n) {
                    *n = format!("{CJS_FREE_PREFIX}{n}");
                }
            }
            Expr::Member {
                object, property, ..
            } if matches!(&**object, Expr::Ident(i) if i == "import") && property == "meta" => {
                *e = Expr::Ident(META.into());
            }
            Expr::Call { func, args, .. } if matches!(&**func, Expr::Ident(i) if i == "import") => {
                for a in args.iter_mut() {
                    self.expr(a);
                }
                let mut full = Vec::with_capacity(3);
                let mut it = std::mem::take(args).into_iter();
                full.push(it.next().unwrap_or(Expr::Undefined));
                full.push(Expr::Str(self.referrer.to_string()));
                full.extend(it.next());
                *e = Expr::Call {
                    func: Box::new(Expr::Ident(IMPORT_FN.into())),
                    args: full,
                    optional: false,
                };
            }
            Expr::Assign { target, op, value } => {
                let hits = matches!(&**target, Expr::Ident(n) if self.import_index(n).is_some());
                if !hits {
                    // A plain name that is not an import is still rewritten
                    // when it is a free CommonJS name (`exports = …`).
                    if matches!(&**target, Expr::Array(_) | Expr::Object(_)) {
                        self.assignment_pattern(target);
                    } else {
                        self.expr(target);
                    }
                    self.expr(value);
                    return;
                }
                self.expr(value);
                let value = std::mem::replace(&mut **value, Expr::Undefined);
                let throw = const_assignment_throw();
                *e = match (op.as_ref(), &**target) {
                    // `x ||= v` only writes when the test lets it through.
                    (Some(AssignOp::Logical(lop)), Expr::Ident(n)) => {
                        let read = Self::import_read(self.import_index(n).unwrap_or(0));
                        Expr::Logical(
                            *lop,
                            Box::new(read),
                            Box::new(Expr::Sequence(vec![value, throw])),
                        )
                    }
                    (Some(AssignOp::Binary(bop)), Expr::Ident(n)) => {
                        let read = Self::import_read(self.import_index(n).unwrap_or(0));
                        Expr::Sequence(vec![
                            Expr::Binary(*bop, Box::new(read), Box::new(value)),
                            throw,
                        ])
                    }
                    _ => Expr::Sequence(vec![value, throw]),
                };
            }
            Expr::Update { target, .. } => {
                if let Expr::Ident(n) = &**target {
                    if let Some(k) = self.import_index(n) {
                        let read = Expr::Unary(UnOp::Pos, Box::new(Self::import_read(k)));
                        *e = Expr::Sequence(vec![read, const_assignment_throw()]);
                        return;
                    }
                }
                self.expr(target);
            }
            Expr::Function {
                params,
                body,
                is_arrow,
                name,
                ..
            } => {
                let arrow = *is_arrow;
                let own = name.clone();
                match body {
                    FnBody::Block(b) => self.function(params, b, own.as_deref(), arrow),
                    FnBody::Expr(x) => {
                        let mut stmts = Vec::new();
                        self.function(params, &mut stmts, own.as_deref(), arrow);
                        let mut names: Vec<String> = Vec::new();
                        for p in params.iter() {
                            crate::compiler::pattern_names(&p.pattern, &mut names);
                        }
                        if !arrow {
                            names.push("arguments".into());
                        }
                        self.with_scope(names, |r| r.expr(x));
                    }
                }
            }
            Expr::Class(c) => self.class(c),
            Expr::Object(props) => {
                for p in props {
                    match p {
                        Prop::KeyValue {
                            key,
                            value,
                            computed,
                        } => {
                            if *computed {
                                self.expr(key);
                            }
                            self.expr(value);
                        }
                        Prop::Spread(x) => self.expr(x),
                        Prop::Accessor {
                            key,
                            computed,
                            func,
                            ..
                        } => {
                            if *computed {
                                self.expr(key);
                            }
                            self.expr(func);
                        }
                    }
                }
            }
            Expr::Template { exprs, .. } => exprs.iter_mut().for_each(|x| self.expr(x)),
            Expr::TaggedTemplate { tag, exprs, .. } => {
                self.expr(tag);
                exprs.iter_mut().for_each(|x| self.expr(x));
            }
            Expr::Yield { arg, .. } => {
                if let Some(a) = arg {
                    self.expr(a);
                }
            }
            Expr::Await(x) | Expr::Spread(x) | Expr::Unary(_, x) => self.expr(x),
            Expr::Array(items) | Expr::Sequence(items) => {
                items.iter_mut().for_each(|x| self.expr(x))
            }
            Expr::Logical(_, l, r) | Expr::Binary(_, l, r) => {
                self.expr(l);
                self.expr(r);
            }
            Expr::Conditional { test, cons, alt } => {
                self.expr(test);
                self.expr(cons);
                self.expr(alt);
            }
            Expr::Call { func, args, .. } => {
                self.expr(func);
                args.iter_mut().for_each(|x| self.expr(x));
            }
            Expr::New { callee, args } => {
                self.expr(callee);
                args.iter_mut().for_each(|x| self.expr(x));
            }
            Expr::Member { object, .. } => self.expr(object),
            Expr::Index { object, index, .. } => {
                self.expr(object);
                self.expr(index);
            }
            Expr::Null
            | Expr::Undefined
            | Expr::True
            | Expr::False
            | Expr::Number(_)
            | Expr::BigInt(_)
            | Expr::Regex(..)
            | Expr::Str(_)
            | Expr::This
            | Expr::Super
            | Expr::NewTarget
            | Expr::Hole => {}
        }
    }
}

/// Lower a parsed module body (see the module docs) to the function
/// expression the loader instantiates. `referrer` is the module's filename,
/// which every `import(…)` in it resolves against; `generator` selects the
/// form a module in an import cycle needs.
pub fn lower(stmts: Vec<Stmt>, info: &ModuleInfo, referrer: &str, generator: bool) -> Expr {
    let mut body: Vec<Stmt> = Vec::with_capacity(stmts.len() + 2);
    for s in stmts {
        let line = s.line;
        match s.kind {
            StmtKind::Import(_) => {}
            StmtKind::Export(ExportDecl::Decl(d))
            | StmtKind::Export(ExportDecl::DefaultDecl(d)) => body.push(*d),
            // `export default <expr>` binds the expression's value to the
            // `*default*` binding (spelled `default`, which no source name can
            // reach), and an anonymous function or class is named "default".
            StmtKind::Export(ExportDecl::DefaultExpr(e)) => body.push(Stmt::new(
                StmtKind::Decl {
                    kind: DeclKind::Const,
                    decls: vec![Declarator {
                        target: Expr::Ident("default".into()),
                        init: Some(e),
                    }],
                },
                line,
            )),
            StmtKind::Export(_) => {}
            other => body.push(Stmt::new(other, line)),
        }
    }
    let mut declared = Vec::new();
    body_names(&body, &mut declared);
    let mut rw = Rewriter {
        imports: info
            .imports
            .iter()
            .enumerate()
            .map(|(k, e)| (e.local.clone(), k))
            .collect(),
        cjs_free: CJS_NAMES
            .iter()
            .copied()
            .filter(|n| !declared.iter().any(|d| d == n))
            .collect(),
        scopes: Vec::new(),
        referrer,
    };
    rw.stmts(&mut body);
    // The getter for each local export, in `local_exports` order. A getter of
    // a re-exported namespace import reads through `%imp` like any reference.
    let getters: Vec<Expr> = info
        .local_exports
        .iter()
        .map(|(_, local)| {
            let mut read = Expr::Ident(local.clone());
            rw.expr(&mut read);
            arrow(FnBody::Expr(Box::new(read)))
        })
        .collect();
    let mut prologue = vec![Stmt::from(StmtKind::Expr(Expr::Call {
        func: Box::new(Expr::Ident(REG.into())),
        args: vec![Expr::Array(getters)],
        optional: false,
    }))];
    if generator {
        prologue.push(Stmt::from(StmtKind::Expr(Expr::Yield {
            arg: None,
            delegate: false,
        })));
    }
    prologue.extend(body);
    let param = |n: &str| Param {
        pattern: Expr::Ident(n.into()),
        default: None,
        rest: false,
    };
    Expr::Function {
        params: vec![param(IMP), param(META), param(REG)],
        body: FnBody::Block(prologue),
        is_arrow: false,
        name: None,
        is_generator: generator,
        is_async: info.has_tla,
        is_method: false,
        span: (0, 0),
    }
}

// ── format detection ────────────────────────────────────────────────────────

/// The script-goal parse errors that only module syntax produces. Node's
/// syntax detection (`containsModuleSyntax`) runs a source with no declared
/// format as an ES module exactly when its CommonJS parse fails with one of
/// these and its module parse succeeds.
const MODULE_ONLY_ERRORS: &[&str] = &[
    "SyntaxError: Cannot use import statement outside a module",
    "SyntaxError: Unexpected token 'export'",
    "SyntaxError: Cannot use 'import.meta' outside a module",
    "SyntaxError: await is only valid in async functions and the top level bodies of modules",
];

/// Node's syntax detection, given the error the CommonJS (script-goal)
/// compile of `src` failed with: the source is an ES module when module
/// syntax explains that error and the module-goal parse succeeds.
///
/// A top-level `await` is recognised by its token rather than by V8's exact
/// message, because where an `await` operand sits decides which message a
/// script parse reports (`f(await x)` is "missing ) after argument list").
pub fn detect_module(src: &str, script_err: &str) -> bool {
    let explained = MODULE_ONLY_ERRORS.iter().any(|m| script_err.starts_with(m))
        || script_err.starts_with("SyntaxError") && src.contains("await");
    explained && crate::parser::parse_module(src).is_ok()
}

/// Node's syntax detection for a source of undeclared format.
pub fn has_module_syntax(src: &str) -> bool {
    match crate::parser::parse(src) {
        Ok(_) => false,
        Err(e) => detect_module(src, &e),
    }
}

/// What a resolved module is (node's `defaultLoad` formats).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Format {
    Module,
    CommonJs,
    Json,
    Builtin,
}

impl Format {
    fn name(self) -> &'static str {
        match self {
            Format::Module => "module",
            Format::CommonJs => "commonjs",
            Format::Json => "json",
            Format::Builtin => "builtin",
        }
    }
}

thread_local! {
    /// `package.json` path → its parsed contents (`None` when unreadable or
    /// not JSON), read once per process like node's package-json cache.
    static PACKAGE_JSON: std::cell::RefCell<HashMap<PathBuf, Option<serde_json::Value>>> =
        std::cell::RefCell::new(HashMap::new());
}

fn read_package_json(path: &Path) -> Option<serde_json::Value> {
    if let Some(hit) = PACKAGE_JSON.with(|c| c.borrow().get(path).cloned()) {
        return hit;
    }
    let parsed = std::fs::read_to_string(path)
        .ok()
        .and_then(|t| serde_json::from_str::<serde_json::Value>(&t).ok());
    PACKAGE_JSON.with(|c| c.borrow_mut().insert(path.to_path_buf(), parsed.clone()));
    parsed
}

/// The nearest `package.json` at or above `dir` (`GetPackageScopeConfig`),
/// stopping at a `node_modules` boundary.
fn package_scope(dir: &Path) -> Option<serde_json::Value> {
    package_scope_file(dir).and_then(|pj| read_package_json(&pj))
}

/// The path of the nearest `package.json` at or above `dir`.
fn package_scope_file(dir: &Path) -> Option<PathBuf> {
    let mut cur = Some(dir);
    while let Some(d) = cur {
        let pj = d.join("package.json");
        if pj.is_file() {
            return Some(pj);
        }
        if d.file_name().is_some_and(|n| n == "node_modules") {
            return None;
        }
        cur = d.parent();
    }
    None
}

/// The format `path` DECLARES — by extension or its package scope's
/// `"type"` — or `None` when only its syntax can tell.
pub fn file_format_declared(path: &Path) -> Option<Format> {
    match path.extension().and_then(|e| e.to_str()) {
        Some("mjs") => return Some(Format::Module),
        Some("cjs") => return Some(Format::CommonJs),
        Some("json") => return Some(Format::Json),
        _ => {}
    }
    let scope_type = path
        .parent()
        .and_then(package_scope)
        .and_then(|p| p.get("type").and_then(|t| t.as_str()).map(str::to_string));
    match scope_type.as_deref() {
        Some("module") => Some(Format::Module),
        Some("commonjs") => Some(Format::CommonJs),
        _ => None,
    }
}

/// The format of the file at `path` (`ESM_FILE_FORMAT`): by extension, then
/// by the package scope's `"type"`, then by syntax detection.
pub fn file_format(path: &Path) -> Format {
    file_format_declared(path).unwrap_or_else(|| match std::fs::read_to_string(path) {
        Ok(src) if has_module_syntax(&src) => Format::Module,
        _ => Format::CommonJs,
    })
}

// ── resolution (node's ESM resolver, `defaultResolve`) ──────────────────────

fn not_found(what: &str, parent: Option<&str>) -> String {
    let msg = match parent {
        Some(p) => format!("Cannot find {what} imported from {p}"),
        None => format!("Cannot find {what}"),
    };
    crate::host::plain_coded_error("Error", "ERR_MODULE_NOT_FOUND", &msg)
}

/// `file://` URL of an absolute path, percent-encoded as `pathToFileURL` does.
pub fn file_url(path: &str) -> String {
    url::Url::from_file_path(path)
        .map(|u| u.to_string())
        .unwrap_or_else(|_| format!("file://{path}"))
}

/// A module the resolver found: its URL (the module-map key), its file (empty
/// for a builtin), and its format.
pub struct Resolved {
    pub url: String,
    pub file: String,
    pub format: Format,
}

fn builtin(spec: &str) -> Option<Resolved> {
    // The URL names the module as written (`node:events`), not the internal
    // namespace `stdlib::resolve` maps it to.
    crate::stdlib::resolve(spec).map(|_| Resolved {
        url: format!("node:{}", spec.strip_prefix("node:").unwrap_or(spec)),
        file: String::new(),
        format: Format::Builtin,
    })
}

/// Resolve `spec` imported from the module file `parent` (`None`: the
/// current directory, as for `node -e`).
pub fn resolve(spec: &str, parent: Option<&str>) -> Result<Resolved, String> {
    if let Some(name) = spec.strip_prefix("node:") {
        return builtin(spec).ok_or_else(|| {
            crate::host::plain_coded_error(
                "Error",
                "ERR_UNKNOWN_BUILTIN_MODULE",
                &format!("No such built-in module: node:{name}"),
            )
        });
    }
    let base_dir = match parent {
        Some(p) => Path::new(p)
            .parent()
            .map(Path::to_path_buf)
            .unwrap_or_default(),
        None => std::env::current_dir().unwrap_or_default(),
    };
    let base_url = url::Url::from_directory_path(&base_dir)
        .map_err(|_| crate::host::type_error(&format!("Invalid URL: {}", base_dir.display())))?;
    let relative = spec.starts_with("./") || spec.starts_with("../") || spec.starts_with('/');
    let url = if relative {
        base_url
            .join(spec)
            .map_err(|_| crate::host::type_error(&format!("Invalid URL: {spec}")))?
    } else if let Ok(u) = url::Url::parse(spec) {
        if u.scheme() != "file" {
            return Err(crate::host::plain_coded_error(
                "Error",
                "ERR_UNSUPPORTED_ESM_URL_SCHEME",
                &format!(
                    "Only URLs with a scheme in: file and node are supported by the default ESM loader. Received protocol '{}:'",
                    u.scheme()
                ),
            ));
        }
        u
    } else {
        if let Some(b) = builtin(spec) {
            return Ok(b);
        }
        let file = if spec.starts_with('#') {
            imports_resolve(spec, &base_dir, parent)?
        } else {
            package_resolve(spec, &base_dir, parent)?
        };
        url::Url::from_file_path(&file)
            .map_err(|_| crate::host::type_error(&format!("Invalid URL: {}", file.display())))?
    };
    finalize(url, parent)
}

/// `finalizeResolution`: the URL must name an existing file (no extension
/// probing, no directory index), and its identity is the REAL path.
fn finalize(url: url::Url, parent: Option<&str>) -> Result<Resolved, String> {
    let path = url
        .to_file_path()
        .map_err(|_| crate::host::type_error(&format!("Invalid URL: {url}")))?;
    if path.is_dir() {
        return Err(crate::host::plain_coded_error(
            "Error",
            "ERR_UNSUPPORTED_DIR_IMPORT",
            &format!(
                "Directory import '{}' is not supported resolving ES modules imported from {}",
                path.display(),
                parent.unwrap_or("")
            ),
        ));
    }
    if !path.is_file() {
        return Err(not_found(&format!("module '{}'", path.display()), parent));
    }
    let real = std::fs::canonicalize(&path).unwrap_or(path);
    let mut url = url::Url::from_file_path(&real).unwrap_or(url.clone());
    // `?query` / `#hash` are part of a module's identity.
    url.set_query(url.query().map(str::to_string).as_deref());
    let file = real.to_string_lossy().into_owned();
    Ok(Resolved {
        url: url.to_string(),
        format: file_format(&real),
        file,
    })
}

/// The conditions node's ESM resolver matches in a package's `"exports"`.
const IMPORT_CONDITIONS: &[&str] = &["node", "import", "module-sync", "default"];

/// `PACKAGE_RESOLVE`: a bare specifier, through `node_modules`.
fn package_resolve(spec: &str, from: &Path, parent: Option<&str>) -> Result<PathBuf, String> {
    let (name, rest) = match spec.strip_prefix('@') {
        Some(s) => match s.split_once('/') {
            Some((scope, tail)) => {
                let (pkg, sub) = tail.split_once('/').unwrap_or((tail, ""));
                (format!("@{scope}/{pkg}"), sub.to_string())
            }
            None => (spec.to_string(), String::new()),
        },
        None => {
            let (pkg, sub) = spec.split_once('/').unwrap_or((spec, ""));
            (pkg.to_string(), sub.to_string())
        }
    };
    let subpath = if rest.is_empty() {
        ".".to_string()
    } else {
        format!("./{rest}")
    };
    // PACKAGE_SELF_RESOLVE: a package may import itself by its own name,
    // through its own `"exports"`.
    if let Some(pj) = package_scope_file(from) {
        if let Some(json) = read_package_json(&pj) {
            let exports = json.get("exports").filter(|e| !e.is_null());
            if let (Some(exports), Some(own)) = (exports, json.get("name").and_then(|n| n.as_str()))
            {
                if own == name {
                    let pkg = pj.parent().unwrap_or(Path::new("/"));
                    return exports_resolve(pkg, &subpath, exports, &pj, parent);
                }
            }
        }
    }
    let mut dir = Some(from);
    while let Some(d) = dir {
        let pkg_dir = d.join("node_modules").join(&name);
        if pkg_dir.is_dir() {
            let pj_path = pkg_dir.join("package.json");
            let pj = read_package_json(&pj_path);
            if let Some(exports) = pj
                .as_ref()
                .and_then(|p| p.get("exports"))
                .filter(|e| !e.is_null())
            {
                return exports_resolve(&pkg_dir, &subpath, exports, &pj_path, parent);
            }
            if subpath == "." {
                return legacy_main(&pkg_dir, pj.as_ref())
                    .ok_or_else(|| not_found(&format!("package '{}'", pkg_dir.display()), parent));
            }
            return Ok(pkg_dir.join(&rest));
        }
        dir = d.parent();
    }
    Err(not_found(&format!("package '{name}'"), parent))
}

/// `legacyMainResolve`: `"main"` with the CommonJS extension probes, then
/// `index.js`.
fn legacy_main(pkg: &Path, pj: Option<&serde_json::Value>) -> Option<PathBuf> {
    let mut cands: Vec<PathBuf> = Vec::new();
    if let Some(main) = pj.and_then(|p| p.get("main")).and_then(|m| m.as_str()) {
        let m = pkg.join(main);
        cands.push(m.clone());
        for ext in ["js", "json", "node"] {
            cands.push(PathBuf::from(format!("{}.{ext}", m.display())));
        }
        for idx in ["index.js", "index.json", "index.node"] {
            cands.push(m.join(idx));
        }
    }
    for idx in ["index.js", "index.json", "index.node"] {
        cands.push(pkg.join(idx));
    }
    cands.into_iter().find(|c| c.is_file())
}

fn not_exported(pkg_json: &Path, subpath: &str, parent: Option<&str>) -> String {
    let from = parent
        .map(|p| format!(" imported from {p}"))
        .unwrap_or_default();
    let msg = if subpath == "." {
        format!(
            "No \"exports\" main defined in {}{from}",
            pkg_json.display()
        )
    } else {
        format!(
            "Package subpath '{subpath}' is not defined by \"exports\" in {}{from}",
            pkg_json.display()
        )
    };
    crate::host::plain_coded_error("Error", "ERR_PACKAGE_PATH_NOT_EXPORTED", &msg)
}

/// `PACKAGE_EXPORTS_RESOLVE` for `subpath` (`"."` or `"./x"`).
fn exports_resolve(
    pkg: &Path,
    subpath: &str,
    exports: &serde_json::Value,
    pj: &Path,
    parent: Option<&str>,
) -> Result<PathBuf, String> {
    // A string, an array, or an object without `.`-keys is the `"."` entry.
    let is_subpath_map = exports
        .as_object()
        .is_some_and(|o| o.keys().next().is_some_and(|k| k.starts_with('.')));
    let found = match exports.as_object() {
        Some(map) if is_subpath_map => match_subpath_map(map, subpath),
        _ => (subpath == ".").then(|| (exports, String::new())),
    };
    let found = found
        .and_then(|(t, pattern)| target_string(t, &pattern))
        .and_then(|s| s.strip_prefix("./").map(|rel| pkg.join(rel)));
    found.ok_or_else(|| not_exported(pj, subpath, parent))
}

/// The entry of an `"exports"` / `"imports"` map that `key` selects: an exact
/// key, else the longest `prefix*suffix` pattern key that matches
/// (`PATTERN_KEY_COMPARE`), with what `*` matched.
fn match_subpath_map<'a>(
    map: &'a serde_json::Map<String, serde_json::Value>,
    key: &str,
) -> Option<(&'a serde_json::Value, String)> {
    if let Some(t) = map.get(key).filter(|_| !key.contains('*')) {
        return Some((t, String::new()));
    }
    let mut best: Option<(&str, String)> = None;
    for k in map.keys() {
        let Some((pre, post)) = k.split_once('*') else {
            continue;
        };
        if key.starts_with(pre)
            && key.len() >= pre.len() + post.len()
            && key.ends_with(post)
            && best.as_ref().map_or(true, |(b, _)| k.len() > b.len())
        {
            best = Some((
                k.as_str(),
                key[pre.len()..key.len() - post.len()].to_string(),
            ));
        }
    }
    best.map(|(k, mid)| (&map[k], mid))
}

/// `PACKAGE_TARGET_RESOLVE` down to the target string (with `*` replaced by
/// `pattern`): a string, a conditions object tried in key order, or a
/// fallback array.
fn target_string(target: &serde_json::Value, pattern: &str) -> Option<String> {
    match target {
        serde_json::Value::String(s) => Some(s.replace('*', pattern)),
        serde_json::Value::Object(o) => o
            .iter()
            .filter(|(k, _)| IMPORT_CONDITIONS.contains(&k.as_str()))
            .find_map(|(_, v)| target_string(v, pattern)),
        serde_json::Value::Array(a) => a.iter().find_map(|v| target_string(v, pattern)),
        _ => None,
    }
}

/// `PACKAGE_IMPORTS_RESOLVE`: a `#name` specifier through the `"imports"` of
/// the importing file's package. A target is a `./` path inside the package or
/// a bare specifier resolved as a package.
fn imports_resolve(spec: &str, from: &Path, parent: Option<&str>) -> Result<PathBuf, String> {
    let undefined = || {
        let from_text = parent
            .map(|p| format!(" imported from {p}"))
            .unwrap_or_default();
        crate::host::plain_coded_error(
            "TypeError",
            "ERR_PACKAGE_IMPORT_NOT_DEFINED",
            &format!("Package import specifier \"{spec}\" is not defined{from_text}"),
        )
    };
    let pj = package_scope_file(from).ok_or_else(undefined)?;
    let pkg = pj.parent().unwrap_or(Path::new("/"));
    let json = read_package_json(&pj).ok_or_else(undefined)?;
    let map = json
        .get("imports")
        .and_then(|i| i.as_object())
        .ok_or_else(undefined)?;
    let (target, pattern) = match_subpath_map(map, spec).ok_or_else(undefined)?;
    let target = target_string(target, &pattern).ok_or_else(undefined)?;
    match target.strip_prefix("./") {
        Some(rel) => Ok(pkg.join(rel)),
        None => package_resolve(&target, pkg, parent),
    }
}

// ── CommonJS named exports (cjs-module-lexer) ───────────────────────────────

/// The names an `import { … }` of a CommonJS module may use: the exports
/// node's `cjs-module-lexer` detects statically, without running the module —
/// `exports.x = …`, `module.exports.x = …`, `Object.defineProperty(exports,
/// "x", …)`, the keys of a `module.exports = { a, b: … }` literal — plus the
/// names of a module it re-exports through `module.exports = require("…")`.
pub fn cjs_export_names(file: &Path, seen: &mut HashSet<PathBuf>) -> Vec<String> {
    let mut out: Vec<String> = Vec::new();
    if !seen.insert(file.to_path_buf()) {
        return out;
    }
    let Ok(src) = std::fs::read_to_string(file) else {
        return out;
    };
    let Ok(stmts) = crate::parser::parse(&src) else {
        return out;
    };
    let mut reexports: Vec<String> = Vec::new();
    let mut scan = CjsScan {
        names: &mut out,
        reexports: &mut reexports,
    };
    for s in &stmts {
        scan.stmt(s);
    }
    let dir = file.parent().unwrap_or(Path::new("/"));
    for spec in reexports {
        if let Some(target) = crate::module::resolve(&spec, dir) {
            for n in cjs_export_names(&target, seen) {
                if !out.contains(&n) {
                    out.push(n);
                }
            }
        }
    }
    out.retain(|n| n != "default");
    out
}

struct CjsScan<'a> {
    names: &'a mut Vec<String>,
    reexports: &'a mut Vec<String>,
}

/// `exports` or `module.exports`.
fn is_exports_obj(e: &Expr) -> bool {
    match e {
        Expr::Ident(n) => n == "exports",
        Expr::Member {
            object, property, ..
        } => property == "exports" && matches!(&**object, Expr::Ident(m) if m == "module"),
        _ => false,
    }
}

impl CjsScan<'_> {
    fn add(&mut self, n: &str) {
        if !self.names.iter().any(|x| x == n) {
            self.names.push(n.to_string());
        }
    }
    fn stmt(&mut self, s: &Stmt) {
        walk_stmt_children(s, &mut |n| match n {
            Node::S(st) => self.stmt(st),
            Node::E(e) => self.expr(e),
        });
        if let StmtKind::FuncDecl { body, .. } = &s.kind {
            body.iter().for_each(|st| self.stmt(st));
        }
    }
    fn expr(&mut self, e: &Expr) {
        match e {
            Expr::Assign { target, value, .. } => {
                match &**target {
                    Expr::Member {
                        object, property, ..
                    } if is_exports_obj(object) => self.add(property),
                    Expr::Index { object, index, .. } if is_exports_obj(object) => {
                        if let Expr::Str(k) = &**index {
                            self.add(k);
                        }
                    }
                    t if is_exports_obj(t) && matches!(t, Expr::Member { .. }) => match &**value {
                        // The lexer reads `{ a, b: c, 'd': e, ...f }`: a key
                        // whose value is an identifier (or a shorthand), a
                        // spread is skipped, and anything else ends the scan —
                        // after the key of a `key: require(…)`.
                        Expr::Object(props) => {
                            for p in props {
                                match p {
                                    Prop::KeyValue {
                                        key: Expr::Str(k),
                                        computed: false,
                                        value,
                                    } => match value {
                                        Expr::Ident(_) => self.add(k),
                                        Expr::Call { func, .. } if matches!(&**func, Expr::Ident(r) if r == "require") =>
                                        {
                                            self.add(k);
                                            break;
                                        }
                                        _ => break,
                                    },
                                    Prop::Spread(_) => {}
                                    _ => break,
                                }
                            }
                        }
                        Expr::Call { func, args, .. } if matches!(&**func, Expr::Ident(r) if r == "require") => {
                            if let Some(Expr::Str(s)) = args.first() {
                                self.reexports.push(s.clone());
                            }
                        }
                        _ => {}
                    },
                    _ => {}
                }
                walk_expr_children(e, &mut |x| self.expr(x));
            }
            // `Object.defineProperty(exports, "x", …)`.
            Expr::Call { func, args, .. } => {
                if let Expr::Member {
                    object, property, ..
                } = &**func
                {
                    if property == "defineProperty"
                        && matches!(&**object, Expr::Ident(o) if o == "Object")
                        && args.first().is_some_and(is_exports_obj)
                    {
                        if let Some(Expr::Str(k)) = args.get(1) {
                            self.add(k);
                        }
                    }
                }
                walk_expr_children(e, &mut |x| self.expr(x));
            }
            Expr::Function { body, .. } => match body {
                FnBody::Block(b) => b.iter().for_each(|st| self.stmt(st)),
                FnBody::Expr(x) => self.expr(x),
            },
            _ => walk_expr_children(e, &mut |x| self.expr(x)),
        }
    }
}

// ── module registry and the natives `esm_loader.js` calls ───────────────────

use crate::host::{self, with_host, JsObj};
use fusevm::Value;

/// A parsed module waiting to be instantiated: its lowered function (built as
/// a plain function; [`compile`] turns it into the generator form a cycle
/// needs) and the text its spans index.
struct Parsed {
    lowered: Option<Expr>,
    source: String,
}

thread_local! {
    static PARSED: std::cell::RefCell<Vec<Parsed>> = const { std::cell::RefCell::new(Vec::new()) };
    /// Module text with no file behind it, keyed by the name it runs under.
    static VIRTUAL: std::cell::RefCell<HashMap<String, String>> =
        std::cell::RefCell::new(HashMap::new());
    /// The loader API object `esm_loader.js` returns, built once per host.
    static LOADER: std::cell::RefCell<Option<Value>> = const { std::cell::RefCell::new(None) };
}

/// Drop every module record. Called from `module::reset` with the heap.
pub fn reset() {
    PARSED.with(|p| p.borrow_mut().clear());
    VIRTUAL.with(|v| v.borrow_mut().clear());
    LOADER.with(|l| *l.borrow_mut() = None);
    PACKAGE_JSON.with(|c| c.borrow_mut().clear());
}

fn s(v: &Value) -> String {
    with_host(|h| h.str_of(v))
}

fn js_str(text: &str) -> Value {
    with_host(|h| h.new_str(text.to_string()))
}

fn js_array(items: Vec<Value>) -> Value {
    with_host(|h| h.new_array(items))
}

fn js_name(n: &Option<String>) -> Value {
    match n {
        Some(n) => js_str(n),
        None => with_host(|h| h.null()),
    }
}

/// The record fields of a parsed module, as the loader reads them.
/// Node's rendering of an import clause as the destructuring it suggests in
/// its "Named export not found" error: whitespace collapsed, `as` turned into
/// `:` (`{ a as b }` → `{ a: b }`).
fn clause_text(source: &str, (start, end): Span) -> String {
    let raw = source.get(start as usize..end as usize).unwrap_or("");
    let words: Vec<&str> = raw.split_whitespace().collect();
    words.join(" ").replace(" as ", ": ")
}

fn info_value(id: usize, info: &ModuleInfo, source: &str) -> Value {
    let requests = info
        .requests
        .iter()
        .map(|(spec, attrs)| {
            let mut props = indexmap::IndexMap::new();
            for (k, v) in attrs {
                props.insert(k.clone(), js_str(v));
            }
            let attrs = with_host(|h| h.new_object(props));
            js_array(vec![js_str(spec), attrs])
        })
        .collect();
    let imports = info
        .imports
        .iter()
        .map(|e| {
            js_array(vec![
                Value::Float(e.request as f64),
                js_name(&e.import_name),
                js_str(&e.local),
                js_str(&clause_text(source, e.clause)),
            ])
        })
        .collect();
    let locals = info
        .local_exports
        .iter()
        .map(|(e, l)| js_array(vec![js_str(e), js_str(l)]))
        .collect();
    let indirect = info
        .indirect_exports
        .iter()
        .map(|e| {
            js_array(vec![
                js_str(&e.export_name),
                Value::Float(e.request as f64),
                js_name(&e.import_name),
            ])
        })
        .collect();
    let stars = info
        .star_exports
        .iter()
        .map(|r| Value::Float(*r as f64))
        .collect();
    let mut props = indexmap::IndexMap::new();
    props.insert("id".to_string(), Value::Float(id as f64));
    props.insert("requests".to_string(), js_array(requests));
    props.insert("imports".to_string(), js_array(imports));
    props.insert("localExports".to_string(), js_array(locals));
    props.insert("indirectExports".to_string(), js_array(indirect));
    props.insert("starExports".to_string(), js_array(stars));
    props.insert("hasTLA".to_string(), Value::Bool(info.has_tla));
    with_host(|h| h.new_object(props))
}

/// ParseModule for the file at `path`: the record fields, with the parsed
/// body kept for [`compile`].
fn parse_file(path: &str) -> Result<Value, String> {
    let source = match VIRTUAL.with(|v| v.borrow_mut().remove(path)) {
        Some(text) => text,
        None => {
            std::fs::read_to_string(path).map_err(|e| format!("Error: cannot read {path}: {e}"))?
        }
    };
    let stmts = crate::parser::parse_module(&source)?;
    let info = analyze(&stmts)?;
    let lowered = lower(stmts, &info, path, false);
    // The early errors of the module's own statement list (duplicate lexical
    // declarations, a `var` colliding with a `let`, …) are checked here, on
    // the lowered body, rather than when the loader first instantiates it.
    if let Expr::Function {
        body: FnBody::Block(b),
        ..
    } = &lowered
    {
        crate::compiler::check_early_errors(b)?;
    }
    let id = PARSED.with(|p| p.borrow().len());
    let record = info_value(id, &info, &source);
    PARSED.with(|p| {
        p.borrow_mut().push(Parsed {
            lowered: Some(lowered),
            source,
        })
    });
    Ok(record)
}

/// Compile parsed module `id` to its function; `generator` selects the form a
/// module in an import cycle is built in (see the module docs).
fn compile(id: usize, generator: bool) -> Result<Value, String> {
    let (lowered, source) = PARSED.with(|p| {
        let mut p = p.borrow_mut();
        let entry = &mut p[id];
        (entry.lowered.take(), std::mem::take(&mut entry.source))
    });
    let mut lowered = lowered.ok_or_else(|| host::type_error("module instantiated twice"))?;
    if generator {
        if let Expr::Function {
            body: FnBody::Block(b),
            is_generator,
            ..
        } = &mut lowered
        {
            *is_generator = true;
            b.insert(
                1,
                Stmt::from(StmtKind::Expr(Expr::Yield {
                    arg: None,
                    delegate: false,
                })),
            );
        }
    }
    let stmts = vec![Stmt::from(StmtKind::Expr(lowered))];
    let prog = crate::with_source(
        crate::compiler::compile_completion_strict(&stmts, false, true, false),
        &source,
    )?;
    host::run_chunk_in_global_scope(crate::load_merged(prog))
}

/// Mark `ns` (a Proxy) as a module namespace object, which `util.inspect`
/// and the refused-write messages recognise.
fn mark_namespace(ns: &Value) {
    with_host(|h| {
        if let Some(JsObj::Proxy { target, .. }) = h.get(ns).cloned() {
            if let Some(JsObj::Object(p)) = h.get_mut(&target) {
                p.insert(NAMESPACE_MARK.to_string(), Value::Bool(true));
            }
        }
    });
}

/// Hidden slot on a namespace's proxy target.
const NAMESPACE_MARK: &str = "@@esmNamespace";

/// Whether `v` is a module namespace object.
pub fn is_namespace(v: &Value) -> bool {
    with_host(|h| match h.get(v) {
        Some(JsObj::Proxy { target, .. }) => {
            matches!(h.get(target), Some(JsObj::Object(p)) if p.contains_key(NAMESPACE_MARK))
        }
        _ => false,
    })
}

/// The `@@esm.<name>` natives.
pub fn native(name: &str, args: &[Value]) -> Result<Value, String> {
    let arg = |i: usize| args.get(i).cloned().unwrap_or(Value::Undef);
    match name {
        "resolve" => {
            let parent = match arg(1) {
                Value::Undef => None,
                v => Some(s(&v)),
            };
            let r = resolve(&s(&arg(0)), parent.as_deref())?;
            Ok(js_array(vec![
                js_str(&r.url),
                js_str(&r.file),
                js_str(r.format.name()),
            ]))
        }
        "parse" => parse_file(&s(&arg(0))),
        "compile" => {
            let id = with_host(|h| h.to_number(&arg(0))) as usize;
            let generator = with_host(|h| h.truthy(&arg(1)));
            compile(id, generator)
        }
        "cjsNames" => {
            let names = cjs_export_names(Path::new(&s(&arg(0))), &mut HashSet::new());
            Ok(js_array(names.iter().map(|n| js_str(n)).collect()))
        }
        "requireCjs" => crate::module::require_file(Path::new(&s(&arg(0))), ""),
        "builtin" => {
            let url = s(&arg(0));
            let spec = url.strip_prefix("node:").unwrap_or(&url);
            crate::module::require(spec, &crate::module::entry_dir(), "")
        }
        "json" => {
            let path = s(&arg(0));
            let text = std::fs::read_to_string(&path)
                .map_err(|e| format!("Error: cannot read {path}: {e}"))?;
            crate::builtins::call_builtin_function("JSON.parse", vec![js_str(&text)])
        }
        "markNamespace" => {
            mark_namespace(&arg(0));
            Ok(Value::Undef)
        }
        // The two entry points compiled code reaches through globals.
        "import" => loader_call("dynamicImport", args.to_vec()),
        "main" => loader_call("runMain", args.to_vec()),
        _ => Err(host::type_error(&format!("@@esm.{name} is not a function"))),
    }
}

/// The loader API (`esm_loader.js`), evaluated once per host.
fn loader() -> Result<Value, String> {
    if let Some(l) = LOADER.with(|l| l.borrow().clone()) {
        return Ok(l);
    }
    let factory = crate::eval_in_global_scope(include_str!("esm_loader.js"))?;
    let mut natives = indexmap::IndexMap::new();
    for n in [
        "resolve",
        "parse",
        "compile",
        "cjsNames",
        "requireCjs",
        "builtin",
        "json",
        "markNamespace",
    ] {
        let f = with_host(|h| h.alloc(JsObj::Builtin(format!("@@esm.{n}"))));
        natives.insert(n.to_string(), f);
    }
    let natives = with_host(|h| h.new_object(natives));
    let api = host::invoke(&factory, vec![natives], None)?;
    LOADER.with(|l| *l.borrow_mut() = Some(api.clone()));
    Ok(api)
}

fn loader_call(method: &str, args: Vec<Value>) -> Result<Value, String> {
    let api = loader()?;
    let f = crate::builtins::get_property(&api, method)?;
    host::invoke(&f, args, None)
}

/// `require()` of an ES module file (node's `require(esm)`): its namespace,
/// or the value it exports under the name `"module.exports"`.
pub fn require_module(path: &Path, parent: &str) -> Result<Value, String> {
    loader_call(
        "requireModule",
        vec![js_str(&path.to_string_lossy()), js_str(parent)],
    )
}

/// The statements that run an ES module ENTRY point: one call that loads and
/// links its graph and queues its evaluation (`esm_loader.js` `runMain`).
///
/// `entry` is the module's file; `source`, when given, is its text — the
/// `[eval1]` module `node -e` and piped stdin run has no file behind it.
/// `is_main` is `import.meta.main`, true only for a file entry point.
pub fn main_program(entry: &str, source: Option<String>, is_main: bool) -> Vec<Stmt> {
    if let Some(text) = source {
        VIRTUAL.with(|v| v.borrow_mut().insert(entry.to_string(), text));
    }
    ENTRY_URL.with(|u| *u.borrow_mut() = file_url(entry));
    let call = Expr::Call {
        func: Box::new(Expr::Ident(MAIN_FN.into())),
        args: vec![
            Expr::Str(entry.to_string()),
            Expr::Str(file_url(entry)),
            if is_main { Expr::True } else { Expr::False },
        ],
        optional: false,
    };
    vec![Stmt::from(StmtKind::Expr(call))]
}

/// The global [`main_program`] calls.
pub const MAIN_FN: &str = "%esm.main";

/// The refused-write operations of a module namespace object, which V8
/// reports in its own words rather than as a proxy trap's.
pub enum Refused {
    Set,
    Delete,
    Define,
}

/// V8's `TypeError` for a refused `op` on property `key` of `recv`, when
/// `recv` is a module namespace object.
pub fn namespace_refusal(recv: &Value, op: Refused, key: &str) -> Option<String> {
    if !is_namespace(recv) {
        return None;
    }
    Some(host::type_error(&match op {
        Refused::Set => format!("Cannot assign to property '{key}' of [object Module]"),
        Refused::Delete => format!("Cannot delete property '{key}' of [object Module]"),
        Refused::Define => format!("Cannot redefine property: {key}"),
    }))
}

/// When the event loop has drained with the ENTRY module still suspended in a
/// top-level `await`, node warns and exits 13 (unless the program chose an
/// exit code). Called once the loop is empty, before the `exit` event.
pub fn settle_unfinished_entry() -> Result<(), String> {
    if LOADER.with(|l| l.borrow().is_none()) {
        return Ok(());
    }
    let pending = loader_call("mainUnsettled", Vec::new())?;
    if !with_host(|h| h.truthy(&pending)) {
        return Ok(());
    }
    let url = ENTRY_URL.with(|u| u.borrow().clone());
    with_host(|h| {
        h.write_out(
            &format!("Warning: Detected unsettled top-level await at {url}\n"),
            true,
        );
        h.exit_code.get_or_insert(13);
    });
    Ok(())
}

thread_local! {
    /// The entry module's URL, named by the unsettled-`await` warning.
    static ENTRY_URL: std::cell::RefCell<String> = const { std::cell::RefCell::new(String::new()) };
}
