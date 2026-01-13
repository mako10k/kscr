#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Module {
    pub name: Option<String>,
    pub items: Vec<Item>,
}

pub type Span = crate::lexer::Span;

pub fn dummy_span() -> Span {
    Span { start: 0, end: 0 }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Item {
    Import(ImportDecl),
    Export(ExportDecl),
    Fixity(FixityDecl),
    Binding(Binding),
    TypeAlias(TypeAlias),
    DataDecl(DataDecl),
    ClassDecl(ClassDecl),
    InstanceDecl(InstanceDecl),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ClassDecl {
    pub name: String,
    pub param: String,
    /// Superclass constraints (Haskell-style): `class (C a, D a) => E a where ...`.
    pub supers: Vec<Predicate>,
    pub methods: Vec<ClassMethodSig>,
    /// Default method implementations inside the class.
    ///
    /// These are optional; instances may omit methods that have defaults.
    pub default_methods: Vec<Binding>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ClassMethodSig {
    pub name: String,
    pub ty: QualType,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct InstanceDecl {
    pub class: String,
    pub ty: Type,
    /// Method bindings inside the instance.
    pub methods: Vec<Binding>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ImportDecl {
    pub module: String,
    pub qualified: bool,
    pub as_name: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ExportSpec {
    Name(String),
    Type { name: String, ctors: ExportCtors },
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ExportCtors {
    All,
    Some(Vec<String>),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ExportDecl {
    pub specs: Vec<ExportSpec>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum FixityAssoc {
    Infix,
    Infixl,
    Infixr,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FixityDecl {
    pub assoc: FixityAssoc,
    pub prec: u8,
    pub ops: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Binding {
    pub pat: Pattern,
    pub expr: Expr,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TypeAlias {
    pub name: String,
    pub params: Vec<String>,
    pub ty: Type,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DataDecl {
    pub name: String,
    pub params: Vec<String>,
    pub ctors: Vec<DataCtor>,
    pub deriving: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DataCtor {
    pub name: String,
    pub args: Vec<Type>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Expr {
    pub kind: ExprKind,
    pub span: Span,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ExprKind {
    Unit,
    Integer(String),
    Float64(String),
    Bool(bool),
    String(String),
    Char(char),
    Var(String),
    Ctor(String),
    Lambda {
        params: Vec<String>,
        body: Box<Expr>,
    },
    Apply {
        func: Box<Expr>,
        args: Vec<Expr>,
    },
    If {
        cond: Box<Expr>,
        then_branch: Box<Expr>,
        else_branch: Box<Expr>,
    },
    Let {
        bindings: Vec<Binding>,
        body: Box<Expr>,
    },
    Where {
        expr: Box<Expr>,
        bindings: Vec<Binding>,
    },
    Annot {
        expr: Box<Expr>,
        ty: QualType,
    },
    Do(Vec<DoStmt>),
    Case {
        expr: Box<Expr>,
        arms: Vec<CaseArm>,
    },
    Cons {
        head: Box<Expr>,
        tail: Box<Expr>,
    },
    List(Vec<Expr>),
    Tuple(Vec<Expr>),
    Record(Vec<(String, Expr)>),
}

impl Expr {
    pub fn new(span: Span, kind: ExprKind) -> Self {
        Self { kind, span }
    }

    pub fn dummy(kind: ExprKind) -> Self {
        Self::new(dummy_span(), kind)
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CaseArm {
    pub pat: Pattern,
    pub guard: Option<Expr>,
    pub body: Expr,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DoStmt {
    Bind { pat: Pattern, expr: Expr },
    Expr(Expr),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Pattern {
    pub kind: PatternKind,
    pub span: Span,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PatternKind {
    Var(String),
    Wildcard,
    Hole(Option<String>),
    Literal(Expr),
    Tuple(Vec<Pattern>),
    List(Vec<Pattern>),
    Record(Vec<(String, Pattern)>),
    /// Loose record pattern: `{a: p, ...}` or `{a: p, ...rest}`.
    ///
    /// When `rest` is present, it is bound to the residual record.
    RecordLoose(Vec<(String, Pattern)>, Option<String>),
    Cons(Box<Pattern>, Box<Pattern>),
    Or(Box<Pattern>, Box<Pattern>),
    As(String, Box<Pattern>),
    View(Box<Pattern>, Box<Expr>),
    Constructor {
        name: String,
        args: Vec<Pattern>,
    },
}

impl Pattern {
    pub fn new(span: Span, kind: PatternKind) -> Self {
        Self { kind, span }
    }

    pub fn dummy(kind: PatternKind) -> Self {
        Self::new(dummy_span(), kind)
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Type {
    Unit,
    Integer,
    Bool,
    Float64,
    Char,
    String,

    List(Box<Type>),
    Tuple(Vec<Type>),
    Record(Vec<(String, Type)>),
    /// Open record type: `{a: T, ...r}`.
    ///
    /// `r` is the residual row.
    RecordOpen(Vec<(String, Type)>, Box<Type>),

    Hole(Option<String>),
    /// Type identifier (lowercase names are treated as type variables; uppercase as constructors).
    Var(String),
    App {
        head: Box<Type>,
        args: Vec<Type>,
    },
    Func(Box<Type>, Box<Type>),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Predicate {
    Show(Type),
    ShowRow(Type),
    Eq(Type),
    EqRow(Type),
    /// User-defined typeclass constraint: `C t`.
    Class {
        class: String,
        ty: Type,
    },
    Lacks {
        label: String,
        row: Type,
    },
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct QualType {
    pub preds: Vec<Predicate>,
    pub ty: Type,
}
