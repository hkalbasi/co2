use std::collections::HashMap;

use co2_ast::{
    DeclarationSpecifier, Declarator, EnumSpecifier, Enumerator, Expression, Spanned,
    StructOrUnionField, StructOrUnionKind, StructOrUnionSpecifier, TypeQualifier, TypeQueryResult,
};
use rustc_public_generative::rustc_public::ty::{IntTy, UintTy};
use rustc_public_generative::{
    DefData, HirTy, HirTyKind, StructField,
    rustc_public::{DefId, ty::Span},
};

use crate::{DefOrLocal, LocalResolver, LocalResolverBase, MirOwnerInfo, ty::CTy};

#[derive(Debug, Clone)]
pub(crate) struct StructData {
    pub(crate) def_id: DefId,
    pub(crate) name: String,
    pub(crate) kind: StructOrUnionKind,
    pub(crate) span: Span,
    pub(crate) emitted_fields: Option<Vec<StructField>>,
    pub(crate) logical_fields: Option<Vec<LogicalAdtFieldInfo>>,
}

#[derive(Debug, Clone)]
pub struct LogicalAdtFieldInfo {
    pub name: String,
    pub ty: HirTy,
    pub kind: LogicalAdtFieldKind,
}

#[derive(Debug, Clone)]
pub enum LogicalAdtFieldKind {
    Direct {
        physical_index: usize,
    },
    Bitfield {
        storage_index: usize,
        storage_ty: HirTy,
        bit_offset: usize,
        bit_width: usize,
        is_signed: bool,
    },
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
                    .emitted_fields
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

    pub fn adt_logical_fields(&self, def: DefId) -> Option<Vec<LogicalAdtFieldInfo>> {
        self.base.borrow().adt_logical_fields(def)
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
            emitted_fields: None,
            logical_fields: None,
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
            .emitted_fields
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
        self.resolve_logical_field_ty(def, field_name)
    }

    pub(crate) fn adt_logical_fields(&self, def: DefId) -> Option<Vec<LogicalAdtFieldInfo>> {
        self.struct_manager
            .definitions
            .get(&def)?
            .logical_fields
            .clone()
    }

    fn define_def(
        &mut self,
        def: DefId,
        fields: &[co2_ast::Spanned<StructOrUnionField<LocalResolver>>],
        _span: Span,
    ) {
        let struct_kind = self.struct_manager.definitions.get(&def).unwrap().kind;
        let data = self.struct_manager.definitions.get(&def).unwrap();
        if data.emitted_fields.is_some() {
            panic!("Redefinition happened");
        }
        let mut anon_field_count = 0;
        let mut emitted_fields = Vec::new();
        let mut logical_fields = Vec::new();
        let mut open_bitfield_storage: Option<OpenBitfieldStorage> = None;
        let total_declarators = fields
            .iter()
            .map(|(field, _)| field.declarators.len())
            .sum::<usize>();
        let mut seen_declarators = 0usize;

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
                seen_declarators += 1;
                let rust_span = self.co2_span_to_rustc(*parser_span);
                let width = declarator
                    .bits
                    .as_ref()
                    .map(|bits| parse_bitfield_width(bits, *parser_span))
                    .transpose()
                    .unwrap_or_else(|msg| self.terminate_with_error(*parser_span, &msg));
                let is_abstract = matches!(declarator.declarator.0, Declarator::Abstract);
                let (name, ty, is_unsized) = if is_abstract {
                    let CTy::Ty(ty) = base.clone() else {
                        self.terminate_with_error(
                            *parser_span,
                            "Function is invalid for anonymous fields",
                        );
                    };
                    let name = if width.is_some() {
                        String::new()
                    } else {
                        let id = anon_field_count;
                        anon_field_count += 1;
                        format!("{ANON_FIELD_PREFIX}{id}")
                    };
                    (name, ty, false)
                } else {
                    self.lower_value_decl_type_maybe_unsized(
                        base.clone(),
                        base_const,
                        declarator.declarator.clone(),
                    )
                };

                if let Some(bit_width) = width {
                    if matches!(struct_kind, StructOrUnionKind::Union) {
                        self.terminate_with_error(
                            *parser_span,
                            "bitfields in unions are not supported yet",
                        );
                    }
                    if is_unsized {
                        self.terminate_with_error(*parser_span, "bitfield type must be sized");
                    }
                    let Some((storage_ty, is_signed, storage_bits)) =
                        bitfield_storage_ty(self, &ty)
                    else {
                        self.terminate_with_error(
                            *parser_span,
                            "bitfield type must be an integer or boolean type",
                        );
                    };
                    if bit_width > storage_bits {
                        self.terminate_with_error(
                            *parser_span,
                            &format!(
                                "bitfield width {bit_width} exceeds storage width {storage_bits}"
                            ),
                        );
                    }
                    if bit_width == 0 {
                        if !name.is_empty() {
                            self.terminate_with_error(
                                *parser_span,
                                "named zero-width bitfields are invalid",
                            );
                        }
                        open_bitfield_storage = None;
                        continue;
                    }

                    let storage_index = ensure_bitfield_storage(
                        &mut emitted_fields,
                        &mut open_bitfield_storage,
                        &storage_ty,
                        def,
                        self,
                        rust_span,
                        storage_bits,
                        bit_width,
                    );
                    let bit_offset = open_bitfield_storage
                        .as_ref()
                        .expect("bitfield storage must be open")
                        .bits_used
                        - bit_width;
                    if !name.is_empty() {
                        logical_fields.push(LogicalAdtFieldInfo {
                            name,
                            ty,
                            kind: LogicalAdtFieldKind::Bitfield {
                                storage_index,
                                storage_ty,
                                bit_offset,
                                bit_width,
                                is_signed,
                            },
                        });
                    }
                    continue;
                }

                open_bitfield_storage = None;
                if is_unsized {
                    let is_last = seen_declarators == total_declarators;
                    if !is_last || matches!(struct_kind, StructOrUnionKind::Union) {
                        self.terminate_with_error(
                            *parser_span,
                            "unsized array is not a first-class declaration type in this context",
                        );
                    }
                }

                let physical_index = emitted_fields.len();
                let id = self
                    .hir_ctx
                    .allocate_def_id(def, DefData::ValueNs(name.clone()));
                emitted_fields.push(StructField {
                    id,
                    name: name.clone(),
                    ty: ty.clone(),
                    span: rust_span,
                });
                logical_fields.push(LogicalAdtFieldInfo {
                    name,
                    ty,
                    kind: LogicalAdtFieldKind::Direct { physical_index },
                });
            }
        }

        let data = self.struct_manager.definitions.get_mut(&def).unwrap();
        if data.emitted_fields.is_some() {
            todo!()
        }
        data.emitted_fields = Some(emitted_fields);
        data.logical_fields = Some(logical_fields);
    }

    fn resolve_logical_field_ty(&self, def: DefId, field_name: &str) -> Option<HirTy> {
        let data = self.struct_manager.definitions.get(&def)?;
        let logical_fields = data.logical_fields.as_ref()?;
        for field in logical_fields {
            if field.name == field_name && !field.name.starts_with(ANON_FIELD_PREFIX) {
                return Some(field.ty.clone());
            }
        }
        for field in logical_fields {
            if !field.name.starts_with(ANON_FIELD_PREFIX) {
                continue;
            }
            let HirTyKind::Adt(nested_def, _) = field.ty.kind else {
                continue;
            };
            if let Some(found) = self.resolve_logical_field_ty(nested_def, field_name) {
                return Some(found);
            }
        }
        None
    }
}

#[derive(Debug, Clone)]
struct OpenBitfieldStorage {
    index: usize,
    storage_ty: HirTy,
    bits_used: usize,
    storage_bits: usize,
}

fn parse_bitfield_width(bits: &Spanned<String>, span: co2_ast::Span) -> Result<usize, String> {
    bits.0
        .parse::<usize>()
        .map_err(|_| format!("invalid bitfield width `{}` at {:?}", bits.0, span))
}

fn bitfield_storage_ty(resolver: &LocalResolverBase, ty: &HirTy) -> Option<(HirTy, bool, usize)> {
    let (kind, is_signed, bits) = match ty.kind {
        HirTyKind::Bool => (HirTyKind::Uint(UintTy::U8), false, 8),
        HirTyKind::Char => (HirTyKind::Uint(UintTy::U8), false, 8),
        HirTyKind::Int(IntTy::I8) => (HirTyKind::Uint(UintTy::U8), true, 8),
        HirTyKind::Uint(UintTy::U8) => (HirTyKind::Uint(UintTy::U8), false, 8),
        HirTyKind::Int(IntTy::I16) => (HirTyKind::Uint(UintTy::U16), true, 16),
        HirTyKind::Uint(UintTy::U16) => (HirTyKind::Uint(UintTy::U16), false, 16),
        HirTyKind::Int(IntTy::I32) => (HirTyKind::Uint(UintTy::U32), true, 32),
        HirTyKind::Uint(UintTy::U32) => (HirTyKind::Uint(UintTy::U32), false, 32),
        HirTyKind::Int(IntTy::I64) => (HirTyKind::Uint(UintTy::U64), true, 64),
        HirTyKind::Uint(UintTy::U64) => (HirTyKind::Uint(UintTy::U64), false, 64),
        HirTyKind::Int(IntTy::I128) => (HirTyKind::Uint(UintTy::U128), true, 128),
        HirTyKind::Uint(UintTy::U128) => (HirTyKind::Uint(UintTy::U128), false, 128),
        HirTyKind::Int(IntTy::Isize) => (HirTyKind::Uint(UintTy::Usize), true, 64),
        HirTyKind::Uint(UintTy::Usize) => (HirTyKind::Uint(UintTy::Usize), false, 64),
        HirTyKind::Adt(def, _) => {
            let underlying = resolver.typedef_tys.get(&def)?;
            return bitfield_storage_ty(resolver, underlying);
        }
        _ => return None,
    };
    Some((
        HirTy {
            kind,
            span: ty.span,
        },
        is_signed,
        bits,
    ))
}

fn ensure_bitfield_storage(
    emitted_fields: &mut Vec<StructField>,
    open_storage: &mut Option<OpenBitfieldStorage>,
    storage_ty: &HirTy,
    def: DefId,
    base: &mut LocalResolverBase,
    span: Span,
    storage_bits: usize,
    requested_bits: usize,
) -> usize {
    let compatible = open_storage.as_ref().is_some_and(|open| {
        same_storage_kind(&open.storage_ty.kind, &storage_ty.kind)
            && open.bits_used + requested_bits <= open.storage_bits
    });
    if !compatible {
        let name = format!("__co2_bitfield_storage_{}", emitted_fields.len());
        let id = base
            .hir_ctx
            .allocate_def_id(def, DefData::ValueNs(name.clone()));
        let index = emitted_fields.len();
        emitted_fields.push(StructField {
            id,
            name,
            ty: storage_ty.clone(),
            span,
        });
        *open_storage = Some(OpenBitfieldStorage {
            index,
            storage_ty: storage_ty.clone(),
            bits_used: 0,
            storage_bits,
        });
    }
    let open = open_storage.as_mut().expect("bitfield storage must exist");
    let index = open.index;
    open.bits_used += requested_bits;
    index
}

fn same_storage_kind(lhs: &HirTyKind, rhs: &HirTyKind) -> bool {
    matches!(
        (lhs, rhs),
        (HirTyKind::Uint(UintTy::U8), HirTyKind::Uint(UintTy::U8))
            | (HirTyKind::Uint(UintTy::U16), HirTyKind::Uint(UintTy::U16))
            | (HirTyKind::Uint(UintTy::U32), HirTyKind::Uint(UintTy::U32))
            | (HirTyKind::Uint(UintTy::U64), HirTyKind::Uint(UintTy::U64))
            | (HirTyKind::Uint(UintTy::U128), HirTyKind::Uint(UintTy::U128))
            | (
                HirTyKind::Uint(UintTy::Usize),
                HirTyKind::Uint(UintTy::Usize)
            )
    )
}
