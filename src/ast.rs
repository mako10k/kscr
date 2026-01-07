#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Module {
    pub name: Option<String>,
    pub items: Vec<Item>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Item {
    Import(ImportDecl),
    Export(ExportDecl),
    Binding(Binding),
    TypeAlias(TypeAlias),
    DataDecl(DataDecl),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ImportDecl {
    pub module: String,
    pub as_name: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ExportDecl {
    pub names: Vec<String>,
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
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DataCtor {
    pub name: String,
    pub args: Vec<Type>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Expr {
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
pub enum Pattern {
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
    Constructor { name: String, args: Vec<Pattern> },
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
    App { head: Box<Type>, args: Vec<Type> },
    Func(Box<Type>, Box<Type>),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Predicate {
    Show(Type),
    ShowRow(Type),
    Lacks { label: String, row: Type },
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct QualType {
    pub preds: Vec<Predicate>,
    pub ty: Type,
}
