use co2_crate_sig::{LocalResolver, WellknownDefs};
use rustc_public_generative::rustc_public::{
    CrateDefType, CrateItem, DefId,
    ty::{FnDef, GenericArgKind, GenericArgs, RigidTy, Span as RustSpan, Ty, TyKind},
};
use std::cell::RefCell;
use std::collections::HashMap;

use la_arena::Arena;

use crate::expr::HirExpr;
use crate::item::{HirLabel, LabelId, LocalId};

pub(crate) struct SwitchScope {
    pub(crate) discr_local: LocalId,
    pub(crate) discr_ty: Ty,
    pub(crate) case_labels: Vec<(HirExpr, LabelId)>,
    pub(crate) default_label: Option<LabelId>,
}

#[derive(Clone, Debug)]
pub enum ResolvedValue {
    Fn(FnDef, Vec<GenericArgKind>),
    ConstInt(i128),
    Static(DefId),
}

impl ResolvedValue {
    pub(crate) fn ty(&self) -> Ty {
        match self {
            ResolvedValue::Fn(fn_def, generic_args) if generic_args.is_empty() => fn_def.ty(),
            ResolvedValue::Fn(fn_def, generic_args) => {
                Ty::from_rigid_kind(RigidTy::FnDef(*fn_def, GenericArgs(generic_args.clone())))
            }
            ResolvedValue::ConstInt(_) => {
                Ty::signed_ty(rustc_public_generative::rustc_public::ty::IntTy::I32)
            }
            ResolvedValue::Static(def) => CrateItem(*def).ty(),
        }
    }
}

type ParserSpan = co2_ast::Span;

pub struct HirCtx<'a> {
    pub(crate) wellknown_defs: WellknownDefs,
    span_converter: &'a dyn Fn(ParserSpan) -> RustSpan,
    labels: RefCell<Arena<HirLabel>>,
    named_labels: RefCell<HashMap<String, LabelId>>,
    continue_labels: RefCell<Vec<LabelId>>,
    break_labels: RefCell<Vec<LabelId>>,
    switch_scopes: RefCell<Vec<SwitchScope>>,
    pub(crate) c_variadic_local: Option<LocalId>,
    pub(crate) decl_resolver: Option<LocalResolver>,
    pub(crate) function_name: Option<String>,
    pub(crate) source_name: String,
    pub(crate) source: &'static str,
    pub(crate) ret_ty: Ty,
}

impl<'a> HirCtx<'a> {
    pub fn new(
        wellknown_defs: WellknownDefs,
        span_converter: &'a dyn Fn(ParserSpan) -> RustSpan,
        source: &'static str,
        source_name: String,
        function_name: Option<String>,
        ret_ty: Ty,
    ) -> Self {
        Self {
            wellknown_defs,
            span_converter,
            labels: RefCell::new(Arena::new()),
            named_labels: RefCell::new(HashMap::new()),
            continue_labels: RefCell::new(Vec::new()),
            break_labels: RefCell::new(Vec::new()),
            switch_scopes: RefCell::new(Vec::new()),
            c_variadic_local: None,
            decl_resolver: None,
            function_name,
            source,
            source_name,
            ret_ty,
        }
    }

    pub fn set_decl_resolver(&mut self, resolver: LocalResolver) {
        self.decl_resolver = Some(resolver);
    }

    pub(crate) fn resolve_value_with_generic_args(
        &self,
        def_id: DefId,
        generic_args: &[co2_ast::Spanned<co2_crate_sig::DefOrLocal>],
    ) -> ResolvedValue {
        let ty = CrateItem(def_id).ty();
        if matches!(ty.kind(), TyKind::RigidTy(RigidTy::FnDef(..))) {
            ResolvedValue::Fn(FnDef(def_id), self.lower_generic_args(generic_args))
        } else {
            ResolvedValue::Static(def_id)
        }
    }

    pub(crate) fn to_rust_span(&self, span: ParserSpan) -> RustSpan {
        (self.span_converter)(span)
    }

    pub(crate) fn terminate_with_error(&self, span: co2_ast::Span, msg: &str) -> ! {
        co2_ast::print_errors_and_terminate(
            self.source_name.clone(),
            self.source,
            vec![co2_ast::Rich::custom(span, msg)],
        );
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

    pub(crate) fn register_default(&self, label: LabelId) -> Result<(), String> {
        let mut scopes = self.switch_scopes.borrow_mut();
        let scope = scopes
            .last_mut()
            .expect("register_default called outside switch");
        if scope.default_label.is_some() {
            return Err("duplicate `default` label in switch".to_owned());
        }
        scope.default_label = Some(label);
        Ok(())
    }

    pub(crate) fn current_continue_label(&self) -> Option<LabelId> {
        self.continue_labels.borrow().last().copied()
    }

    pub(crate) fn current_break_label(&self) -> Option<LabelId> {
        self.break_labels.borrow().last().copied()
    }
}
