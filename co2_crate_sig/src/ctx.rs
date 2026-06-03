use std::{cell::RefCell, collections::HashMap, rc::Rc, sync::Arc};

use rustc_public_generative::{
    DefData, FileId, HirModuleItem, HirStructureCtx, rustc_public::DefId,
};

use crate::{LocalResolverBase, MirOwnerInfo};

pub(crate) struct CrateSigCtx<'a> {
    pub(crate) hir_ctx: &'a HirStructureCtx<'a>,
    pub(crate) file_ids: Arc<HashMap<co2_ast::FileId, FileId>>,
    pub(crate) resolver: Rc<RefCell<LocalResolverBase>>,
    pub(crate) clone_trait: DefId,
    pub(crate) clone_trait_fn: DefId,
    pub(crate) copy_trait: DefId,
    pub(crate) mir_owners: HashMap<DefId, MirOwnerInfo>,
    pub(crate) hir_items: Vec<HirModuleItem>,
}

impl CrateSigCtx<'_> {
    pub(crate) fn terminate_with_spanned_error((span, msg): (co2_ast::Span, String)) -> ! {
        Self::terminate_with_error(span, &msg)
    }

    pub(crate) fn terminate_with_error(span: co2_ast::Span, msg: &str) -> ! {
        co2_ast::emit_errors_and_terminate(vec![co2_ast::Rich::custom(span, msg)]);
    }

    pub(crate) fn root_crate_def_id(&self) -> DefId {
        self.hir_ctx.root_crate_def_id()
    }

    pub(crate) fn allocate_def_id(&self, parent: DefId, data: &DefData) -> DefId {
        self.hir_ctx.allocate_def_id(parent, data)
    }

    pub(crate) fn resolve(&self, path: &str) -> Option<(DefId, co2_ast::TypeQueryResult)> {
        self.resolver.borrow().resolver.resolve(path)
    }

    pub(crate) fn resolve_in_current<'a>(
        &self,
        path: impl IntoIterator<Item = &'a str>,
    ) -> Option<(DefId, co2_ast::TypeQueryResult)> {
        self.resolver.borrow().resolver.resolve_in_current(path)
    }
}
