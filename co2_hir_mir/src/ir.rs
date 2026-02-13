use co2_parser::{RustPath, Span};

#[derive(Clone, Debug)]
pub enum Type {
    Void,
    Int,
    Char,
    Ptr(Box<Type>),
    Array(Box<Type>),
    RustPath(RustPath),
}

#[derive(Clone, Debug)]
pub struct FuncSig {
    pub params: Vec<Type>,
    pub ret: Type,
}

#[derive(Clone, Debug)]
pub struct ExternFunction {
    pub name: String,
    pub sig: FuncSig,
}

#[derive(Clone, Debug)]
pub struct Function {
    pub name: String,
    pub sig: FuncSig,
    pub locals: Vec<LocalDecl>,
    pub params: Vec<usize>,
    pub ops: Vec<MirOp>,
}

#[derive(Clone, Debug)]
pub struct LocalDecl {
    pub name: String,
    pub ty: Type,
}

#[derive(Clone, Debug)]
pub enum Operand {
    Local(usize),
    ConstInt(i64, Span),
    ConstStr(String, Span),
}

#[derive(Clone, Debug)]
pub enum Callee {
    Path(String),
}

#[derive(Clone, Debug)]
pub enum MirOp {
    Assign {
        dst: usize,
        src: Operand,
    },
    Call {
        func: Callee,
        args: Vec<Operand>,
        dest: Option<usize>,
    },
    Return,
}

#[derive(Clone, Debug)]
pub struct MirModule {
    pub uses: Vec<String>,
    pub externs: Vec<ExternFunction>,
    pub functions: Vec<Function>,
}
