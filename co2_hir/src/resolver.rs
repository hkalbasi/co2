use co2_crate_sig::{LocalResolver, WellknownDefs};
use rustc_data_structures::fx::FxHashMap;
use rustc_public_generative::rustc_public::{
    CrateDefType, CrateItem, DefId,
    ty::{FnDef, GenericArgKind, GenericArgs, RigidTy, Span as RustSpan, Ty, TyKind},
};
use std::cell::RefCell;

use la_arena::Arena;

use crate::expr::HirExpr;
use crate::item::{HirLabel, LabelId, LocalId};

pub(crate) struct SwitchScope {
    pub(crate) discr_local: LocalId,
    pub(crate) discr_ty: Ty,
    pub(crate) case_labels: Vec<(HirExpr, LabelId)>,
    pub(crate) default_label: Option<LabelId>,
}

/// Runtime shape of a variable-length array: the innermost element type and
/// each dimension (in indexing order) as a local holding its value.
#[derive(Clone, Debug)]
pub(crate) struct VlaShape {
    pub(crate) elem_ty: Ty,
    pub(crate) dims: Vec<LocalId>,
}

/// VLA metadata for the body currently being lowered.
#[derive(Default)]
pub(crate) struct VlaState {
    /// Locals that are themselves VLA *objects* (`int a[n][m];`). The local's
    /// value is a `*mut elem` into the heap block.
    pub(crate) objects: FxHashMap<LocalId, VlaShape>,
    /// Locals that are *pointers to VLA* (`int (*p)[n];`). The local's value is
    /// a `*mut elem` pointing at the start of the pointed-to array.
    pub(crate) ptrs: FxHashMap<LocalId, VlaShape>,
    /// Every variably modified declaration seen so far, in source order, with
    /// the block nesting depth it was declared at. Used to reject `goto` jumps
    /// into the scope of a VLA.
    pub(crate) decls: Vec<(usize, co2_ast::Span)>,
}

/// One dimension of a VLA under construction. Dimensions are stored in
/// indexing order (outermost first).
#[derive(Clone)]
pub(crate) enum VlaDim {
    Const(u64),
    Expr(co2_ast::Spanned<co2_ast::Expression<LocalResolver>>),
    /// A dimension whose value was already evaluated into a local (used when
    /// expanding a VLA typedef whose bound was fixed at its declaration).
    Local(LocalId),
}

/// Accumulates a VLA shape while a declarator is being walked.
#[derive(Clone)]
pub(crate) struct VlaBuild {
    pub(crate) elem: Ty,
    pub(crate) dims: Vec<VlaDim>,
    /// Set when the accumulated array is wrapped in a pointer declarator
    /// (`int (*p)[n]`).
    pub(crate) is_ptr: bool,
}

#[derive(Clone, Debug)]
pub enum ResolvedValue {
    Fn(FnDef, Vec<GenericArgKind>),
    FnPtr(FnDef, Vec<GenericArgKind>),
    ConstInt(i128),
    Static(DefId),
    StaticConst(DefId),
}

impl ResolvedValue {
    pub(crate) fn ty(&self) -> Ty {
        match self {
            ResolvedValue::Fn(fn_def, generic_args) if generic_args.is_empty() => fn_def.ty(),
            ResolvedValue::Fn(fn_def, generic_args) => {
                Ty::from_rigid_kind(RigidTy::FnDef(*fn_def, GenericArgs(generic_args.clone())))
            }
            ResolvedValue::FnPtr(fn_def, generic_args) => {
                let fn_ty = if generic_args.is_empty() {
                    fn_def.ty()
                } else {
                    Ty::from_rigid_kind(RigidTy::FnDef(*fn_def, GenericArgs(generic_args.clone())))
                };
                let sig = fn_ty
                    .kind()
                    .fn_sig()
                    .expect("fn ptr must come from fn item");
                Ty::from_rigid_kind(RigidTy::FnPtr(sig))
            }
            ResolvedValue::ConstInt(_) => {
                Ty::signed_ty(rustc_public_generative::rustc_public::ty::IntTy::I32)
            }
            ResolvedValue::Static(def) | ResolvedValue::StaticConst(def) => CrateItem(*def).ty(),
        }
    }
}

type ParserSpan = co2_ast::Span;

pub(crate) fn spanned_error(
    span: co2_ast::Span,
    msg: impl Into<String>,
) -> (co2_ast::Span, String) {
    (span, msg.into())
}

pub(crate) fn invalid_span() -> co2_ast::Span {
    co2_ast::Span::from_parts(co2_ast::FileId::INVALID, 0..0)
}

pub struct HirCtx<'a> {
    pub(crate) wellknown_defs: WellknownDefs,
    span_converter: &'a dyn Fn(ParserSpan) -> RustSpan,
    rust_span_converter: &'a dyn Fn(RustSpan) -> ParserSpan,
    labels: RefCell<Arena<HirLabel>>,
    named_labels: RefCell<FxHashMap<String, LabelId>>,
    continue_labels: RefCell<Vec<LabelId>>,
    break_labels: RefCell<Vec<LabelId>>,
    switch_scopes: RefCell<Vec<SwitchScope>>,
    pub(crate) c_variadic_local: Option<LocalId>,
    /// Backing `Vec<Vec<u128>>` local, lazily created on the first `alloca`.
    pub(crate) alloca_storage: RefCell<Option<LocalId>>,
    /// VLA shapes for the current body.
    pub(crate) vla: RefCell<VlaState>,
    /// Scratch VLA shape being accumulated by the current declarator walk.
    pub(crate) pending_vla: RefCell<Option<VlaBuild>>,
    /// VLA typedefs (e.g. `typedef int row_t[n];`) visible in the current body,
    /// keyed by the C name. Bounds are fixed when the typedef is declared.
    pub(crate) vla_typedefs: RefCell<FxHashMap<String, VlaShape>>,
    /// Lexical block nesting depth while lowering the current body.
    pub(crate) block_depth: std::cell::Cell<usize>,
    /// Forward `goto`s seen so far in the current body that may jump into the
    /// scope of a VLA declared later: the label, the block depth at the goto,
    /// the number of VLA declarations before the goto, and the goto span.
    pub(crate) pending_gotos: RefCell<Vec<(LabelId, usize, usize, co2_ast::Span)>>,
    pub(crate) decl_resolver: LocalResolver,
    pub(crate) function_name: Option<String>,
    pub(crate) ret_ty: Ty,
}

impl<'a> HirCtx<'a> {
    pub fn new(
        wellknown_defs: WellknownDefs,
        span_converter: &'a dyn Fn(ParserSpan) -> RustSpan,
        rust_span_converter: &'a dyn Fn(RustSpan) -> ParserSpan,
        function_name: Option<String>,
        ret_ty: Ty,
        decl_resolver: LocalResolver,
    ) -> Self {
        Self {
            wellknown_defs,
            span_converter,
            rust_span_converter,
            labels: RefCell::new(Arena::new()),
            named_labels: RefCell::new(FxHashMap::default()),
            continue_labels: RefCell::new(Vec::new()),
            break_labels: RefCell::new(Vec::new()),
            switch_scopes: RefCell::new(Vec::new()),
            c_variadic_local: None,
            alloca_storage: RefCell::new(None),
            vla: RefCell::new(VlaState::default()),
            pending_vla: RefCell::new(None),
            vla_typedefs: RefCell::new(FxHashMap::default()),
            block_depth: std::cell::Cell::new(0),
            pending_gotos: RefCell::new(Vec::new()),
            decl_resolver,
            function_name,
            ret_ty,
        }
    }

    pub(crate) fn resolve_value_with_generic_args(
        &self,
        def_id: DefId,
        generic_args: &[co2_ast::Spanned<co2_ast::RustTy<co2_crate_sig::LocalResolver>>],
    ) -> ResolvedValue {
        let ty = CrateItem(def_id).ty();
        if matches!(ty.kind(), TyKind::RigidTy(RigidTy::FnDef(..))) {
            let fn_params = match ty.kind() {
                TyKind::RigidTy(RigidTy::FnDef(_, GenericArgs(params))) => params.clone(),
                _ => vec![],
            };
            let args: Vec<GenericArgKind> = generic_args
                .iter()
                .enumerate()
                .map(|(i, arg)| self.lower_generic_arg_with_wild(i, arg, &fn_params))
                .collect();
            ResolvedValue::Fn(FnDef(def_id), args)
        } else if self.decl_resolver.is_constexpr_def(def_id) {
            ResolvedValue::StaticConst(def_id)
        } else {
            ResolvedValue::Static(def_id)
        }
    }

    pub(crate) fn to_rust_span(&self, span: ParserSpan) -> RustSpan {
        (self.span_converter)(span)
    }

    pub(crate) fn to_parser_span(&self, span: RustSpan) -> ParserSpan {
        (self.rust_span_converter)(span)
    }

    pub(crate) fn terminate_with_error(&self, span: co2_ast::Span, msg: &str) -> ! {
        co2_ast::emit_errors_and_terminate(vec![co2_ast::Rich::custom(span, msg)]);
    }

    pub(crate) fn terminate_with_spanned_error(&self, (span, msg): (co2_ast::Span, String)) -> ! {
        self.terminate_with_error(span, &msg)
    }

    pub(crate) fn reset_labels(&self) {
        *self.labels.borrow_mut() = Arena::new();
        self.named_labels.borrow_mut().clear();
        self.continue_labels.borrow_mut().clear();
        self.break_labels.borrow_mut().clear();
        self.switch_scopes.borrow_mut().clear();
    }

    pub(crate) fn take_labels(&self) -> Arena<HirLabel> {
        std::mem::take(&mut *self.labels.borrow_mut())
    }

    pub(crate) fn fresh_label(&self) -> LabelId {
        self.labels.borrow_mut().alloc(HirLabel { name: None })
    }

    pub(crate) fn resolve_or_insert_label(&self, name: String) -> LabelId {
        if let Some(found) = self.named_labels.borrow().get(&name).copied() {
            return found;
        }
        let id = self.labels.borrow_mut().alloc(HirLabel {
            name: Some(name.clone()),
        });
        self.named_labels.borrow_mut().insert(name, id);
        id
    }

    pub(crate) fn enter_loop(&self, continue_label: LabelId, break_label: LabelId) {
        self.continue_labels.borrow_mut().push(continue_label);
        self.break_labels.borrow_mut().push(break_label);
    }

    pub(crate) fn exit_loop(&self) {
        self.continue_labels.borrow_mut().pop();
        self.break_labels.borrow_mut().pop();
    }

    pub(crate) fn enter_switch_scope(
        &self,
        discr_local: LocalId,
        discr_ty: Ty,
        break_label: LabelId,
    ) {
        self.switch_scopes.borrow_mut().push(SwitchScope {
            discr_local,
            discr_ty,
            case_labels: Vec::new(),
            default_label: None,
        });
        self.break_labels.borrow_mut().push(break_label);
    }

    pub(crate) fn exit_switch_scope(&self) -> SwitchScope {
        self.break_labels.borrow_mut().pop();
        self.switch_scopes
            .borrow_mut()
            .pop()
            .expect("exit_switch_scope called outside switch")
    }

    pub(crate) fn current_switch_discr(&self) -> Option<(LocalId, Ty)> {
        self.switch_scopes
            .borrow()
            .last()
            .map(|s| (s.discr_local, s.discr_ty))
    }

    pub(crate) fn in_switch(&self) -> bool {
        !self.switch_scopes.borrow().is_empty()
    }

    pub(crate) fn register_case(&self, cond: HirExpr, label: LabelId) {
        self.switch_scopes
            .borrow_mut()
            .last_mut()
            .expect("register_case called outside switch")
            .case_labels
            .push((cond, label));
    }

    pub(crate) fn register_default(&self, label: LabelId, span: co2_ast::Span) {
        let mut scopes = self.switch_scopes.borrow_mut();
        let scope = scopes
            .last_mut()
            .expect("register_default called outside switch");
        if scope.default_label.is_some() {
            self.terminate_with_error(span, "duplicate `default` label in switch");
        }
        scope.default_label = Some(label);
    }

    pub(crate) fn current_continue_label(&self) -> Option<LabelId> {
        self.continue_labels.borrow().last().copied()
    }

    pub(crate) fn current_break_label(&self) -> Option<LabelId> {
        self.break_labels.borrow().last().copied()
    }
}
