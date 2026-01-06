#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Module {
    pub items: Vec<Item>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Item {
    Binding(Binding),
    TypeAlias(TypeAlias),
    DataDecl(DataDecl),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Binding {
    pub name: String,
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
    Integer(String),
    Float64(String),
    Bool(bool),
    String(String),
    Var(String),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Type {
    Unit,
    Integer,
    Bool,
    Float64,
    String,
    Var(String),
    App { head: Box<Type>, args: Vec<Type> },
    Func(Box<Type>, Box<Type>),
}
