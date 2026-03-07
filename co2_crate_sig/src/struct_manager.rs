use std::collections::HashMap;

use co2_ast::{
    DeclarationSpecifier, Declarator, EnumSpecifier, StructOrUnionField, StructOrUnionKind,
    StructOrUnionSpecifier, TypeQueryResult,
};
use rustc_public_generative::{
    DefData, HirTy, StructField,
    rustc_public::{
        DefId,
        ty::{IntTy, Span},
    },
};

use crate::{CrateSigCtx, MirOwnerInfo, ty::TyOrFunction};

pub(crate) struct StructData {
    pub(crate) def_id: DefId,
    pub(crate) name: String,
    pub(crate) kind: StructOrUnionKind,
    pub(crate) span: Span,
    pub(crate) fields: Option<Vec<StructField>>,
}

#[derive(Default)]
pub(crate) struct StructManager {
    name_to_def: HashMap<String, DefId>,
    definitions: HashMap<DefId, StructData>,
}

const ANON_FIELD_PREFIX: &str = "__anon_field_";

impl CrateSigCtx<'_> {
    fn allocate_undef(&mut self, kind: StructOrUnionKind, span: Span, hint: &str) -> DefId {
        let name = format!(
            "__co2_c_adt_{hint}_{}",
            self.struct_manager.definitions.len()
        );
        let def_id = self.allocate_def_id(
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

    fn def_id_of_named(&mut self, name: &str, kind: StructOrUnionKind, span: Span) -> DefId {
        if let Some(def) = self.struct_manager.name_to_def.get(name) {
            return *def;
        }

        let def_id = self.allocate_undef(kind, span, name);
        self.struct_manager
            .name_to_def
            .insert(name.to_owned(), def_id);
        def_id
    }

    pub(crate) fn lower_struct_specifier(
        &mut self,
        kind: StructOrUnionKind,
        specifier: StructOrUnionSpecifier,
        span: Span,
    ) -> DefId {
        match specifier {
            StructOrUnionSpecifier::Defined { ident, fields } => {
                let def = self.def_id_of_named(&ident.0, kind, span);
                self.define_def(def, &fields, span);
                def
            }
            StructOrUnionSpecifier::Declared { ident } => {
                self.def_id_of_named(&ident.0, kind, span)
            }
            StructOrUnionSpecifier::Anonymous { fields } => {
                let def = self.allocate_undef(kind, span, "");
                self.define_def(def, &fields, span);
                def
            }
        }
    }

    pub(crate) fn emit_structs(&mut self) -> impl Iterator<Item = StructData> + use<> {
        let taken = std::mem::take(&mut self.struct_manager.definitions);
        taken.into_values()
    }

    fn define_def(
        &mut self,
        def: DefId,
        fields: &[(StructOrUnionField, co2_ast::Span)],
        _span: Span,
    ) {
        let data = self.struct_manager.definitions.get(&def).unwrap();
        if data.fields.is_some() {
            return;
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
                            let TyOrFunction::Ty(ty) = base.clone() else {
                                self.terminate_with_error(
                                    parser_span,
                                    "Function is invalid for anon fields",
                                );
                            };
                            (format!("{ANON_FIELD_PREFIX}{id}"), ty)
                        } else {
                            self.lower_value_decl_type(base.clone(), declarator.declarator)
                        };

                        let id = self.allocate_def_id(def, DefData::ValueNs(name.clone()));
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

    pub(crate) fn collect_enum_constants(&mut self, specifier: EnumSpecifier, _span: Span) {
        match specifier {
            EnumSpecifier::Declared { ident: _ } => (),
            EnumSpecifier::Defined {
                ident: _,
                enumerators,
            }
            | EnumSpecifier::Anonymous { enumerators } => {
                let mut prev = None;
                for (e, parser_span) in enumerators {
                    let span = self.co2_span_to_rustc(parser_span);
                    let name = &e.ident.0;
                    let def_id = self.allocate_def_id(
                        self.hir_ctx.root_crate_def_id(),
                        DefData::ValueNs(name.clone()),
                    );
                    self.resolver
                        .insert_into_current(name, Some((def_id, TypeQueryResult::Expr)));
                    self.hir_items
                        .push(rustc_public_generative::HirModuleItem::Static {
                            name: name.clone(),
                            id: def_id,
                            ty: HirTy::signed_ty(IntTy::I32, span),
                            mutable: false,
                            span,
                        });
                    let mir_info = match e.value {
                        Some(initializer) => MirOwnerInfo::EnumConstExplicit { initializer },
                        None => match prev {
                            Some(prev) => MirOwnerInfo::EnumConstPrevPlus(prev, span),
                            None => MirOwnerInfo::EnumConstZeroed,
                        },
                    };
                    self.mir_owners.insert(def_id, mir_info);
                    prev = Some(def_id);
                }
            }
        }
    }
}
