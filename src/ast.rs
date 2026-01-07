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
        ty: Type,
    },
    Do(Vec<DoStmt>),
    Case {
        expr: Box<Expr>,
        arms: Vec<(Pattern, Expr)>,
    },
    List(Vec<Expr>),
    Tuple(Vec<Expr>),
    Record(Vec<(String, Expr)>),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DoStmt {
    Bind { name: String, expr: Expr },
    Expr(Expr),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Pattern {
    Var(String),
    Wildcard,
    Literal(Expr),
    Tuple(Vec<Pattern>),
    List(Vec<Pattern>),
    Record(Vec<(String, Pattern)>),
    Cons(Box<Pattern>, Box<Pattern>),
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

    Hole(Option<String>),
    Var(String),
    App { head: Box<Type>, args: Vec<Type> },
    Func(Box<Type>, Box<Type>),
}
