use std::collections::HashMap;

use rustc_public_generative::{
    DefData, FileId, FunctionSignature, HirModuleItem, HirStructureCtx, rustc_public::DefId,
};

use crate::{MirOwnerInfo, resolver::Resolver, struct_manager::StructManager};

pub(crate) struct CrateSigCtx<'a> {
    pub(crate) hir_ctx: HirStructureCtx<'a>,
    pub(crate) source_name: String,
    pub(crate) source: &'static str,
    pub(crate) file_id: FileId,
    pub(crate) resolver: Resolver,
    pub(crate) unrepresentable_typedefs: HashMap<String, FunctionSignature>,
    pub(crate) struct_manager: StructManager,
    pub(crate) mir_owners: HashMap<DefId, MirOwnerInfo>,
    pub(crate) hir_items: Vec<HirModuleItem>,
}

impl CrateSigCtx<'_> {
    pub(crate) fn terminate_with_error(&self, span: co2_ast::Span, msg: &str) -> ! {
        co2_ast::print_errors_and_terminate(
            self.source_name.clone(),
            self.source,
            vec![co2_ast::Rich::custom(span, msg)],
        );
    }

    pub(crate) fn root_crate_def_id(&self) -> DefId {
        self.hir_ctx.root_crate_def_id()
    }

    pub(crate) fn allocate_def_id(&self, parent: DefId, data: DefData) -> DefId {
        self.hir_ctx.allocate_def_id(parent, data)
    }
}
