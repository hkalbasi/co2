use rustc_public_generative::rustc_public::{
    CrateDefType, CrateItem, DefId,
    ty::{FnDef, Span as RustSpan, Ty},
};
use std::cell::RefCell;
use std::collections::HashMap;

use la_arena::Arena;

use crate::item::{HirLabel, LabelId};

#[derive(Clone, Debug)]
pub enum ResolvedValue {
    Fn(FnDef),
    ConstInt(i64),
    Static { def: DefId, ty: Option<Ty> },
}

impl ResolvedValue {
    pub(crate) fn ty(&self) -> Ty {
        match self {
            ResolvedValue::Fn(fn_def) => fn_def.ty(),
            ResolvedValue::ConstInt(_) => {
                Ty::signed_ty(rustc_public_generative::rustc_public::ty::IntTy::I32)
            }
            ResolvedValue::Static { def, ty } => ty.unwrap_or_else(|| CrateItem(*def).ty()),
        }
    }
}

type ParserSpan = co2_parser::Span;

pub struct HirCtx<'a, R> {
    resolver: &'a R,
    resolve_value: fn(&R, &str) -> Option<ResolvedValue>,
    resolve_type: fn(&R, &str) -> Option<Ty>,
    span_converter: &'a dyn Fn(ParserSpan) -> RustSpan,
    labels: RefCell<Arena<HirLabel>>,
    named_labels: RefCell<HashMap<String, LabelId>>,
    continue_labels: RefCell<Vec<LabelId>>,
    break_labels: RefCell<Vec<LabelId>>,
}

impl<'a, R> HirCtx<'a, R> {
    pub fn new(
        resolver: &'a R,
        resolve_value: fn(&R, &str) -> Option<ResolvedValue>,
        resolve_type: fn(&R, &str) -> Option<Ty>,
        span_converter: &'a dyn Fn(ParserSpan) -> RustSpan,
    ) -> Self {
        Self {
            resolver,
            resolve_value,
            resolve_type,
            span_converter,
            labels: RefCell::new(Arena::new()),
            named_labels: RefCell::new(HashMap::new()),
            continue_labels: RefCell::new(Vec::new()),
            break_labels: RefCell::new(Vec::new()),
        }
    }

    pub(crate) fn resolve_value(&self, path: &str) -> Option<ResolvedValue> {
        (self.resolve_value)(self.resolver, path)
    }

    pub(crate) fn resolve_type(&self, path: &str) -> Option<Ty> {
        (self.resolve_type)(self.resolver, path)
    }

    pub(crate) fn to_rust_span(&self, span: ParserSpan) -> RustSpan {
        (self.span_converter)(span)
    }

    pub(crate) fn reset_labels(&self) {
        *self.labels.borrow_mut() = Arena::new();
        self.named_labels.borrow_mut().clear();
        self.continue_labels.borrow_mut().clear();
        self.break_labels.borrow_mut().clear();
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

    pub(crate) fn enter_switch(&self, break_label: LabelId) {
        self.break_labels.borrow_mut().push(break_label);
    }

    pub(crate) fn exit_switch(&self) {
        self.break_labels.borrow_mut().pop();
    }

    pub(crate) fn current_continue_label(&self) -> Option<LabelId> {
        self.continue_labels.borrow().last().copied()
    }

    pub(crate) fn current_break_label(&self) -> Option<LabelId> {
        self.break_labels.borrow().last().copied()
    }
}
