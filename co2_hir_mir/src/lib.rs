use std::collections::{HashMap, HashSet};

use co2_parser::{
    parse_compound_statement, parse_items, Constant, Declaration, Expression, InitDeclarator,
    Item, LazyCompoundStatement, RustPath, RustType, Span, Spanned, Statement,
    StatementOrDeclaration, State,
};

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
    Assign { dst: usize, src: Operand },
    Call { func: Callee, args: Vec<Operand>, dest: Option<usize> },
    Return,
}

#[derive(Clone, Debug)]
pub struct MirModule {
    pub uses: Vec<String>,
    pub externs: Vec<ExternFunction>,
    pub functions: Vec<Function>,
}

#[derive(Default)]
struct Resolver {
    type_names: HashSet<String>,
}

impl Resolver {
    fn new() -> Self {
        let mut r = Resolver::default();
        r.type_names.insert("int".to_owned());
        r.type_names.insert("char".to_owned());
        r.type_names.insert("void".to_owned());
        r
    }

    fn register_typedef(&mut self, name: &str) {
        self.type_names.insert(name.to_owned());
    }

    fn register_use(&mut self, path: &str) {
        let last = path.rsplit("::").next().unwrap_or(path);
        self.type_names.insert(last.to_owned());
        self.type_names.insert(path.to_owned());
    }
}

pub fn parse_and_lower(filename: String, src: &'static str) -> Result<MirModule, String> {
    let state = parse_items(filename.clone(), src).ok_or("parse failed")?;
    lower_state(filename, src, state)
}

fn lower_state(filename: String, src: &'static str, state: State) -> Result<MirModule, String> {
    let mut uses = Vec::new();
    let mut externs = Vec::new();
    let mut functions = Vec::new();

    let mut resolver = Resolver::new();

    for (item, _) in &state.items {
        match item {
            Item::Use(use_item) => {
                let path = use_path_to_string(&use_item.path);
                uses.push(path.clone());
                resolver.register_use(&path);
            }
            Item::TypeDef { name, .. } => {
                resolver.register_typedef(&name.0);
            }
            Item::Struct { name, .. } => {
                resolver.register_typedef(&name.0);
            }
            _ => {}
        }
    }

    for (item, span) in state.items {
        match item {
            Item::Use(_) => {}
            Item::ExternFunction { name, sig } => {
                externs.push(ExternFunction {
                    name: name.0,
                    sig: lower_sig(&sig)?,
                });
            }
            Item::Function { name, sig, body } => {
                let sig = lower_sig(&sig)?;
                let func = lower_function(
                    filename.clone(),
                    src,
                    &resolver,
                    name.0,
                    sig,
                    body,
                    span,
                )?;
                functions.push(func);
            }
            Item::TypeDef { .. } | Item::Static { .. } | Item::Struct { .. } => {
                // TODO: extend
            }
        }
    }

    Ok(MirModule { uses, externs, functions })
}

fn lower_sig(sig: &co2_parser::FnSig) -> Result<FuncSig, String> {
    let params = sig.inputs.iter().map(lower_type).collect::<Result<Vec<_>, _>>()?;
    let ret = lower_type(&sig.output)?;
    Ok(FuncSig { params, ret })
}

fn lower_type(ty: &RustType) -> Result<Type, String> {
    match ty {
        RustType::Void => Ok(Type::Void),
        RustType::Int(n) => {
            if *n == 1 {
                Ok(Type::Char)
            } else {
                Ok(Type::Int)
            }
        }
        RustType::TypeDef(path) => Ok(Type::RustPath(path.clone())),
        RustType::Function(_) => Err("function types are not supported".to_owned()),
        RustType::Ptr(inner) => Ok(Type::Ptr(Box::new(lower_type(&inner.0)?))),
        RustType::Array(inner) => Ok(Type::Array(Box::new(lower_type(&inner.0)?))),
    }
}

fn rust_path_to_string(path: &RustPath) -> String {
    path.segments
        .iter()
        .map(|seg| match &seg.0 {
            co2_parser::RustPathSegment::Ident(s) => s.clone(),
            co2_parser::RustPathSegment::Generics(parts) => {
                let inner = parts.iter().map(|p| rust_path_to_string(&p.0)).collect::<Vec<_>>().join(", ");
                format!("<{inner}>")
            }
        })
        .collect::<Vec<_>>()
        .join("::")
}

fn use_path_to_string(path: &[Spanned<String>]) -> String {
    path.iter()
        .map(|segment| segment.0.clone())
        .collect::<Vec<_>>()
        .join("::")
}

fn lower_function(
    filename: String,
    src: &'static str,
    resolver: &Resolver,
    name: String,
    sig: FuncSig,
    body: LazyCompoundStatement,
    span: Span,
) -> Result<Function, String> {
    let mut locals = Vec::new();
    let mut params = Vec::new();
    let mut locals_map = HashMap::new();

    locals.push(LocalDecl { name: "_ret".to_owned(), ty: sig.ret.clone() });

    for (i, ty) in sig.params.iter().enumerate() {
        let pname = format!("arg{i}");
        params.push(locals.len());
        locals_map.insert(pname.clone(), locals.len());
        locals.push(LocalDecl { name: pname, ty: ty.clone() });
    }

    let Some((compound, _)) = parse_compound_statement(&body.tokens.0, filename.clone(), src) else {
        return Err("failed to parse function body".to_owned());
    };

    let mut ops = Vec::new();

    for stmt in compound.statements {
        match stmt.0 {
            StatementOrDeclaration::Statement(s) => lower_stmt(s.0, &mut locals, &mut locals_map, &mut ops)?,
            StatementOrDeclaration::Declaration(d) => lower_decl(d.0, resolver, &mut locals, &mut locals_map, &mut ops)?,
        }
    }

    ops.push(MirOp::Return);

    Ok(Function { name, sig, locals, params, ops })
}

fn lower_decl(
    decl: Declaration,
    _resolver: &Resolver,
    locals: &mut Vec<LocalDecl>,
    locals_map: &mut HashMap<String, usize>,
    ops: &mut Vec<MirOp>,
) -> Result<(), String> {
    match decl {
        Declaration::FunctionDefinition { .. } => {
            // Nested functions are not supported.
            Ok(())
        }
        Declaration::Declaration { declaration_specifiers, declarators } => {
            let base_span = declarators
                .first()
                .map(|d| d.1)
                .unwrap_or_else(|| Span::from(0..0));
            let base = co2_parser::type_ir::base_type_of_decl::<RustType>(
                declaration_specifiers.clone(),
                base_span,
            );
            for decl in declarators {
                let InitDeclarator { declarator, initializer } = decl.0;
                let (rust_ty, name) =
                    co2_parser::type_ir::extract_type_of_decl::<RustType>(base.clone(), declarator);
                let Some((name, _span)) = name else {
                    continue;
                };
                let ty = lower_type(&rust_ty)?;
                let local = locals.len();
                locals.push(LocalDecl { name: name.clone(), ty });
                locals_map.insert(name.clone(), local);
                if let Some(init) = initializer {
                    match init.0 {
                        Expression::Call { func, params } => {
                            let (args, mut pre) = lower_call_args(params, locals, locals_map)?;
                            ops.append(&mut pre);
                            let callee = lower_callee(func.0)?;
                            ops.push(MirOp::Call { func: callee, args, dest: Some(local) });
                        }
                        other => {
                            let (operand, mut pre) =
                                lower_expr((other, init.1), locals, locals_map)?;
                            ops.append(&mut pre);
                            ops.push(MirOp::Assign { dst: local, src: operand });
                        }
                    }
                }
            }
            Ok(())
        }
    }
}

fn lower_stmt(
    stmt: Statement,
    locals: &mut Vec<LocalDecl>,
    locals_map: &mut HashMap<String, usize>,
    ops: &mut Vec<MirOp>,
) -> Result<(), String> {
    match stmt {
        Statement::Return(expr) => {
            if let Some(expr) = expr {
                match expr.0 {
                    Expression::Call { func, params } => {
                        let (args, mut pre) = lower_call_args(params, locals, locals_map)?;
                        ops.append(&mut pre);
                        let callee = lower_callee(func.0)?;
                        ops.push(MirOp::Call { func: callee, args, dest: Some(0) });
                    }
                    other => {
                        let (operand, mut pre) =
                            lower_expr((other, expr.1), locals, locals_map)?;
                        ops.append(&mut pre);
                        ops.push(MirOp::Assign { dst: 0, src: operand });
                    }
                }
            }
            ops.push(MirOp::Return);
        }
        Statement::Expression(expr) => {
            match expr.0 {
                Expression::Call { func, params } => {
                    let (args, mut pre) = lower_call_args(params, locals, locals_map)?;
                    ops.append(&mut pre);
                    let callee = lower_callee(func.0)?;
                    ops.push(MirOp::Call { func: callee, args, dest: None });
                }
                other => {
                    let (operand, mut pre) =
                        lower_expr((other, expr.1), locals, locals_map)?;
                    ops.append(&mut pre);
                    match operand {
                        Operand::Local(_) | Operand::ConstInt(_, _) | Operand::ConstStr(_, _) => {}
                    }
                }
            }
        }
    }
    Ok(())
}

fn lower_expr(
    expr: Spanned<Expression>,
    locals: &mut Vec<LocalDecl>,
    locals_map: &mut HashMap<String, usize>,
) -> Result<(Operand, Vec<MirOp>), String> {
    match expr.0 {
        Expression::Identifier(path) => {
            let name = rust_path_to_string(&path.0);
            if let Some(&local) = locals_map.get(&name) {
                Ok((Operand::Local(local), Vec::new()))
            } else {
                Ok((Operand::Local(insert_temp(locals, locals_map, &name)), Vec::new()))
            }
        }
        Expression::Constant(c) => match c {
            Constant::Int(i) => Ok((Operand::ConstInt(i as i64, expr.1), Vec::new())),
            Constant::String(s) => Ok((Operand::ConstStr(s, expr.1), Vec::new())),
        },
        Expression::Call { func: _, params: _ } => {
            Err("call expressions are only supported in statements or initializers".to_owned())
        }
        Expression::Subscript(_, _) => Err("subscript is not supported yet".to_owned()),
        Expression::BinOp(_, _) => Err("binary op is not supported yet".to_owned()),
        Expression::Empty => Err("empty expression".to_owned()),
    }
}

fn insert_temp(
    locals: &mut Vec<LocalDecl>,
    locals_map: &mut HashMap<String, usize>,
    name: &str,
) -> usize {
    let local = locals.len();
    locals.push(LocalDecl { name: name.to_owned(), ty: Type::Int });
    locals_map.insert(name.to_owned(), local);
    local
}

fn lower_call_args(
    params: Vec<Spanned<Expression>>,
    locals: &mut Vec<LocalDecl>,
    locals_map: &mut HashMap<String, usize>,
) -> Result<(Vec<Operand>, Vec<MirOp>), String> {
    let mut ops = Vec::new();
    let mut args = Vec::new();
    for param in params {
        let (op, mut pre) = lower_expr(param, locals, locals_map)?;
        ops.append(&mut pre);
        args.push(op);
    }
    Ok((args, ops))
}

fn lower_callee(expr: Expression) -> Result<Callee, String> {
    match expr {
        Expression::Identifier(path) => Ok(Callee::Path(rust_path_to_string(&path.0))),
        other => Err(format!("unsupported callee: {other:?}")),
    }
}
