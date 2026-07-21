//! JavaScript abstract syntax tree.
//!
//! Every node here has a direct lowering in `compiler.rs`. JS is
//! statement-oriented with brace-delimited blocks, so the tree separates `Stmt`
//! (blocks of these form a program/function body) from `Expr`. Numbers are all
//! IEEE-754 `f64`, matching JavaScript's single number type.

/// A binary operator (`a <op> b`). `&&`/`||`/`??` are `LogicalOp` because they
/// short-circuit and yield an operand value, not a coerced boolean.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BinOp {
    Add,
    Sub,
    Mul,
    Div,
    Mod,
    Pow, // **
    // Comparison
    Lt,
    Le,
    Gt,
    Ge,
    EqEqEq, // ===
    NeEqEq, // !==
    EqEq,   // ==  (loose, coercing)
    NeEq,   // !=
    // Bitwise / shift
    BitAnd,
    BitOr,
    BitXor,
    Shl,  // <<
    Shr,  // >>
    UShr, // >>>
    // `in` / `instanceof`
    In,
    InstanceOf,
}

/// A short-circuiting logical operator.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LogicalOp {
    And,     // &&
    Or,      // ||
    Nullish, // ??
}

/// A unary prefix operator.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum UnOp {
    Neg,    // -x
    Pos,    // +x
    Not,    // !x
    BitNot, // ~x
    TypeOf, // typeof x
    Void,   // void x
    Delete, // delete x
}

/// The kind of a variable declaration.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DeclKind {
    Var,
    Let,
    Const,
}

/// The update (increment/decrement) operator, prefix or postfix.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum UpdateOp {
    Inc, // ++
    Dec, // --
}

/// A property of an object literal.
#[derive(Debug, Clone, PartialEq)]
pub enum Prop {
    /// `key: value` — `computed` marks `[expr]: value`.
    KeyValue {
        key: Expr,
        value: Expr,
        computed: bool,
    },
    /// `...spread`.
    Spread(Expr),
    /// `get key() {}` / `set key(v) {}` — an accessor property.
    Accessor {
        key: Expr,
        computed: bool,
        /// `true` for a getter, `false` for a setter.
        is_getter: bool,
        /// The accessor function (an `Expr::Function`).
        func: Expr,
    },
}

/// A JavaScript expression.
#[derive(Debug, Clone, PartialEq)]
pub enum Expr {
    Null,
    Undefined,
    True,
    False,
    Number(f64),
    /// A `BigInt` literal as its canonical decimal digit string (`"123"` for
    /// `123n`). Lowered to a heap `JsObj::BigInt`.
    BigInt(String),
    /// A regex literal: `(pattern, flags)`. Lowered to a `JsObj::RegExp`.
    Regex(String, String),
    Str(String),
    /// A template literal: alternating literal quasis and interpolated exprs.
    /// `quasis.len() == exprs.len() + 1`.
    Template {
        quasis: Vec<String>,
        exprs: Vec<Expr>,
    },
    /// A tagged template `` tag`a${x}b` ``: calls `tag(strings, ...values)` where
    /// `strings` is the cooked-quasi array carrying a `.raw` array of `raws`.
    TaggedTemplate {
        tag: Box<Expr>,
        quasis: Vec<String>,
        raws: Vec<String>,
        exprs: Vec<Expr>,
    },

    /// A bare identifier (`x`); the compiler resolves scope at runtime.
    Ident(String),
    /// `this`.
    This,
    /// `super` (only valid as `super(...)` call callee or `super.x` object).
    Super,
    /// `new.target`.
    NewTarget,
    /// `yield expr` / `yield* expr` / `yield` (generator).
    Yield {
        arg: Option<Box<Expr>>,
        delegate: bool,
    },
    /// `await expr` (async function).
    Await(Box<Expr>),
    /// A `class` expression.
    Class(Box<ClassNode>),

    Array(Vec<Expr>),
    Object(Vec<Prop>),
    /// `...expr` — a spread element (array/call).
    Spread(Box<Expr>),

    Logical(LogicalOp, Box<Expr>, Box<Expr>),
    Unary(UnOp, Box<Expr>),
    Binary(BinOp, Box<Expr>, Box<Expr>),

    /// `test ? cons : alt`.
    Conditional {
        test: Box<Expr>,
        cons: Box<Expr>,
        alt: Box<Expr>,
    },

    /// `target = value` (or a compound `target op= value` desugared by the parser).
    Assign {
        target: Box<Expr>,
        value: Box<Expr>,
    },
    /// `++x` / `x++` / `--x` / `x--`.
    Update {
        op: UpdateOp,
        prefix: bool,
        target: Box<Expr>,
    },

    /// A call `func(args)`. `optional` marks `?.(`.
    Call {
        func: Box<Expr>,
        args: Vec<Expr>,
        optional: bool,
    },
    /// `new Ctor(args)`.
    New {
        callee: Box<Expr>,
        args: Vec<Expr>,
    },
    /// `value.name` — `optional` marks `?.name`.
    Member {
        object: Box<Expr>,
        property: String,
        optional: bool,
    },
    /// `value[expr]` — `optional` marks `?.[expr]`.
    Index {
        object: Box<Expr>,
        index: Box<Expr>,
        optional: bool,
    },

    /// A function expression / arrow function.
    Function {
        params: Vec<Param>,
        body: FnBody,
        is_arrow: bool,
        name: Option<String>,
        is_generator: bool,
        is_async: bool,
    },

    /// `,`-sequence expression: evaluate all, yield the last.
    Sequence(Vec<Expr>),
}

/// A `class` declaration/expression body.
#[derive(Debug, Clone, PartialEq)]
pub struct ClassNode {
    pub name: Option<String>,
    /// The `extends` expression, if any.
    pub parent: Option<Box<Expr>>,
    pub members: Vec<ClassMember>,
}

/// One member of a class body: a method, accessor, or field, on the instance or
/// static side.
#[derive(Debug, Clone, PartialEq)]
pub struct ClassMember {
    /// The property key (an `Expr::Str` for a plain name, or any expr when
    /// `computed`).
    pub key: Expr,
    pub computed: bool,
    pub kind: MemberKind,
    pub is_static: bool,
    pub is_generator: bool,
    pub is_async: bool,
    /// Params + body for a method/accessor/constructor.
    pub params: Vec<Param>,
    pub body: Vec<Stmt>,
    /// Initializer expression for a field (`x = expr;`).
    pub field_init: Option<Expr>,
}

/// The kind of a class member.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MemberKind {
    Constructor,
    Method,
    Get,
    Set,
    Field,
}

/// A function/arrow body: either a brace-delimited statement list or (arrow) a
/// single expression whose value is returned.
#[derive(Debug, Clone, PartialEq)]
pub enum FnBody {
    Block(Vec<Stmt>),
    Expr(Box<Expr>),
}

/// A formal parameter.
#[derive(Debug, Clone, PartialEq)]
pub struct Param {
    /// The binding target — an `Ident`, or an array/object pattern (also an
    /// `Expr::Array`/`Expr::Object` used as a destructuring target).
    pub pattern: Expr,
    /// `= default`.
    pub default: Option<Expr>,
    /// `...rest`.
    pub rest: bool,
}

/// One `case`/`default` clause of a `switch`.
#[derive(Debug, Clone, PartialEq)]
pub struct SwitchCase {
    /// `None` for the `default:` clause.
    pub test: Option<Expr>,
    pub body: Vec<Stmt>,
}

/// A single declarator inside a `var`/`let`/`const`.
#[derive(Debug, Clone, PartialEq)]
pub struct Declarator {
    pub target: Expr,
    pub init: Option<Expr>,
}

/// A JavaScript statement.
#[derive(Debug, Clone, PartialEq)]
pub enum StmtKind {
    /// An expression evaluated for effect (value discarded).
    Expr(Expr),
    /// `var`/`let`/`const` declaration list.
    Decl {
        kind: DeclKind,
        decls: Vec<Declarator>,
    },
    /// `{ ... }` block.
    Block(Vec<Stmt>),
    /// `function name(params) { body }`.
    FuncDecl {
        name: String,
        params: Vec<Param>,
        body: Vec<Stmt>,
        is_generator: bool,
        is_async: bool,
    },
    /// `class Name … { … }`.
    ClassDecl(ClassNode),

    If {
        test: Expr,
        cons: Box<Stmt>,
        alt: Option<Box<Stmt>>,
    },
    While {
        test: Expr,
        body: Box<Stmt>,
    },
    DoWhile {
        body: Box<Stmt>,
        test: Expr,
    },
    /// C-style `for (init; test; update) body`.
    For {
        init: Option<Box<Stmt>>,
        test: Option<Expr>,
        update: Option<Expr>,
        body: Box<Stmt>,
    },
    /// `for (decl of iterable) body`. `is_await` marks `for await (…)`.
    ForOf {
        decl_kind: Option<DeclKind>,
        target: Expr,
        iter: Expr,
        body: Box<Stmt>,
        is_await: bool,
    },
    /// `for (decl in object) body`.
    ForIn {
        decl_kind: Option<DeclKind>,
        target: Expr,
        object: Expr,
        body: Box<Stmt>,
    },
    Switch {
        disc: Expr,
        cases: Vec<SwitchCase>,
    },

    /// `label: stmt` — a labeled statement (typically a loop), targetable by
    /// `break label` / `continue label`.
    Labeled {
        label: String,
        body: Box<Stmt>,
    },

    Return(Option<Expr>),
    Break(Option<String>),
    Continue(Option<String>),
    Throw(Expr),
    Try {
        block: Vec<Stmt>,
        handler: Option<(Option<Expr>, Vec<Stmt>)>, // (param pattern, body)
        finalizer: Option<Vec<Stmt>>,
    },

    Empty,
}

/// A statement plus its 1-based source line.
#[derive(Debug, Clone, PartialEq)]
pub struct Stmt {
    pub kind: StmtKind,
    pub line: u32,
}

impl Stmt {
    pub fn new(kind: StmtKind, line: u32) -> Stmt {
        Stmt { kind, line }
    }
}

impl From<StmtKind> for Stmt {
    /// Wrap a `StmtKind` as a synthetic statement (line 0).
    fn from(kind: StmtKind) -> Stmt {
        Stmt { kind, line: 0 }
    }
}
