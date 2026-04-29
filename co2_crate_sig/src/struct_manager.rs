use std::collections::HashMap;

use co2_ast::{
    DeclarationSpecifier, Declarator, EnumSpecifier, Enumerator, Expression, Spanned,
    StructOrUnionField, StructOrUnionKind, StructOrUnionSpecifier, TypeQualifier, TypeQueryResult,
};
use rustc_public_generative::{
    DefData, HirTy, StructField,
    rustc_public::{DefId, ty::Span},
};

use crate::{DefOrLocal, LocalResolver, LocalResolverBase, MirOwnerInfo, ty::CTy};

#[derive(Debug, Clone)]
pub(crate) struct StructData {
    pub(crate) def_id: DefId,
    pub(crate) name: String,
    pub(crate) kind: StructOrUnionKind,
    pub(crate) span: Span,
    pub(crate) fields: Option<Vec<StructField>>,
}

#[derive(Debug, Clone)]
pub(crate) struct PendingEnum {
    pub(crate) name: String,
    pub(crate) def_id: DefId,
    pub(crate) mir_info: MirOwnerInfo,
}

#[derive(Debug, Default)]
pub(crate) struct StructManager {
    pub(crate) definitions: HashMap<DefId, StructData>,
    pub(crate) pending_enum_consts: Vec<PendingEnum>,
}

const ANON_FIELD_PREFIX: &str = "__anon_field_";

fn has_const_qualifier_in_decl_specs(
    specs: &[Spanned<DeclarationSpecifier<LocalResolver>>],
) -> bool {
    specs.iter().any(|(spec, _)| {
        matches!(
            spec,
            DeclarationSpecifier::TypeQualifier((TypeQualifier::Const, _))
        )
    })
}

impl LocalResolver {
    fn def_id_of_named(
        &self,
        name: &str,
        kind: StructOrUnionKind,
        span: Span,
        redefine: bool,
    ) -> DefId {
        if let Some(def) = self.struct_tags.borrow().struct_tags.get(name) {
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
            .struct_tags
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

    pub(crate) fn collect_enumerator(
        &self,
        enumerator: Enumerator<LocalResolver>,
        _span: co2_ast::Span,
    ) -> (DefId, String, Option<Spanned<Expression<LocalResolver>>>) {
        let mut base = self.base.borrow_mut();
        let (def_id, fake_name) = base.emit_fake_def(rustc_public_generative::DefData::ValueNs);

        self.locals.borrow_mut().insert(
            enumerator.ident.0,
            (DefOrLocal::Const(def_id), TypeQueryResult::Expr),
        );
        (def_id, fake_name, enumerator.value)
    }

    pub(crate) fn collect_enum_constants(
        &self,
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
                let span = self.base.borrow().co2_span_to_rustc(span);
                let mut prev = None;
                for ((def_id, fake_name, value), _) in enumerators {
                    let mut base = self.base.borrow_mut();
                    let mir_info = match value {
                        Some((initializer, span)) => {
                            let initializer = (initializer, span);
                            MirOwnerInfo::EnumConstExplicit {
                                resolver: self.clone(),
                                initializer,
                            }
                        }
                        None => match prev {
                            Some(prev) => MirOwnerInfo::EnumConstPrevPlus(prev, span),
                            None => MirOwnerInfo::EnumConstZeroed,
                        },
                    };
                    base.struct_manager.pending_enum_consts.push(PendingEnum {
                        name: fake_name,
                        def_id,
                        mir_info,
                    });
                    prev = Some(def_id);
                }
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
        self.struct_manager.definitions.clone().into_values()
    }

    pub(crate) fn emit_enums(&mut self) -> impl Iterator<Item = PendingEnum> + use<> {
        self.struct_manager.pending_enum_consts.clone().into_iter()
    }

    pub(crate) fn adt_layout_info(
        &self,
        def: DefId,
    ) -> Option<(StructOrUnionKind, Vec<rustc_public_generative::HirTy>)> {
        let data = self.struct_manager.definitions.get(&def)?;
        let fields = data
            .fields
            .as_ref()?
            .iter()
            .map(|field| field.ty.clone())
            .collect();
        Some((data.kind, fields))
    }

    pub(crate) fn adt_field_ty(
        &self,
        def: DefId,
        field_name: &str,
    ) -> Option<rustc_public_generative::HirTy> {
        let data = self.struct_manager.definitions.get(&def)?;
        data.fields
            .as_ref()?
            .iter()
            .find(|field| field.name == field_name)
            .map(|field| field.ty.clone())
    }

    fn define_def(
        &mut self,
        def: DefId,
        fields: &[co2_ast::Spanned<StructOrUnionField<LocalResolver>>],
        _span: Span,
    ) {
        let struct_kind = self.struct_manager.definitions.get(&def).unwrap().kind;
        let data = self.struct_manager.definitions.get(&def).unwrap();
        if data.fields.is_some() {
            panic!("Redefinition happened");
        }
        let mut anon_field_count = 0;

        struct PendingField {
            name: String,
            ty: HirTy,
            span: co2_ast::Span,
            is_unsized: bool,
        }

        let mut pending_fields = Vec::new();

        for (field, span) in fields {
            let specifiers = field
                .specifiers
                .iter()
                .map(|f| {
                    let spec = match &f.0 {
                        co2_ast::SpecifierQualifier::TypeSpecifier(ts) => {
                            DeclarationSpecifier::TypeSpecifier(ts.clone())
                        }
                        co2_ast::SpecifierQualifier::TypeQualifier(tq) => {
                            DeclarationSpecifier::TypeQualifier(*tq)
                        }
                    };
                    (spec, f.1)
                })
                .collect::<Vec<_>>();
            let base_const = has_const_qualifier_in_decl_specs(&specifiers);
            let base = self.base_ty_of_decl(specifiers, *span);
            for (declarator, parser_span) in &field.declarators {
                let (name, ty, is_unsized) =
                    if matches!(declarator.declarator.0, Declarator::Abstract) {
                        let id = anon_field_count;
                        anon_field_count += 1;
                        let CTy::Ty(ty) = base.clone() else {
                            self.terminate_with_error(
                                *parser_span,
                                "Function is invalid for anon fields",
                            );
                        };
                        (format!("{ANON_FIELD_PREFIX}{id}"), ty, false)
                    } else {
                        self.lower_value_decl_type_maybe_unsized(
                            base.clone(),
                            base_const,
                            declarator.declarator.clone(),
                        )
                    };
                pending_fields.push(PendingField {
                    name,
                    ty,
                    span: *parser_span,
                    is_unsized,
                });
            }
        }

        let fields_len = pending_fields.len();
        let fields = pending_fields
            .into_iter()
            .enumerate()
            .map(|(i, pending)| {
                if pending.is_unsized {
                    let is_last = i == fields_len - 1;
                    if !is_last || matches!(struct_kind, StructOrUnionKind::Union) {
                        self.terminate_with_error(
                            pending.span,
                            "unsized array is not a first-class declaration type in this context",
                        );
                    }
                }

                let id = self
                    .hir_ctx
                    .allocate_def_id(def, DefData::ValueNs(pending.name.clone()));
                StructField {
                    id,
                    name: pending.name,
                    ty: pending.ty,
                    span: self.co2_span_to_rustc(pending.span),
                }
            })
            .collect();

        let data = self.struct_manager.definitions.get_mut(&def).unwrap();
        if data.fields.is_some() {
            todo!()
        }
        data.fields = Some(fields);
    }
}
