#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Module {
    pub name: Option<String>,
    pub items: Vec<Item>,
}

/// Internal module identity.
///
/// This is intentionally decoupled from the syntactic module name string.
#[derive(Debug, Copy, Clone, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct ModuleId(pub u32);

/// Internal typeclass identity.
///
/// Stage 2: class references in predicates/instances use `ClassId`.
#[derive(Debug, Clone, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct ClassId {
    pub module: ModuleId,
    pub name: String,
}

impl ClassId {
    /// Create a placeholder class id.
    ///
    /// During parsing/desugaring, `module` may still be unresolved; later passes can
    /// resolve it using import qualifiers.
    pub fn dummy(name: impl Into<String>) -> Self {
        Self {
            module: ModuleId(0),
            name: name.into(),
        }
    }
}

/// A name that can be either syntactic (unresolved) or resolved to a module.
///
/// The `module_name` is kept for diagnostics and pretty-printing.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub enum ResolvedName {
    Unresolved(String),
    Resolved {
        module: ModuleId,
        module_name: String,
        name: String,
    },
}

impl ResolvedName {
    pub fn unresolved(s: impl Into<String>) -> Self {
        ResolvedName::Unresolved(s.into())
    }

    pub fn local_name(&self) -> &str {
        match self {
            ResolvedName::Unresolved(s) => s,
            ResolvedName::Resolved { name, .. } => name,
        }
    }

    pub fn qualified_text(&self) -> String {
        match self {
            ResolvedName::Unresolved(s) => s.clone(),
            ResolvedName::Resolved {
                module_name, name, ..
            } => format!("{module_name}.{name}"),
        }
    }

    pub fn is_unresolved_eq(&self, s: &str) -> bool {
        matches!(self, ResolvedName::Unresolved(x) if x == s)
    }
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
    pub doc: Option<String>,
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
    /// Instance context constraints (Haskell-style): `instance (C a, D a) => E (F a) where ...`.
    pub preds: Vec<Predicate>,
    pub class: ClassId,
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
    pub doc: Option<String>,
    pub pat: Pattern,
    pub expr: Expr,
    pub span: Span,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TypeAlias {
    pub doc: Option<String>,
    pub name: String,
    pub params: Vec<String>,
    pub ty: Type,
    pub span: Span,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DataDecl {
    pub doc: Option<String>,
    pub name: String,
    pub params: Vec<String>,
    pub ctors: Vec<DataCtor>,
    pub deriving: Vec<String>,
    pub span: Span,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DataCtor {
    pub doc: Option<String>,
    pub name: String,
    pub args: Vec<Type>,
    pub span: Span,
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
    Ctor(ResolvedName),
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
        name: ResolvedName,
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
        class: ClassId,
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
