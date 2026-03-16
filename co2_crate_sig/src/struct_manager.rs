use std::collections::HashMap;

use co2_ast::{
    DeclarationSpecifier, Declarator, EnumSpecifier, StructOrUnionField, StructOrUnionKind,
    StructOrUnionSpecifier,
};
use rustc_public_generative::{
    DefData, StructField,
    rustc_public::{DefId, ty::Span},
};

use crate::{LocalResolver, LocalResolverBase, MirOwnerInfo, ty::CTy};

pub(crate) struct StructData {
    pub(crate) def_id: DefId,
    pub(crate) name: String,
    pub(crate) kind: StructOrUnionKind,
    pub(crate) span: Span,
    pub(crate) fields: Option<Vec<StructField>>,
}

pub(crate) struct PendingEnum {
    pub(crate) name: String,
    pub(crate) def_id: DefId,
    pub(crate) mir_info: MirOwnerInfo,
}

#[derive(Default)]
pub(crate) struct StructManager {
    definitions: HashMap<DefId, StructData>,
    pending_enum_consts: Vec<PendingEnum>,
}

const ANON_FIELD_PREFIX: &str = "__anon_field_";

impl LocalResolver {
    fn def_id_of_named(
        &self,
        name: &str,
        kind: StructOrUnionKind,
        span: Span,
        redefine: bool,
    ) -> DefId {
        if let Some(def) = self.struct_tags.borrow().get(name) {
            if !redefine
                || self.base.borrow().struct_manager.definitions[def]
                    .fields
                    .is_none()
            {
                return *def;
            }
        }

        let def_id = self.base.borrow_mut().allocate_undef(kind, span, name);
        self.struct_tags
            .borrow_mut()
            .insert(name.to_owned(), def_id);
        def_id
    }

    pub(crate) fn lower_struct_specifier(
        &self,
        kind: StructOrUnionKind,
        specifier: StructOrUnionSpecifier<LocalResolver>,
        parser_span: co2_ast::Span,
    ) -> DefId {
        let span = self.base.borrow_mut().co2_span_to_rustc(parser_span);
        match specifier {
            StructOrUnionSpecifier::Defined { ident, fields } => {
                let def = self.def_id_of_named(&ident.0, kind, span, true);
                self.base.borrow_mut().define_def(def, &fields, span);
                def
            }
            StructOrUnionSpecifier::Declared { ident } => {
                self.def_id_of_named(&ident.0, kind, span, false)
            }
            StructOrUnionSpecifier::Anonymous { fields } => {
                let mut base = self.base.borrow_mut();
                let def = base.allocate_undef(kind, span, "");
                base.define_def(def, &fields, span);
                def
            }
        }
    }
}

impl LocalResolverBase {
    fn allocate_undef(&mut self, kind: StructOrUnionKind, span: Span, hint: &str) -> DefId {
        let name = format!(
            "__co2_c_adt_{hint}_{}",
            self.struct_manager.definitions.len()
        );
        let def_id = self.hir_ctx.allocate_def_id(
            self.hir_ctx.root_crate_def_id(),
            DefData::TypeNs(name.clone()),
        );
        let data = StructData {
            def_id,
            name,
            kind,
            span,
            fields: None,
        };
        self.struct_manager.definitions.insert(def_id, data);
        def_id
    }

    pub(crate) fn emit_structs(&mut self) -> impl Iterator<Item = StructData> + use<> {
        let taken = std::mem::take(&mut self.struct_manager.definitions);
        taken.into_values()
    }

    pub(crate) fn emit_enums(&mut self) -> impl Iterator<Item = PendingEnum> + use<> {
        let taken = std::mem::take(&mut self.struct_manager.pending_enum_consts);
        taken.into_iter()
    }

    fn define_def(
        &mut self,
        def: DefId,
        fields: &[co2_ast::Spanned<StructOrUnionField<LocalResolver>>],
        _span: Span,
    ) {
        let data = self.struct_manager.definitions.get(&def).unwrap();
        if data.fields.is_some() {
            panic!("Redefinition happened");
        }
        let mut anon_field_count = 0;
        let fields = fields
            .iter()
            .flat_map(|(field, span)| {
                let specifiers = field
                    .specifiers
                    .iter()
                    .map(|f| (DeclarationSpecifier::TypeSpecifier(f.clone()), f.1))
                    .collect::<Vec<_>>();
                let base = self.base_ty_of_decl(specifiers, *span);
                field
                    .declarators
                    .iter()
                    .cloned()
                    .map(|(declarator, parser_span)| {
                        let span = self.co2_span_to_rustc(parser_span);

                        let (name, ty) = if matches!(declarator.declarator.0, Declarator::Abstract)
                        {
                            let id = anon_field_count;
                            anon_field_count += 1;
                            let CTy::Ty(ty) = base.clone() else {
                                self.terminate_with_error(
                                    parser_span,
                                    "Function is invalid for anon fields",
                                );
                            };
                            (format!("{ANON_FIELD_PREFIX}{id}"), ty)
                        } else {
                            self.lower_value_decl_type(base.clone(), declarator.declarator)
                        };

                        let id = self
                            .hir_ctx
                            .allocate_def_id(def, DefData::ValueNs(name.clone()));
                        StructField { id, name, ty, span }
                    })
                    .collect::<Vec<_>>()
                    .into_iter()
            })
            .collect();
        let data = self.struct_manager.definitions.get_mut(&def).unwrap();
        if data.fields.is_some() {
            todo!()
        }
        data.fields = Some(fields);
    }

    pub(crate) fn collect_enum_constants(
        &mut self,
        specifier: EnumSpecifier<LocalResolver>,
        span: co2_ast::Span,
    ) {
        match specifier {
            EnumSpecifier::Declared { ident: _ } => (),
            EnumSpecifier::Defined {
                ident: _,
                enumerators,
            }
            | EnumSpecifier::Anonymous { enumerators } => {
                let span = self.co2_span_to_rustc(span);
                let mut prev = None;
                for (e, _) in enumerators {
                    let def_id = self.resolver.resolve_in_current([&*e.ident.0]).unwrap().0;
                    let mir_info = match e.value {
                        Some((initializer, span)) => {
                            let initializer = (initializer, span);
                            MirOwnerInfo::EnumConstExplicit { initializer }
                        }
                        None => match prev {
                            Some(prev) => MirOwnerInfo::EnumConstPrevPlus(prev, span),
                            None => MirOwnerInfo::EnumConstZeroed,
                        },
                    };
                    self.struct_manager.pending_enum_consts.push(PendingEnum {
                        name: e.ident.0.clone(),
                        def_id,
                        mir_info,
                    });
                    prev = Some(def_id);
                }
            }
        }
    }
}
