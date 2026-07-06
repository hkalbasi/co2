use std::{
    ascii::escape_default,
    fmt::{self, Write},
    sync::Arc,
};

use crate::{
    BinOp, CompoundStatement, Constant, Declaration, DeclarationSpecifier, Declarator, Designator,
    EnumSpecifier, Expression, FileId, FloatSuffix, ForInit, FunctionDefinitionSignature,
    FunctionSpecifier, GenericAssociation, InitDeclarator, Initializer, InitializerItem,
    IntegerSuffix, LazyCompoundStatement, LazyRustConstExpr, LazySubscription, ModItem, PackAction,
    ParameterList, RustAttribute, RustFunctionParam, RustFunctionSignature, RustStructField,
    RustTy, Span, Spanned, SpecifierQualifier, Statement, StatementOrDeclaration,
    StorageClassSpecifier, StructDeclarator, StructOrUnionField, StructOrUnionKind,
    StructOrUnionSpecifier, TranslationUnit, TypeName, TypeQualifier, TypeSpecifier, UnaryOp,
    UpdateOp, UseItem, Visibility,
};

pub trait TypeResolver
where
    Self: crate::TypeResolver,
{
    fn pretty_print_su_ident(
        pp: &mut PrettyPrinter,
        item: &<Self as crate::TypeResolver>::StructOrUnionIdentifier,
    );
}

impl<T> TypeResolver for T
where
    T: crate::TypeResolver,
    <T as crate::TypeResolver>::StructOrUnionIdentifier: PrettyPrint,
{
    fn pretty_print_su_ident(
        pp: &mut PrettyPrinter,
        item: &<T as crate::TypeResolver>::StructOrUnionIdentifier,
    ) {
        item.pretty_print(pp);
    }
}

/// Pretty-print a `Spanned<CompoundStatement<LocalResolver>>` with default config.
pub fn pretty_print_compound<R: TypeResolver>(compound: &Spanned<CompoundStatement<R>>) {
    let config = PrettyConfig::default();
    let mut pp = PrettyPrinter::new(&config);
    compound.pretty_print(&mut pp);
    eprintln!("{}", pp.finish());
}

// ---------------------------------------------------------------------------
// Configuration
// ---------------------------------------------------------------------------

pub struct PrettyConfig {
    /// Indentation per nesting level (default 2).
    pub indent: usize,
    /// Prepend file-id (or file name) before the byte span.
    pub show_file_name: bool,
    /// Map `FileId` to a human-readable file name. When `None`, the raw `FileId` is shown.
    pub file_name_for_id: Option<Arc<dyn Fn(FileId) -> String>>,
}

impl Default for PrettyConfig {
    fn default() -> Self {
        Self {
            indent: 2,
            show_file_name: false,
            file_name_for_id: None,
        }
    }
}

// ---------------------------------------------------------------------------
// Pretty-printer state
// ---------------------------------------------------------------------------

pub struct PrettyPrinter<'a> {
    pub config: &'a PrettyConfig,
    output: String,
    indent: usize,
}

impl<'a> PrettyPrinter<'a> {
    pub fn new(config: &'a PrettyConfig) -> Self {
        Self {
            config,
            output: String::new(),
            indent: 0,
        }
    }

    /// Interior node with children.
    pub fn node(&mut self, name: &str, span: &str, f: impl FnOnce(&mut Self)) {
        self.write_indent();
        self.output.push_str(name);
        self.output.push_str(span);
        self.output.push('\n');
        self.indent += self.config.indent;
        f(self);
        self.indent -= self.config.indent;
    }

    /// Leaf node (no children).
    pub fn leaf(&mut self, name: &str, span: &str) {
        self.write_indent();
        self.output.push_str(name);
        self.output.push_str(span);
        self.output.push('\n');
    }

    /// Leaf node with extra data.
    pub fn leaf_data(&mut self, name: &str, span: &str, data: impl fmt::Display) {
        self.write_indent();
        self.output.push_str(name);
        self.output.push_str(span);
        write!(self.output, " {data}").unwrap();
        self.output.push('\n');
    }

    /// Data-only line (no name, no span).
    pub fn data(&mut self, data: impl fmt::Display) {
        self.write_indent();
        write!(self.output, "{data}").unwrap();
        self.output.push('\n');
    }

    fn write_indent(&mut self) {
        for _ in 0..self.indent {
            self.output.push(' ');
        }
    }

    pub fn finish(self) -> String {
        self.output
    }
}

// ---------------------------------------------------------------------------
// Traits
// ---------------------------------------------------------------------------

pub trait PrettyPrint {
    fn pretty_print(&self, pp: &mut PrettyPrinter);
}

// ---------------------------------------------------------------------------
// Span formatting
// ---------------------------------------------------------------------------

pub fn fmt_span(span: &Span, config: &PrettyConfig) -> String {
    let span = span.data();
    if config.show_file_name {
        if let Some(ref f) = config.file_name_for_id {
            let name = (f)(span.context);
            format!("@{}:{}..{}", name, span.start, span.end)
        } else {
            format!("@{:?}:{}..{}", span.context, span.start, span.end)
        }
    } else {
        format!("@{}..{}", span.start, span.end)
    }
}

// ---------------------------------------------------------------------------
// Blanket impls (Box, Vec, Option)
// ---------------------------------------------------------------------------

impl<T: PrettyPrint> PrettyPrint for Box<T> {
    fn pretty_print(&self, pp: &mut PrettyPrinter) {
        (**self).pretty_print(pp);
    }
}

impl<T: PrettyPrint> PrettyPrint for Vec<T> {
    fn pretty_print(&self, pp: &mut PrettyPrinter) {
        for item in self {
            item.pretty_print(pp);
        }
    }
}

impl<T: PrettyPrint> PrettyPrint for Option<T> {
    fn pretty_print(&self, pp: &mut PrettyPrinter) {
        if let Some(inner) = self {
            inner.pretty_print(pp);
        }
    }
}

// ---------------------------------------------------------------------------
// Constants and operators
// ---------------------------------------------------------------------------

impl PrettyPrint for Constant {
    fn pretty_print(&self, pp: &mut PrettyPrinter) {
        match self {
            Constant::Int(v, suffix) => {
                let s = match suffix {
                    IntegerSuffix::None => format!("{v}"),
                    IntegerSuffix::Unsigned => format!("{v}u"),
                    IntegerSuffix::Long => format!("{v}l"),
                    IntegerSuffix::LongLong => format!("{v}ll"),
                    IntegerSuffix::UnsignedLong => format!("{v}ul"),
                    IntegerSuffix::UnsignedLongLong => format!("{v}ull"),
                    IntegerSuffix::Usize => format!("{v}usize"),
                    IntegerSuffix::Isize => format!("{v}isize"),
                    IntegerSuffix::U8 => format!("{v}u8"),
                    IntegerSuffix::U16 => format!("{v}u16"),
                    IntegerSuffix::U32 => format!("{v}u32"),
                    IntegerSuffix::U64 => format!("{v}u64"),
                    IntegerSuffix::U128 => format!("{v}u128"),
                    IntegerSuffix::I8 => format!("{v}i8"),
                    IntegerSuffix::I16 => format!("{v}i16"),
                    IntegerSuffix::I32 => format!("{v}i32"),
                    IntegerSuffix::I64 => format!("{v}i64"),
                    IntegerSuffix::I128 => format!("{v}i128"),
                };
                pp.leaf_data("Int", "", &s);
            }
            Constant::Float(v, suffix) => {
                let s = match suffix {
                    FloatSuffix::None => format!("{v}"),
                    FloatSuffix::Float => format!("{v}f"),
                    FloatSuffix::Long => format!("{v}l"),
                };
                pp.leaf_data("Float", "", &s);
            }
            Constant::Char(c) => {
                if let Ok(b) = u8::try_from(*c) {
                    let escaped: String = escape_default(b).map(|e| e as char).collect();
                    pp.leaf_data("Char", "", format_args!("'{escaped}'"));
                } else {
                    pp.leaf_data("Char", "", format_args!("U+{c:X}"));
                }
            }
            Constant::String(literal) => {
                let escaped: String = literal
                    .bytes
                    .iter()
                    .copied()
                    .flat_map(escape_default)
                    .map(|e| e as char)
                    .collect();
                pp.leaf_data(
                    "String",
                    "",
                    format_args!("{}\"{escaped}\"", literal.prefix.as_str()),
                );
            }
        }
    }
}

impl PrettyPrint for BinOp {
    fn pretty_print(&self, pp: &mut PrettyPrinter) {
        pp.data(format_args!("{self:?}"));
    }
}

impl PrettyPrint for UnaryOp {
    fn pretty_print(&self, pp: &mut PrettyPrinter) {
        pp.data(format_args!("{self:?}"));
    }
}

impl PrettyPrint for UpdateOp {
    fn pretty_print(&self, pp: &mut PrettyPrinter) {
        pp.data(format_args!("{self:?}"));
    }
}

// ---------------------------------------------------------------------------
// Qualifiers and specifiers (always wrapped in Spanned)
// ---------------------------------------------------------------------------

impl PrettyPrint for Spanned<TypeQualifier> {
    fn pretty_print(&self, pp: &mut PrettyPrinter) {
        let sp = fmt_span(&self.1, pp.config);
        match self.0 {
            TypeQualifier::Const => pp.leaf("Const", &sp),
            TypeQualifier::Constexpr => pp.leaf("Constexpr", &sp),
            TypeQualifier::Restrict => pp.leaf("Restrict", &sp),
            TypeQualifier::Volatile => pp.leaf("Volatile", &sp),
            TypeQualifier::Atomic => pp.leaf("Atomic", &sp),
        }
    }
}

impl PrettyPrint for Spanned<StorageClassSpecifier> {
    fn pretty_print(&self, pp: &mut PrettyPrinter) {
        let sp = fmt_span(&self.1, pp.config);
        match self.0 {
            StorageClassSpecifier::Typedef => pp.leaf("Typedef", &sp),
            StorageClassSpecifier::Extern => pp.leaf("Extern", &sp),
            StorageClassSpecifier::Static => pp.leaf("Static", &sp),
            StorageClassSpecifier::Constexpr => pp.leaf("Constexpr", &sp),
            StorageClassSpecifier::Atomic => pp.leaf("Atomic", &sp),
            StorageClassSpecifier::ThreadLocal => pp.leaf("ThreadLocal", &sp),
            StorageClassSpecifier::Auto => pp.leaf("Auto", &sp),
            StorageClassSpecifier::Register => pp.leaf("Register", &sp),
        }
    }
}

impl PrettyPrint for Spanned<FunctionSpecifier> {
    fn pretty_print(&self, pp: &mut PrettyPrinter) {
        let sp = fmt_span(&self.1, pp.config);
        match self.0 {
            FunctionSpecifier::Inline => pp.leaf("Inline", &sp),
        }
    }
}

impl PrettyPrint for PackAction {
    fn pretty_print(&self, pp: &mut PrettyPrinter) {
        match self {
            PackAction::PushSet(n) => pp.data(format_args!("PushSet({n})")),
            PackAction::PushOnly => pp.data("PushOnly"),
            PackAction::Pop => pp.data("Pop"),
            PackAction::Set(n) => pp.data(format_args!("Set({n})")),
            PackAction::Reset => pp.data("Reset"),
        }
    }
}

// ---------------------------------------------------------------------------
// Spanned<DeclarationSpecifier<R>>
// ---------------------------------------------------------------------------

impl<R: TypeResolver> PrettyPrint for Spanned<DeclarationSpecifier<R>> {
    fn pretty_print(&self, pp: &mut PrettyPrinter) {
        let sp = fmt_span(&self.1, pp.config);
        match &self.0 {
            DeclarationSpecifier::TypeSpecifier(ts) => {
                pp.node("TypeSpecifier", &sp, |pp| ts.pretty_print(pp));
            }
            DeclarationSpecifier::TypeQualifier(tq) => {
                pp.node("TypeQualifier", &sp, |pp| tq.pretty_print(pp));
            }
            DeclarationSpecifier::StorageSpecifier(ss) => {
                pp.node("StorageSpecifier", &sp, |pp| ss.pretty_print(pp));
            }
            DeclarationSpecifier::FunctionSpecifier(fs) => {
                pp.node("FunctionSpecifier", &sp, |pp| fs.pretty_print(pp));
            }
        }
    }
}

// ---------------------------------------------------------------------------
// Spanned<TypeSpecifier<R>>
// ---------------------------------------------------------------------------

impl<R: TypeResolver> PrettyPrint for Spanned<TypeSpecifier<R>> {
    fn pretty_print(&self, pp: &mut PrettyPrinter) {
        let sp = fmt_span(&self.1, pp.config);
        match &self.0 {
            TypeSpecifier::Int => pp.leaf("Int", &sp),
            TypeSpecifier::Bool => pp.leaf("Bool", &sp),
            TypeSpecifier::Void => pp.leaf("Void", &sp),
            TypeSpecifier::Char => pp.leaf("Char", &sp),
            TypeSpecifier::Short => pp.leaf("Short", &sp),
            TypeSpecifier::Long => pp.leaf("Long", &sp),
            TypeSpecifier::Float => pp.leaf("Float", &sp),
            TypeSpecifier::Double => pp.leaf("Double", &sp),
            TypeSpecifier::Signed => pp.leaf("Signed", &sp),
            TypeSpecifier::Unsigned => pp.leaf("Unsigned", &sp),
            TypeSpecifier::StructOrUnion { kind, specifier } => {
                let label = if matches!(kind, StructOrUnionKind::Struct) {
                    "Struct"
                } else {
                    "Union"
                };
                pp.node(label, &sp, |pp| {
                    let sp = fmt_span(&specifier.1, pp.config);
                    pp.node("Specifier", &sp, |pp| {
                        R::pretty_print_su_ident(pp, &specifier.0);
                    });
                });
            }
            TypeSpecifier::Enum(specifier) => {
                pp.node("Enum", &sp, |pp| {
                    let sp = fmt_span(&specifier.1, pp.config);
                    pp.leaf_data("Specifier", &sp, format_args!("{:?}", &specifier.0));
                });
            }
            TypeSpecifier::TypedefName(path) => {
                pp.leaf_data("TypedefName", &sp, format_args!("{:?}", &path.0));
            }
            TypeSpecifier::TypeofType(ty) => {
                pp.node("TypeofType", &sp, |pp| ty.pretty_print(pp));
            }
            TypeSpecifier::TypeofExpr(expr) => {
                pp.node("TypeofExpr", &sp, |pp| expr.pretty_print(pp));
            }
            TypeSpecifier::Alignas => {
                pp.node("Alignas", &sp, |_pp| ());
            }
        }
    }
}

impl PrettyPrint for StructOrUnionSpecifier<crate::StatelessResolver> {
    fn pretty_print(&self, pp: &mut PrettyPrinter) {
        match self {
            StructOrUnionSpecifier::Defined { ident, fields } => {
                pp.node("Defined", "", |pp| {
                    pp.leaf_data("Ident", &fmt_span(&ident.1, pp.config), &ident.0);
                    for f in fields {
                        f.pretty_print(pp);
                    }
                });
            }
            StructOrUnionSpecifier::Declared { ident } => {
                pp.node("Declared", "", |pp| {
                    pp.leaf_data("Ident", &fmt_span(&ident.1, pp.config), &ident.0);
                });
            }
            StructOrUnionSpecifier::Anonymous { fields } => {
                pp.node("Anonymous", "", |pp| {
                    for f in fields {
                        f.pretty_print(pp);
                    }
                });
            }
        }
    }
}

// ---------------------------------------------------------------------------
// Spanned<SpecifierQualifier<R>>
// ---------------------------------------------------------------------------

impl<R: TypeResolver> PrettyPrint for Spanned<SpecifierQualifier<R>> {
    fn pretty_print(&self, pp: &mut PrettyPrinter) {
        let sp = fmt_span(&self.1, pp.config);
        match &self.0 {
            SpecifierQualifier::TypeSpecifier(ts) => {
                pp.node("TypeSpecifier", &sp, |pp| ts.pretty_print(pp));
            }
            SpecifierQualifier::TypeQualifier(tq) => {
                pp.node("TypeQualifier", &sp, |pp| tq.pretty_print(pp));
            }
        }
    }
}

// ---------------------------------------------------------------------------
// Spanned<Statement<R>>
// ---------------------------------------------------------------------------

impl<R: TypeResolver> PrettyPrint for Spanned<Statement<R>> {
    fn pretty_print(&self, pp: &mut PrettyPrinter) {
        let sp = fmt_span(&self.1, pp.config);
        match &self.0 {
            Statement::Empty => pp.leaf("Empty", &sp),
            Statement::Goto(label) => {
                pp.leaf_data("Goto", &sp, &label.0);
            }
            Statement::IndirectGoto(expr) => {
                pp.node("IndirectGoto", &sp, |pp| expr.pretty_print(pp));
            }
            Statement::Break => pp.leaf("Break", &sp),
            Statement::BreakCo2 => pp.leaf("BreakCo2", &sp),
            Statement::Continue => pp.leaf("Continue", &sp),
            Statement::Switch { expr, body } => {
                pp.node("Switch", &sp, |pp| {
                    expr.pretty_print(pp);
                    body.pretty_print(pp);
                });
            }
            Statement::Case { expr, statement } => {
                pp.node("Case", &sp, |pp| {
                    expr.pretty_print(pp);
                    statement.pretty_print(pp);
                });
            }
            Statement::Default {
                keyword_span: _,
                statement,
            } => {
                pp.node("Default", &sp, |pp| statement.pretty_print(pp));
            }
            Statement::Label { name, statement } => {
                pp.node("Label", &sp, |pp| {
                    pp.leaf_data("Name", &fmt_span(&name.1, pp.config), &name.0);
                    statement.pretty_print(pp);
                });
            }
            Statement::Return(expr) => {
                pp.node("Return", &sp, |pp| {
                    if let Some(e) = expr {
                        e.pretty_print(pp);
                    }
                });
            }
            Statement::Expression(expr) => {
                pp.node("Expression", &sp, |pp| expr.pretty_print(pp));
            }
            Statement::Compound(compound) => {
                pp.node("Compound", &sp, |pp| compound.pretty_print(pp));
            }
            Statement::If {
                cond,
                then_branch,
                else_branch,
            } => {
                pp.node("If", &sp, |pp| {
                    cond.pretty_print(pp);
                    then_branch.pretty_print(pp);
                    if let Some(el) = else_branch {
                        el.pretty_print(pp);
                    }
                });
            }
            Statement::While { cond, body } => {
                pp.node("While", &sp, |pp| {
                    cond.pretty_print(pp);
                    body.pretty_print(pp);
                });
            }
            Statement::DoWhile { body, cond } => {
                pp.node("DoWhile", &sp, |pp| {
                    body.pretty_print(pp);
                    cond.pretty_print(pp);
                });
            }
            Statement::For {
                init,
                cond,
                post,
                body,
            } => {
                pp.node("For", &sp, |pp| {
                    if let Some(i) = init {
                        i.pretty_print(pp);
                    }
                    if let Some(c) = cond {
                        c.pretty_print(pp);
                    }
                    if let Some(p) = post {
                        p.pretty_print(pp);
                    }
                    body.pretty_print(pp);
                });
            }
        }
    }
}

// ---------------------------------------------------------------------------
// ForInit<R>
// ---------------------------------------------------------------------------

impl<R: TypeResolver> PrettyPrint for ForInit<R> {
    fn pretty_print(&self, pp: &mut PrettyPrinter) {
        match self {
            ForInit::Expression(expr) => {
                pp.node("Expression", "", |pp| expr.pretty_print(pp));
            }
            ForInit::Declaration(decl) => {
                pp.node("Declaration", "", |pp| decl.pretty_print(pp));
            }
        }
    }
}

// ---------------------------------------------------------------------------
// Spanned<CompoundStatement<R>>
// ---------------------------------------------------------------------------

impl<R: TypeResolver> PrettyPrint for Spanned<CompoundStatement<R>> {
    fn pretty_print(&self, pp: &mut PrettyPrinter) {
        let sp = fmt_span(&self.1, pp.config);
        pp.node("CompoundStatement", &sp, |pp| {
            for stmt in &self.0.statements {
                stmt.pretty_print(pp);
            }
        });
    }
}

// ---------------------------------------------------------------------------
// Spanned<StatementOrDeclaration<R>>
// ---------------------------------------------------------------------------

impl<R: TypeResolver> PrettyPrint for Spanned<StatementOrDeclaration<R>> {
    fn pretty_print(&self, pp: &mut PrettyPrinter) {
        let sp = fmt_span(&self.1, pp.config);
        match &self.0 {
            StatementOrDeclaration::Declaration(d) => {
                pp.node("Declaration", &sp, |pp| d.pretty_print(pp));
            }
            StatementOrDeclaration::Statement(s) => {
                pp.node("Statement", &sp, |pp| s.pretty_print(pp));
            }
        }
    }
}

// ---------------------------------------------------------------------------
// Spanned<LazyCompoundStatement>
// ---------------------------------------------------------------------------

impl PrettyPrint for Spanned<LazyCompoundStatement> {
    fn pretty_print(&self, pp: &mut PrettyPrinter) {
        let sp = fmt_span(&self.1, pp.config);
        pp.leaf_data(
            "LazyCompoundStatement",
            &sp,
            format_args!("{} tokens", self.0.tokens.0.len()),
        );
    }
}

// ---------------------------------------------------------------------------
// Spanned<Expression<R>>
// ---------------------------------------------------------------------------

impl<R: TypeResolver> PrettyPrint for Spanned<Expression<R>> {
    fn pretty_print(&self, pp: &mut PrettyPrinter) {
        let sp = fmt_span(&self.1, pp.config);
        match &self.0 {
            Expression::Empty => pp.leaf("Empty", &sp),
            Expression::Constant(c) => {
                pp.node("Constant", &sp, |pp| c.pretty_print(pp));
            }
            Expression::Identifier(path) => {
                pp.leaf_data("Identifier", &sp, format_args!("{:?}", &path.0));
            }
            Expression::LabelAddress(name) => {
                pp.leaf_data("LabelAddress", &sp, &name.0);
            }
            Expression::Field(base, field) => {
                pp.node("Field", &sp, |pp| {
                    base.pretty_print(pp);
                    pp.leaf_data("FieldName", &fmt_span(&field.1, pp.config), &field.0);
                });
            }
            Expression::Arrow(base, field) => {
                pp.node("Arrow", &sp, |pp| {
                    base.pretty_print(pp);
                    pp.leaf_data("FieldName", &fmt_span(&field.1, pp.config), &field.0);
                });
            }
            Expression::MethodCall {
                receiver,
                method,
                generics,
                params,
            } => {
                pp.node("MethodCall", &sp, |pp| {
                    receiver.pretty_print(pp);
                    pp.leaf_data("Method", &fmt_span(&method.1, pp.config), &method.0);
                    if !generics.is_empty() {
                        pp.leaf(
                            "Generics",
                            &format!(
                                "<{}>",
                                generics
                                    .iter()
                                    .map(|g| format!("{:?}", g.0))
                                    .collect::<Vec<_>>()
                                    .join(", ")
                            ),
                        );
                    }
                    for param in params {
                        param.pretty_print(pp);
                    }
                });
            }
            Expression::Subscript(base, index) => {
                pp.node("Subscript", &sp, |pp| {
                    base.pretty_print(pp);
                    index.pretty_print(pp);
                });
            }
            Expression::Call { func, params } => {
                pp.node("Call", &sp, |pp| {
                    func.pretty_print(pp);
                    for p in params {
                        p.pretty_print(pp);
                    }
                });
            }
            Expression::Update {
                expr,
                op,
                is_postfix,
            } => {
                pp.node("Update", &sp, |pp| {
                    expr.pretty_print(pp);
                    op.pretty_print(pp);
                    pp.leaf_data("IsPostfix", "", if *is_postfix { "true" } else { "false" });
                });
            }
            Expression::AssignWithOp { lhs, op, rhs } => {
                pp.node("AssignWithOp", &sp, |pp| {
                    lhs.pretty_print(pp);
                    op.pretty_print(pp);
                    rhs.pretty_print(pp);
                });
            }
            Expression::Cast { type_name, expr } => {
                pp.node("Cast", &sp, |pp| {
                    type_name.pretty_print(pp);
                    expr.pretty_print(pp);
                });
            }
            Expression::SizeofType(ty) => {
                pp.node("SizeofType", &sp, |pp| ty.pretty_print(pp));
            }
            Expression::Sizeof(expr) => {
                pp.node("Sizeof", &sp, |pp| expr.pretty_print(pp));
            }
            Expression::AlignofType(ty) => {
                pp.node("AlignofType", &sp, |pp| ty.pretty_print(pp));
            }
            Expression::Alignof(expr) => {
                pp.node("Alignof", &sp, |pp| expr.pretty_print(pp));
            }
            Expression::Offsetof {
                ty,
                field,
                field_span,
            } => {
                pp.node("Offsetof", &sp, |pp| {
                    ty.pretty_print(pp);
                    pp.leaf_data("Field", &fmt_span(field_span, pp.config), field);
                });
            }
            Expression::UnaryOp(op, expr) => {
                pp.node("UnaryOp", &sp, |pp| {
                    op.pretty_print(pp);
                    expr.pretty_print(pp);
                });
            }
            Expression::BinOp(lhs, op, rhs) => {
                pp.node("BinOp", &sp, |pp| {
                    lhs.pretty_print(pp);
                    op.pretty_print(pp);
                    rhs.pretty_print(pp);
                });
            }
            Expression::Conditional {
                cond,
                then_expr,
                else_expr,
            } => {
                pp.node("Conditional", &sp, |pp| {
                    cond.pretty_print(pp);
                    then_expr.pretty_print(pp);
                    else_expr.pretty_print(pp);
                });
            }
            Expression::CompoundLiteral {
                type_name,
                initializer,
            } => {
                pp.node("CompoundLiteral", &sp, |pp| {
                    type_name.pretty_print(pp);
                    initializer.pretty_print(pp);
                });
            }
            Expression::GnuStatementExpr { body } => {
                pp.node("GnuStatementExpr", &sp, |pp| body.pretty_print(pp));
            }
            Expression::VaStart { args, last_param } => {
                pp.node("VaStart", &sp, |pp| {
                    args.pretty_print(pp);
                    pp.leaf_data(
                        "LastParam",
                        &fmt_span(&last_param.1, pp.config),
                        &last_param.0,
                    );
                });
            }
            Expression::VaArg { args, type_name } => {
                pp.node("VaArg", &sp, |pp| {
                    args.pretty_print(pp);
                    type_name.pretty_print(pp);
                });
            }
            Expression::VaCopy { dest, src } => {
                pp.node("VaCopy", &sp, |pp| {
                    dest.pretty_print(pp);
                    src.pretty_print(pp);
                });
            }
            Expression::VaEnd { args } => {
                pp.node("VaEnd", &sp, |pp| args.pretty_print(pp));
            }
            Expression::GenericSelection {
                controlling,
                associations,
            } => {
                pp.node("GenericSelection", &sp, |pp| {
                    controlling.pretty_print(pp);
                    for a in associations {
                        a.pretty_print(pp);
                    }
                });
            }
            Expression::BuiltinConstantP { expr } => {
                pp.node("BuiltinConstantP", &sp, |pp| expr.pretty_print(pp));
            }
            Expression::BuiltinTypesCompatibleP { ty1, ty2 } => {
                pp.node("BuiltinTypesCompatibleP", &sp, |pp| {
                    ty1.pretty_print(pp);
                    ty2.pretty_print(pp);
                });
            }
        }
    }
}

// ---------------------------------------------------------------------------
// GenericAssociation<R>
// ---------------------------------------------------------------------------

impl<R: TypeResolver> PrettyPrint for Spanned<GenericAssociation<R>> {
    fn pretty_print(&self, pp: &mut PrettyPrinter) {
        let sp = fmt_span(&self.1, pp.config);
        match &self.0 {
            GenericAssociation::Type { type_name, expr } => {
                pp.node("Type", &sp, |pp| {
                    type_name.pretty_print(pp);
                    expr.pretty_print(pp);
                });
            }
            GenericAssociation::Default { expr } => {
                pp.node("Default", &sp, |pp| expr.pretty_print(pp));
            }
        }
    }
}

// ---------------------------------------------------------------------------
// FunctionDefinitionSignature<R>
// ---------------------------------------------------------------------------

impl<R: TypeResolver> PrettyPrint for FunctionDefinitionSignature<R> {
    fn pretty_print(&self, pp: &mut PrettyPrinter) {
        match self {
            FunctionDefinitionSignature::C {
                declaration_specifiers,
                declarator,
            } => {
                pp.node("C", "", |pp| {
                    for spec in declaration_specifiers {
                        spec.pretty_print(pp);
                    }
                    declarator.pretty_print(pp);
                });
            }
            FunctionDefinitionSignature::Rust(sig) => {
                pp.node("Rust", "", |pp| sig.pretty_print(pp));
            }
        }
    }
}

// ---------------------------------------------------------------------------
// RustFunctionSignature<R>, RustFunctionParam<R>
// ---------------------------------------------------------------------------

impl<R: TypeResolver> PrettyPrint for RustFunctionSignature<R> {
    fn pretty_print(&self, pp: &mut PrettyPrinter) {
        pp.node("RustFunctionSignature", "", |pp| {
            for attr in &self.attrs {
                attr.pretty_print(pp);
            }
            pp.leaf_data(
                "Name",
                &fmt_span(&self.name.1, pp.config),
                format_args!("{:?}", &self.name.0),
            );
            for param in &self.params {
                param.pretty_print(pp);
            }
            pp.node("ReturnType", &fmt_span(&self.ret_ty.1, pp.config), |pp| {
                self.ret_ty.pretty_print(pp);
            });
            if self.visibility == Visibility::Public {
                pp.data("pub");
            }
        });
    }
}

impl<R: TypeResolver> PrettyPrint for RustFunctionParam<R> {
    fn pretty_print(&self, pp: &mut PrettyPrinter) {
        pp.node("Param", &fmt_span(&self.name.1, pp.config), |pp| {
            pp.leaf_data("Name", "", format_args!("{:?}", &self.name.0));
            self.ty.pretty_print(pp);
        });
    }
}

impl<R: TypeResolver> PrettyPrint for RustStructField<R> {
    fn pretty_print(&self, pp: &mut PrettyPrinter) {
        pp.node("Field", &fmt_span(&self.name.1, pp.config), |pp| {
            pp.leaf_data("Name", "", format_args!("{:?}", &self.name.0));
            pp.leaf_data("Visibility", "", format_args!("{:?}", self.visibility));
            self.ty.pretty_print(pp);
        });
    }
}

// ---------------------------------------------------------------------------
// Spanned<Declaration<R>>
// ---------------------------------------------------------------------------

impl<R: TypeResolver> PrettyPrint for Spanned<Declaration<R>> {
    fn pretty_print(&self, pp: &mut PrettyPrinter) {
        let sp = fmt_span(&self.1, pp.config);
        match &self.0 {
            Declaration::FunctionDefinition {
                attrs,
                signature,
                body,
            } => {
                pp.node("FunctionDefinition", &sp, |pp| {
                    for attr in attrs {
                        attr.pretty_print(pp);
                    }
                    signature.pretty_print(pp);
                    body.pretty_print(pp);
                });
            }
            Declaration::Declaration {
                attrs,
                declaration_specifiers,
                declarators,
            } => {
                pp.node("Declaration", &sp, |pp| {
                    for attr in attrs {
                        attr.pretty_print(pp);
                    }
                    for spec in declaration_specifiers {
                        spec.pretty_print(pp);
                    }
                    for decl in declarators {
                        decl.pretty_print(pp);
                    }
                });
            }
            Declaration::RustTypeAlias {
                attrs,
                ident,
                ty,
                visibility,
            } => {
                pp.node("RustTypeAlias", &sp, |pp| {
                    for attr in attrs {
                        attr.pretty_print(pp);
                    }
                    pp.leaf_data(
                        "Ident",
                        &fmt_span(&ident.1, pp.config),
                        format_args!("{:?}", &ident.0),
                    );
                    ty.pretty_print(pp);
                    if *visibility == Visibility::Public {
                        pp.data("pub");
                    }
                });
            }
            Declaration::RustStruct {
                attrs,
                ident,
                fields,
                visibility,
            } => {
                pp.node("RustStruct", &sp, |pp| {
                    for attr in attrs {
                        attr.pretty_print(pp);
                    }
                    pp.leaf_data(
                        "Ident",
                        &fmt_span(&ident.1, pp.config),
                        format_args!("{:?}", &ident.0),
                    );
                    for field in fields {
                        field.pretty_print(pp);
                    }
                    if *visibility == Visibility::Public {
                        pp.data("pub");
                    }
                });
            }
            Declaration::PragmaPack { action } => {
                pp.node("PragmaPack", &sp, |pp| action.pretty_print(pp));
            }
            Declaration::BreakCo2 => pp.leaf("BreakCo2", &sp),
        }
    }
}

// ---------------------------------------------------------------------------
// Spanned<Declarator<R>>
// ---------------------------------------------------------------------------

impl<R: TypeResolver> PrettyPrint for Spanned<Declarator<R>> {
    fn pretty_print(&self, pp: &mut PrettyPrinter) {
        let sp = fmt_span(&self.1, pp.config);
        match &self.0 {
            Declarator::Abstract => pp.leaf("Abstract", &sp),
            Declarator::Identifier(ident) => {
                pp.leaf_data("Identifier", &sp, format_args!("{:?}", &ident.0));
            }
            Declarator::FunctionDeclarator {
                declarator,
                param_list,
            } => {
                pp.node("FunctionDeclarator", &sp, |pp| {
                    declarator.pretty_print(pp);
                    param_list.pretty_print(pp);
                });
            }
            Declarator::PointerDeclarator {
                declarator,
                qualifiers,
            } => {
                pp.node("PointerDeclarator", &sp, |pp| {
                    for q in qualifiers {
                        q.pretty_print(pp);
                    }
                    declarator.pretty_print(pp);
                });
            }
            Declarator::ArrayDeclarator {
                declarator,
                subscription,
            } => {
                pp.node("ArrayDeclarator", &sp, |pp| {
                    let sp = fmt_span(&subscription.1, pp.config);
                    pp.leaf_data("Subscription", &sp, "opaque");
                    declarator.pretty_print(pp);
                });
            }
        }
    }
}

// ---------------------------------------------------------------------------
// ParameterList<R>
// ---------------------------------------------------------------------------

impl<R: TypeResolver> PrettyPrint for ParameterList<R> {
    fn pretty_print(&self, pp: &mut PrettyPrinter) {
        pp.node("ParameterList", "", |pp| {
            for (specs, decl) in &self.parameters {
                pp.node("Param", "", |pp| {
                    for s in specs {
                        s.pretty_print(pp);
                    }
                    decl.pretty_print(pp);
                });
            }
            if self.ellipsis {
                pp.data("...");
            }
            if self.empty_is_variadic {
                pp.data("empty_is_variadic");
            }
        });
    }
}

// ---------------------------------------------------------------------------
// Spanned<RustTy<R>>
// ---------------------------------------------------------------------------

impl<R: TypeResolver> PrettyPrint for Spanned<RustTy<R>> {
    fn pretty_print(&self, pp: &mut PrettyPrinter) {
        let sp = fmt_span(&self.1, pp.config);
        match &self.0 {
            RustTy::Path(path) => {
                pp.leaf_data("Path", &sp, format_args!("{:?}", &path.0));
            }
            RustTy::Tuple(elems) => {
                pp.node("Tuple", &sp, |pp| {
                    for e in elems {
                        e.pretty_print(pp);
                    }
                });
            }
            RustTy::Ref { mutable, inner } => {
                pp.node(if *mutable { "RefMut" } else { "Ref" }, &sp, |pp| {
                    inner.pretty_print(pp);
                });
            }
            RustTy::Ptr { mutable, inner } => {
                pp.node(if *mutable { "PtrMut" } else { "Ptr" }, &sp, |pp| {
                    inner.pretty_print(pp);
                });
            }
            RustTy::Slice(inner) => {
                pp.node("Slice", &sp, |pp| inner.pretty_print(pp));
            }
            RustTy::Array { inner, len } => {
                pp.node("Array", &sp, |pp| {
                    inner.pretty_print(pp);
                    len.pretty_print(pp);
                });
            }
            RustTy::BareFn { params, ret_ty } => {
                pp.node("BareFn", &sp, |pp| {
                    for p in params {
                        p.pretty_print(pp);
                    }
                    ret_ty.pretty_print(pp);
                });
            }
            RustTy::Never => pp.leaf("Never", &sp),
            RustTy::Wild => pp.leaf("Wild", &sp),
        }
    }
}

// ---------------------------------------------------------------------------
// LazyRustConstExpr, LazySubscription
// ---------------------------------------------------------------------------

impl PrettyPrint for Spanned<LazyRustConstExpr> {
    fn pretty_print(&self, pp: &mut PrettyPrinter) {
        let sp = fmt_span(&self.1, pp.config);
        pp.leaf_data(
            "LazyRustConstExpr",
            &sp,
            format_args!("{} tokens", self.0.tokens.len()),
        );
    }
}

impl PrettyPrint for Spanned<LazySubscription> {
    fn pretty_print(&self, pp: &mut PrettyPrinter) {
        let sp = fmt_span(&self.1, pp.config);
        pp.leaf_data(
            "LazySubscription",
            &sp,
            format_args!("{} tokens", self.0.tokens.len()),
        );
    }
}

// ---------------------------------------------------------------------------
// TypeName<R>
// ---------------------------------------------------------------------------

impl<R: TypeResolver> PrettyPrint for TypeName<R> {
    fn pretty_print(&self, pp: &mut PrettyPrinter) {
        pp.node("TypeName", "", |pp| {
            for sq in &self.specifier_qualifier_list {
                sq.pretty_print(pp);
            }
            if let Some(decl) = &self.abstract_declarator {
                decl.pretty_print(pp);
            }
        });
    }
}

// ---------------------------------------------------------------------------
// InitDeclarator<R>
// ---------------------------------------------------------------------------

impl<R: TypeResolver> PrettyPrint for Spanned<InitDeclarator<R>> {
    fn pretty_print(&self, pp: &mut PrettyPrinter) {
        let sp = fmt_span(&self.1, pp.config);
        pp.node("InitDeclarator", &sp, |pp| {
            self.0.declarator.pretty_print(pp);
            if let Some(init) = &self.0.initializer {
                init.pretty_print(pp);
            }
            if self.0.is_transparent_union {
                pp.data("transparent_union");
            }
        });
    }
}

// ---------------------------------------------------------------------------
// Spanned<Initializer<R>>
// ---------------------------------------------------------------------------

impl<R: TypeResolver> PrettyPrint for Spanned<Initializer<R>> {
    fn pretty_print(&self, pp: &mut PrettyPrinter) {
        let sp = fmt_span(&self.1, pp.config);
        match &self.0 {
            Initializer::Expr(expr) => {
                pp.node("Expr", &sp, |pp| expr.pretty_print(pp));
            }
            Initializer::List(items) => {
                pp.node("List", &sp, |pp| {
                    for item in items {
                        item.pretty_print(pp);
                    }
                });
            }
        }
    }
}

// ---------------------------------------------------------------------------
// Spanned<InitializerItem<R>>
// ---------------------------------------------------------------------------

impl<R: TypeResolver> PrettyPrint for Spanned<InitializerItem<R>> {
    fn pretty_print(&self, pp: &mut PrettyPrinter) {
        let sp = fmt_span(&self.1, pp.config);
        pp.node("InitializerItem", &sp, |pp| {
            if let Some(ds) = &self.0.designators {
                for d in ds {
                    d.pretty_print(pp);
                }
            }
            self.0.initializer.pretty_print(pp);
        });
    }
}

// ---------------------------------------------------------------------------
// Spanned<Designator<R>>
// ---------------------------------------------------------------------------

impl<R: TypeResolver> PrettyPrint for Spanned<Designator<R>> {
    fn pretty_print(&self, pp: &mut PrettyPrinter) {
        let sp = fmt_span(&self.1, pp.config);
        match &self.0 {
            Designator::Subscript(expr) => {
                pp.node("Subscript", &sp, |pp| expr.pretty_print(pp));
            }
            Designator::Range(lhs, rhs) => {
                pp.node("Range", &sp, |pp| {
                    lhs.pretty_print(pp);
                    rhs.pretty_print(pp);
                });
            }
            Designator::Field(name) => {
                pp.leaf_data("Field", &sp, &name.0);
            }
        }
    }
}

// ---------------------------------------------------------------------------
// Spanned<StructOrUnionField<R>>, StructDeclarator<R>
// ---------------------------------------------------------------------------

impl<R: TypeResolver> PrettyPrint for Spanned<StructOrUnionField<R>> {
    fn pretty_print(&self, pp: &mut PrettyPrinter) {
        let sp = fmt_span(&self.1, pp.config);
        pp.node("StructOrUnionField", &sp, |pp| {
            for sq in &self.0.specifiers {
                sq.pretty_print(pp);
            }
            for d in &self.0.declarators {
                d.pretty_print(pp);
            }
        });
    }
}

impl<R: TypeResolver> PrettyPrint for Spanned<StructDeclarator<R>> {
    fn pretty_print(&self, pp: &mut PrettyPrinter) {
        let sp = fmt_span(&self.1, pp.config);
        pp.node("StructDeclarator", &sp, |pp| {
            self.0.declarator.pretty_print(pp);
            if let Some(bits) = &self.0.bits {
                bits.pretty_print(pp);
            }
        });
    }
}

// ---------------------------------------------------------------------------
// Spanned<StructOrUnionSpecifier<R>>
// ---------------------------------------------------------------------------

impl<R: TypeResolver> PrettyPrint for Spanned<StructOrUnionSpecifier<R>> {
    fn pretty_print(&self, pp: &mut PrettyPrinter) {
        let sp = fmt_span(&self.1, pp.config);
        match &self.0 {
            StructOrUnionSpecifier::Defined { ident, fields } => {
                pp.node("Defined", &sp, |pp| {
                    pp.leaf_data("Ident", &fmt_span(&ident.1, pp.config), &ident.0);
                    for f in fields {
                        f.pretty_print(pp);
                    }
                });
            }
            StructOrUnionSpecifier::Declared { ident } => {
                pp.node("Declared", &sp, |pp| {
                    pp.leaf_data("Ident", &fmt_span(&ident.1, pp.config), &ident.0);
                });
            }
            StructOrUnionSpecifier::Anonymous { fields } => {
                pp.node("Anonymous", &sp, |pp| {
                    for f in fields {
                        f.pretty_print(pp);
                    }
                });
            }
        }
    }
}

// ---------------------------------------------------------------------------
// Spanned<EnumSpecifier<R>>
// ---------------------------------------------------------------------------

impl<R: TypeResolver> PrettyPrint for Spanned<EnumSpecifier<R>> {
    fn pretty_print(&self, pp: &mut PrettyPrinter) {
        let sp = fmt_span(&self.1, pp.config);
        match &self.0 {
            EnumSpecifier::Defined { ident, enumerators } => {
                pp.node("Defined", &sp, |pp| {
                    pp.leaf_data("Ident", &fmt_span(&ident.1, pp.config), &ident.0);
                    for e in enumerators {
                        let sp = fmt_span(&e.1, pp.config);
                        pp.leaf_data("Enumerator", &sp, format_args!("{:?}", &e.0));
                    }
                });
            }
            EnumSpecifier::Declared { ident } => {
                pp.node("Declared", &sp, |pp| {
                    pp.leaf_data("Ident", &fmt_span(&ident.1, pp.config), &ident.0);
                });
            }
            EnumSpecifier::Anonymous { enumerators } => {
                pp.node("Anonymous", &sp, |pp| {
                    for e in enumerators {
                        let sp = fmt_span(&e.1, pp.config);
                        pp.leaf_data("Enumerator", &sp, format_args!("{:?}", &e.0));
                    }
                });
            }
        }
    }
}

// ---------------------------------------------------------------------------
// Enumerator entries — handled inline in EnumSpecifier via Debug.
// ---------------------------------------------------------------------------

// ---------------------------------------------------------------------------
// TranslationUnit<R>
// ---------------------------------------------------------------------------

impl<R: TypeResolver> PrettyPrint for TranslationUnit<R> {
    fn pretty_print(&self, pp: &mut PrettyPrinter) {
        pp.node("TranslationUnit", "", |pp| {
            for attr in &self.attrs {
                attr.pretty_print(pp);
            }
            for item in &self.rust_use_items {
                item.pretty_print(pp);
            }
            for item in &self.rust_mod_items {
                item.pretty_print(pp);
            }
            for item in &self.items {
                item.pretty_print(pp);
            }
        });
    }
}

// ---------------------------------------------------------------------------
// UseItem, ModItem
// ---------------------------------------------------------------------------

impl PrettyPrint for Spanned<RustAttribute> {
    fn pretty_print(&self, pp: &mut PrettyPrinter) {
        let sp = fmt_span(&self.1, pp.config);
        pp.node("RustAttribute", &sp, |pp| {
            pp.leaf_data("Style", "", format_args!("{:?}", self.0.style));
            for segment in &self.0.path {
                pp.leaf_data("Segment", &fmt_span(&segment.1, pp.config), &segment.0);
            }
            if !self.0.args.is_empty() {
                pp.leaf_data("Args", "", format_args!("{} tokens", self.0.args.len()));
            }
        });
    }
}

impl PrettyPrint for Spanned<UseItem> {
    fn pretty_print(&self, pp: &mut PrettyPrinter) {
        let sp = fmt_span(&self.1, pp.config);
        pp.node("UseItem", &sp, |pp| {
            for attr in &self.0.attrs {
                attr.pretty_print(pp);
            }
            for seg in &self.0.path {
                pp.leaf_data("Segment", &fmt_span(&seg.1, pp.config), &seg.0);
            }
            if let Some(alias) = &self.0.alias {
                pp.leaf_data("Alias", &fmt_span(&alias.1, pp.config), &alias.0);
            }
        });
    }
}

impl PrettyPrint for Spanned<ModItem> {
    fn pretty_print(&self, pp: &mut PrettyPrinter) {
        let sp = fmt_span(&self.1, pp.config);
        pp.node("ModItem", &sp, |pp| {
            for attr in &self.0.attrs {
                attr.pretty_print(pp);
            }
            pp.leaf_data("Name", &fmt_span(&self.0.name.1, pp.config), &self.0.name.0);
            if let Some(ref content) = self.0.inline_content {
                pp.leaf_data(
                    "InlineContent",
                    &fmt_span(&content.1, pp.config),
                    format_args!("{} tokens", content.0.len()),
                );
            }
        });
    }
}
