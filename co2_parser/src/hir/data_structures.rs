use std::{collections::HashMap, fmt::Debug};

use la_arena::{Arena, Idx};

use crate::{Span, parser::LazyCompoundStatement};

#[derive(Debug)]
pub struct Stmt<C: HirCtxInterface> {
    pub kind: StmtKind<C>,
    pub span: Span,
}

pub struct LocalData<C: HirCtxInterface> {
    ty: C::Ty,
}

pub struct LabelData;

pub type Local<C> = Idx<LocalData<C>>;
pub type Label = Idx<LabelData>;

#[derive(Debug)]
pub enum StmtKind<C: HirCtxInterface> {
    Block(Block<C>),
    Expr(Expr<C>),
    Decl(Vec<Local<C>>),
    Ret(Option<Expr<C>>),
    Label(Label, Option<Box<Stmt<C>>>),
    Goto(Label),
    If(Expr<C>, Box<Stmt<C>>, Option<Box<Stmt<C>>>),
    Noop,
}

#[derive(Debug)]
pub struct Block<C: HirCtxInterface> {
    pub stmts: Vec<Stmt<C>>,
    pub span: Span,
}

#[derive(Debug)]
pub struct Expr<C: HirCtxInterface> {
    pub kind: ExprKind<C>,
    pub ty: C::Ty,
    pub span: Span,
}

#[derive(Debug)]
pub enum ExprKind<C: HirCtxInterface> {
    Lit(Lit),
    Local(Local<C>),
    Call(Box<Expr<C>>, Vec<Expr<C>>),
    Binary(BinOp, Box<Expr<C>>, Box<Expr<C>>),
    Unary(UnOp, Box<Expr<C>>),
    Assign(Box<Expr<C>>, Box<Expr<C>>),
    AssignWithBinOp(Box<Expr<C>>, Box<Expr<C>>, BinOp, C::Ty, ReturnSemantic),
    Field(Box<Expr<C>>, usize),
    PtrOffset(Box<Expr<C>>, Box<Expr<C>>),
    PtrDiff(Box<Expr<C>>, Box<Expr<C>>),
    AssignPtrOffset(Box<Expr<C>>, Box<Expr<C>>, ReturnSemantic),
    Cast(Box<Expr<C>>),
    InitializerList(Box<InitializerTree<C>>),
    Comma(Vec<Expr<C>>),
    // Sizeof(Sizeof),
    // VaArg(Box<Expr<C>>, Ty),
    OffsetOf,
    Cond(Box<Expr<C>>, Box<Expr<C>>, Box<Expr<C>>),
    GnuBlock(Block<C>),
    Empty,
}

#[derive(Debug, Clone)]
pub struct Lit {
    pub kind: LitKind,
    pub span: Span,
}

#[derive(Debug, Clone)]
pub enum LitKind {
    Str(String),
    Char(char),
    Int(i128),
    Float(f64),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BinOp {
    Add,
    Sub,
    Mul,
    Div,
    Rem,
    Or,
    And,
    BitOr,
    BitXor,
    BitAnd,
    Eq,
    Lt,
    Le,
    Ne,
    Ge,
    Gt,
    Shl,
    Shr,
}

impl BinOp {
    const COMPARISONS: &[Self] = &[
        BinOp::Eq,
        BinOp::Le,
        BinOp::Ge,
        BinOp::Gt,
        BinOp::Lt,
        BinOp::Ne,
    ];
    const SHORT_CIRCUITS: &[Self] = &[BinOp::And, BinOp::Or];

    fn to_un_op(self) -> Option<UnOp> {
        match self {
            BinOp::Add => Some(UnOp::Pos),
            BinOp::Sub => Some(UnOp::Neg),
            BinOp::Mul => Some(UnOp::Deref),
            BinOp::And => Some(UnOp::AddrOf),
            BinOp::Div
            | BinOp::Rem
            | BinOp::Or
            | BinOp::BitOr
            | BinOp::BitXor
            | BinOp::BitAnd
            | BinOp::Eq
            | BinOp::Lt
            | BinOp::Le
            | BinOp::Ne
            | BinOp::Ge
            | BinOp::Gt
            | BinOp::Shl
            | BinOp::Shr => None,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum UnOp {
    Not,
    Neg,
    Com,
    Pos,
    AddrOf,
    Deref,
}

#[derive(Debug)]
pub enum ReturnSemantic {
    /// For x += 1 and ++x
    AfterAssign,
    /// For x++
    BeforeAssign,
}

#[derive(Debug)]
pub enum DesignatorKind {
    Subscript { value: i128 },
    Field { name: String },
}

#[derive(Debug)]
pub struct Designator {
    pub kind: DesignatorKind,
    pub span: Span,
}

// #[derive(Debug)]
// pub struct InitializerItem {
//     pub designators: Option<Vec<Designator>>,
//     pub value: ExprOrList,
// }

// #[derive(Debug)]
// pub enum ExprOrList {
//     Expr(Expr),
//     List(Vec<InitializerItem>),
// }

#[derive(Debug)]
pub enum InitializerTree<C: HirCtxInterface> {
    Middle { children: Vec<InitializerTree<C>> },
    Leaf(Expr<C>),
    Zeroed,
}

pub struct HirBody<C: HirCtxInterface> {
    pub locals: Arena<LocalData<C>>,
    pub root: Block<C>,
}

pub trait HirCtxInterface {
    type Ty: Debug;
}
