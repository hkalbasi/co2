use rustc_data_structures::fx::{FxHashMap, FxHashSet};

use co2_ast::{
    DeclarationSpecifier, Declarator, EnumSpecifier, Enumerator, Expression, Spanned,
    StructOrUnionField, StructOrUnionKind, StructOrUnionSpecifier, TypeName, TypeQualifier,
    TypeQueryResult,
};
use rustc_public_generative::rustc_public::ty::{FloatTy, IntTy, UintTy};
use rustc_public_generative::{
    DefData, HirTy, HirTyConst, HirTyKind, StructField, Visibility,
    rustc_public::{DefId, ty::Span},
};

use crate::{DefOrLocal, LocalResolver, LocalResolverBase, MirOwnerInfo, ty::CTy};

#[derive(Debug, Clone)]
pub(crate) struct StructData {
    pub(crate) def_id: DefId,
    pub(crate) name: String,
    pub(crate) tag_name: Option<String>,
    pub(crate) kind: StructOrUnionKind,
    pub(crate) span: Span,
    pub(crate) emitted_fields: Option<Vec<StructField>>,
    pub(crate) logical_fields: Option<Vec<LogicalAdtFieldInfo>>,
    /// Effective `#pragma pack` alignment in bytes at the point this struct was defined,
    /// `None` means default (no packing).
    pub(crate) pack_align: Option<u32>,
    pub(crate) skip_emit: bool,
    /// True for enums declared with an explicit C23 fixed underlying type
    /// (`enum E : type { ... }`).
    pub(crate) fixed_underlying: bool,
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
pub struct CAdtDisplayInfo {
    pub kind: StructOrUnionKind,
    pub tag_name: Option<String>,
    pub anonymous_id: String,
    pub is_enum: bool,
}

#[derive(Debug, Clone)]
pub(crate) struct PendingEnum {
    pub(crate) name: String,
    pub(crate) def_id: DefId,
    pub(crate) mir_info: MirOwnerInfo,
}

#[derive(Debug, Default)]
pub(crate) struct StructManager {
    pub(crate) definitions: FxHashMap<DefId, StructData>,
    pub(crate) enum_defs: FxHashSet<DefId>,
    pub(crate) pending_enum_consts: Vec<PendingEnum>,
    /// Stack of pushed pack alignments (`None` = default).
    pub(crate) pack_stack: Vec<Option<u32>>,
    /// Current effective pack alignment (`None` = default).
    pub(crate) current_pack: Option<u32>,
}

const ANON_FIELD_PREFIX: &str = "__anon_field_";
pub(crate) const ENUM_FIELD_NAME: &str = "__co2_enum_value";

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
        if let Some(def) = self.struct_tags.borrow().struct_tags.get(name)
            && (!redefine
                || self.base.borrow().struct_manager.definitions[def]
                    .emitted_fields
                    .is_none())
        {
            return *def;
        }

        let def_id = self.base.borrow_mut().allocate_undef(kind, span, name);
        self.struct_tags
            .borrow_mut()
            .struct_tags
            .insert(name.to_owned(), def_id);
        def_id
    }

    fn def_id_of_enum(
        &self,
        name: &str,
        span: Span,
        _redefine: bool,
        underlying_ty: Option<HirTy>,
    ) -> DefId {
        if let Some(def) = self.struct_tags.borrow().struct_tags.get(name) {
            return *def;
        }

        let def_id = self
            .base
            .borrow_mut()
            .allocate_enum(span, name, underlying_ty);
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
    ) -> DefId {
        let rust_span = self.base.borrow().co2_span_to_rustc(span);
        let underlying_ty = |underlying_type: Option<TypeName<LocalResolver>>| {
            underlying_type.map(|ty| self.base.borrow_mut().lower_type_name_for_const(ty, span))
        };
        match specifier {
            EnumSpecifier::Declared {
                ident,
                underlying_type,
            } => self.def_id_of_enum(&ident.0, rust_span, false, underlying_ty(underlying_type)),
            EnumSpecifier::Defined {
                ident,
                underlying_type,
                enumerators,
            } => {
                let underlying_hir = underlying_ty(underlying_type);
                let def = self.def_id_of_enum(&ident.0, rust_span, true, underlying_hir.clone());
                let mut prev = None;
                let mut const_defs = Vec::new();
                let mut check_infos = Vec::new();
                for ((def_id, fake_name, value), espan) in enumerators {
                    let explicit = value.is_some();
                    let mut base = self.base.borrow_mut();
                    base.enum_const_defs.insert(def_id, def);
                    let mir_info = match value {
                        Some((initializer, span)) => {
                            let initializer = (initializer, span);
                            MirOwnerInfo::EnumConstExplicit {
                                resolver: self.clone(),
                                initializer,
                            }
                        }
                        None => match prev {
                            Some(prev) => MirOwnerInfo::EnumConstPrevPlus(prev, rust_span),
                            None => MirOwnerInfo::EnumConstZeroed,
                        },
                    };
                    base.struct_manager.pending_enum_consts.push(PendingEnum {
                        name: fake_name,
                        def_id,
                        mir_info,
                    });
                    const_defs.push(def_id);
                    check_infos.push((def_id, explicit, espan));
                    prev = Some(def_id);
                }
                if let Some(hir) = &underlying_hir {
                    self.check_fixed_enum_range(hir, &check_infos, span);
                } else {
                    self.base.borrow_mut().set_plain_enum_payload_ty_from_range(
                        def,
                        &const_defs,
                        span,
                    );
                }
                def
            }
            EnumSpecifier::Anonymous {
                underlying_type,
                enumerators,
            } => {
                let underlying_ty = underlying_ty(underlying_type);
                let def =
                    self.base
                        .borrow_mut()
                        .allocate_enum(rust_span, "", underlying_ty.clone());
                let mut prev = None;
                let mut const_defs = Vec::new();
                let mut check_infos = Vec::new();
                for ((def_id, fake_name, value), espan) in enumerators {
                    let explicit = value.is_some();
                    let mut base = self.base.borrow_mut();
                    base.enum_const_defs.insert(def_id, def);
                    let mir_info = match value {
                        Some((initializer, span)) => {
                            let initializer = (initializer, span);
                            MirOwnerInfo::EnumConstExplicit {
                                resolver: self.clone(),
                                initializer,
                            }
                        }
                        None => match prev {
                            Some(prev) => MirOwnerInfo::EnumConstPrevPlus(prev, rust_span),
                            None => MirOwnerInfo::EnumConstZeroed,
                        },
                    };
                    base.struct_manager.pending_enum_consts.push(PendingEnum {
                        name: fake_name,
                        def_id,
                        mir_info,
                    });
                    const_defs.push(def_id);
                    check_infos.push((def_id, explicit, espan));
                    prev = Some(def_id);
                }
                if let Some(hir) = &underlying_ty {
                    self.check_fixed_enum_range(hir, &check_infos, span);
                } else {
                    self.base.borrow_mut().set_plain_enum_payload_ty_from_range(
                        def,
                        &const_defs,
                        span,
                    );
                }
                def
            }
        }
    }

    /// For enums with a C23 fixed underlying type, every enumerator value must
    /// be representable in that type (C23 6.7.2.2p2). GCC rejects violations
    /// (`overflow in enumeration values` for implicit `prev + 1` overflow,
    /// `enumerator value outside the range of underlying type` for explicit
    /// initializers); previously CO2 silently wrapped via truncation.
    fn check_fixed_enum_range(
        &self,
        underlying: &HirTy,
        consts: &[(DefId, bool, co2_ast::Span)],
        scope_span: co2_ast::Span,
    ) {
        let Some((min, max)) = Self::int_range_of_hir_ty(underlying) else {
            return;
        };
        let mut errors = Vec::new();
        for (def_id, explicit, espan) in consts {
            let Ok(value) = self.base.borrow_mut().eval_local_const(*def_id, scope_span) else {
                continue;
            };
            if value < min || value > max {
                let msg = if *explicit {
                    "enumerator value outside the range of underlying type"
                } else {
                    "overflow in enumeration values"
                };
                errors.push(co2_ast::Rich::custom(*espan, msg.to_owned()));
            }
        }
        if !errors.is_empty() {
            co2_ast::emit_errors(errors);
        }
    }

    fn int_range_of_hir_ty(ty: &HirTy) -> Option<(i128, i128)> {
        match ty.kind {
            HirTyKind::Int(IntTy::I8) => Some((i8::MIN as i128, i8::MAX as i128)),
            HirTyKind::Int(IntTy::I16) => Some((i16::MIN as i128, i16::MAX as i128)),
            HirTyKind::Int(IntTy::I32) => Some((i32::MIN as i128, i32::MAX as i128)),
            HirTyKind::Int(IntTy::I64) => Some((i64::MIN as i128, i64::MAX as i128)),
            HirTyKind::Int(IntTy::I128) => Some((i128::MIN, i128::MAX)),
            HirTyKind::Int(IntTy::Isize) => Some((isize::MIN as i128, isize::MAX as i128)),
            HirTyKind::Uint(UintTy::U8) => Some((0, u8::MAX as i128)),
            HirTyKind::Uint(UintTy::U16) => Some((0, u16::MAX as i128)),
            HirTyKind::Uint(UintTy::U32) => Some((0, u32::MAX as i128)),
            HirTyKind::Uint(UintTy::U64) => Some((0, u64::MAX as i128)),
            HirTyKind::Uint(UintTy::U128) => Some((0, i128::MAX)),
            HirTyKind::Uint(UintTy::Usize) => Some((0, usize::MAX as i128)),
            HirTyKind::Bool => Some((0, 1)),
            _ => None,
        }
    }

    pub fn adt_logical_fields(&self, def: DefId) -> Option<Vec<LogicalAdtFieldInfo>> {
        self.base.borrow().adt_logical_fields(def)
    }
}

impl LocalResolverBase {
    fn allocate_enum(&mut self, span: Span, hint: &str, underlying_ty: Option<HirTy>) -> DefId {
        let name = format!(
            "__co2_c_enum_{hint}_{}",
            self.struct_manager.definitions.len()
        );
        let def_id = self.hir_ctx.allocate_def_id(
            self.hir_ctx.root_crate_def_id(),
            &DefData::TypeNs(name.clone()),
        );
        let field_id = self
            .hir_ctx
            .allocate_def_id(def_id, &DefData::ValueNs(ENUM_FIELD_NAME.to_owned()));
        let fixed_underlying = underlying_ty.is_some();
        let field_ty = underlying_ty.unwrap_or_else(|| HirTy::signed_ty(IntTy::I32, span));
        let data = StructData {
            def_id,
            name,
            tag_name: (!hint.is_empty()).then(|| hint.to_owned()),
            kind: StructOrUnionKind::Struct,
            span,
            emitted_fields: Some(vec![StructField {
                id: field_id,
                name: ENUM_FIELD_NAME.to_owned(),
                ty: field_ty,
                span,
                visibility: Visibility::Public,
            }]),
            logical_fields: None,
            pack_align: None,
            skip_emit: false,
            fixed_underlying,
        };
        self.struct_manager.definitions.insert(def_id, data);
        self.struct_manager.enum_defs.insert(def_id);
        def_id
    }

    fn allocate_undef(&mut self, kind: StructOrUnionKind, span: Span, hint: &str) -> DefId {
        let name = format!(
            "__co2_c_adt_{hint}_{}",
            self.struct_manager.definitions.len()
        );
        let def_id = self.hir_ctx.allocate_def_id(
            self.hir_ctx.root_crate_def_id(),
            &DefData::TypeNs(name.clone()),
        );
        let data = StructData {
            def_id,
            name,
            tag_name: (!hint.is_empty()).then(|| hint.to_owned()),
            kind,
            span,
            emitted_fields: None,
            logical_fields: None,
            pack_align: None,
            skip_emit: false,
            fixed_underlying: false,
        };
        self.struct_manager.definitions.insert(def_id, data);
        def_id
    }

    pub(crate) fn is_enum_def(&self, def_id: DefId) -> bool {
        self.struct_manager.enum_defs.contains(&def_id)
    }

    pub(crate) fn enum_payload_field_ty(&self, def_id: DefId) -> Option<HirTy> {
        let field = self
            .struct_manager
            .definitions
            .get(&def_id)?
            .emitted_fields
            .as_ref()?
            .first()?;
        Some(field.ty.clone())
    }

    fn set_enum_payload_field_ty(&mut self, def_id: DefId, ty: HirTy) {
        if let Some(data) = self.struct_manager.definitions.get_mut(&def_id)
            && let Some(fields) = data.emitted_fields.as_mut()
            && let Some(field) = fields.first_mut()
        {
            field.ty = ty;
        }
    }

    /// For plain enums (no fixed underlying type) the payload type must be able
    /// to represent all enumerator values. GCC picks the smallest of
    /// {int, unsigned int, long, unsigned long} that fits; we mirror that so
    /// large enumerator values are not truncated/sign-extended as i32.
    fn set_plain_enum_payload_ty_from_range(
        &mut self,
        enum_def: DefId,
        const_defs: &[DefId],
        span: co2_ast::Span,
    ) {
        let mut min = i128::MAX;
        let mut max = i128::MIN;
        for &const_def in const_defs {
            let Ok(value) = self.eval_local_const(const_def, span) else {
                return;
            };
            min = min.min(value);
            max = max.max(value);
        }
        let rust_span = self.co2_span_to_rustc(span);
        self.set_enum_payload_field_ty(
            enum_def,
            Self::plain_enum_payload_ty_from_range(min, max, rust_span),
        );
    }

    fn plain_enum_payload_ty_from_range(min: i128, max: i128, span: Span) -> HirTy {
        const INT_MIN: i128 = i32::MIN as i128;
        const INT_MAX: i128 = i32::MAX as i128;
        const UINT_MAX: i128 = u32::MAX as i128;
        const LONG_MIN: i128 = i64::MIN as i128;
        const LONG_MAX: i128 = i64::MAX as i128;
        const ULONG_MAX: i128 = u64::MAX as i128;
        // TODO: this function looks very wrong, but I didn't fix it since we don't have tests for it.
        if min >= 0 {
            if max <= INT_MAX {
                HirTy::signed_ty(IntTy::I32, span)
            } else if max <= UINT_MAX {
                HirTy::unsigned_ty(UintTy::U32, span)
            } else if max <= ULONG_MAX {
                HirTy::unsigned_ty(UintTy::U64, span)
            } else {
                HirTy::signed_ty(IntTy::I64, span)
            }
        } else if min >= INT_MIN && max <= INT_MAX {
            HirTy::signed_ty(IntTy::I32, span)
        } else if min >= LONG_MIN && max <= LONG_MAX {
            HirTy::signed_ty(IntTy::I64, span)
        } else {
            HirTy::unsigned_ty(UintTy::U64, span)
        }
    }

    pub(crate) fn is_fixed_underlying_enum(&self, def_id: DefId) -> bool {
        self.struct_manager
            .definitions
            .get(&def_id)
            .map_or(false, |data| data.fixed_underlying)
    }

    pub(crate) fn c_adt_display_info(&self, def_id: DefId) -> Option<CAdtDisplayInfo> {
        let data = self.struct_manager.definitions.get(&def_id)?;
        if data.skip_emit {
            return None;
        }
        let anonymous_id = data
            .name
            .rsplit_once('_')
            .map_or_else(|| data.name.clone(), |(_, suffix)| suffix.to_owned());
        Some(CAdtDisplayInfo {
            kind: data.kind,
            tag_name: data.tag_name.clone(),
            anonymous_id,
            is_enum: self.struct_manager.enum_defs.contains(&def_id),
        })
    }

    pub(crate) fn apply_pack_action(&mut self, action: &co2_ast::PackAction) {
        use co2_ast::PackAction;
        match action {
            PackAction::PushSet(n) => {
                let prev = self.struct_manager.current_pack;
                self.struct_manager.pack_stack.push(prev);
                self.struct_manager.current_pack = Some(*n);
            }
            PackAction::PushOnly => {
                let prev = self.struct_manager.current_pack;
                self.struct_manager.pack_stack.push(prev);
            }
            PackAction::Pop => {
                self.struct_manager.current_pack = self.struct_manager.pack_stack.pop().flatten();
            }
            PackAction::Set(n) => {
                self.struct_manager.current_pack = Some(*n);
            }
            PackAction::Reset => {
                self.struct_manager.current_pack = None;
            }
        }
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

    pub(crate) fn define_def(
        &mut self,
        def: DefId,
        fields: &[co2_ast::Spanned<StructOrUnionField<LocalResolver>>],
        _span: Span,
    ) {
        let struct_kind = self.struct_manager.definitions.get(&def).unwrap().kind;
        let data = self.struct_manager.definitions.get(&def).unwrap();
        assert!(data.emitted_fields.is_none(), "Redefinition happened");
        let mut anon_field_count = 0;
        let mut emitted_fields: Vec<StructField> = Vec::new();
        let mut logical_fields: Vec<LogicalAdtFieldInfo> = Vec::new();
        let mut open_bitfield_storage: Option<OpenBitfieldStorage> = None;
        let mut abs_bit = 0usize;
        let mut struct_max_align_bits = 8usize;
        let mut last_field_was_zero = false;
        let mut abs_before_last_zero = 0usize;
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
                    .map(|bits| -> Result<usize, (co2_ast::Span, String)> {
                        let value = self.eval_const_expr(bits)?;
                        usize::try_from(value).map_err(|_| {
                            (
                                bits.1,
                                format!(
                                    "bitfield width must be a non-negative integer, got {value}"
                                ),
                            )
                        })
                    })
                    .transpose()
                    .unwrap_or_else(|err| self.terminate_with_spanned_error(err));
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
                        if matches!(struct_kind, StructOrUnionKind::Union) {
                            abs_bit = 0;
                            last_field_was_zero = true;
                        } else {
                            // The zero-width field forces the next field to a
                            // fresh boundary aligned to this field's type.
                            abs_before_last_zero = abs_bit;
                            abs_bit = round_up_bit(abs_bit, storage_bits);
                            last_field_was_zero = true;
                        }
                        continue;
                    }

                    let (storage_index, bit_offset) =
                        if matches!(struct_kind, StructOrUnionKind::Union) {
                            // In a union all fields overlap at byte 0, so every bitfield starts
                            // at bit offset 0 and gets its own physical storage field (distinct
                            // field index in the Rust union, same underlying memory).
                            abs_bit = 0;
                            let storage_name =
                                format!("__co2_bitfield_storage_{}", emitted_fields.len());
                            let id = self
                                .hir_ctx
                                .allocate_def_id(def, &DefData::ValueNs(storage_name.clone()));
                            let index = emitted_fields.len();
                            emitted_fields.push(StructField {
                                id,
                                name: storage_name,
                                ty: storage_ty.clone(),
                                span: rust_span,
                                visibility: Visibility::Public,
                            });
                            (index, 0usize)
                        } else {
                            // GCC SysV: try to share current open unit if
                            // it has room, even crossing aligned boundary;
                            // otherwise advance to next aligned storage unit.
                            let candidate = if let Some(open) = &open_bitfield_storage {
                                let rel = abs_bit.saturating_sub(open.storage_start_bit);
                                if rel + bit_width <= open.storage_bits {
                                    abs_bit
                                } else if abs_bit / storage_bits
                                    != (abs_bit + bit_width - 1) / storage_bits
                                {
                                    round_up_bit(abs_bit, storage_bits)
                                } else {
                                    abs_bit
                                }
                            } else if abs_bit / storage_bits
                                != (abs_bit + bit_width - 1) / storage_bits
                            {
                                round_up_bit(abs_bit, storage_bits)
                            } else {
                                abs_bit
                            };
                            let (storage_index, bit_offset) = ensure_bitfield_storage(
                                &mut emitted_fields,
                                &mut open_bitfield_storage,
                                &storage_ty,
                                def,
                                self,
                                rust_span,
                                storage_bits,
                                bit_width,
                                candidate,
                                &mut logical_fields,
                            );
                            abs_bit = candidate + bit_width;
                            (storage_index, bit_offset)
                        };

                    if !name.is_empty() {
                        // Only named bitfields contribute to struct alignment (anonymous
                        // fields do not affect GCC's reported align). Use the
                        // declared type's alignment, not the (possibly reused)
                        // storage unit's current size.
                        struct_max_align_bits = struct_max_align_bits.max(storage_bits);
                        let actual_storage_ty = match &open_bitfield_storage {
                            Some(open) => open.storage_ty.clone(),
                            None => storage_ty.clone(),
                        };
                        logical_fields.push(LogicalAdtFieldInfo {
                            name,
                            ty,
                            kind: LogicalAdtFieldKind::Bitfield {
                                storage_index,
                                storage_ty: actual_storage_ty,
                                bit_offset,
                                bit_width,
                                is_signed,
                            },
                        });
                    }
                    last_field_was_zero = false;
                    continue;
                }

                if let Some(open) = open_bitfield_storage.take() {
                    // GCC SysV: a non-bit-field may be placed inside the
                    // tail padding of the last bit-field allocation unit.
                    // The next member's offset is `ceil(high_water, align)`,
                    // not `start + allocation_size`.  Shrink the Rust
                    // storage field to the smallest uint covering the used
                    // bits so Rust's `repr(C)` places the next field at the
                    // GCC-compatible offset (ponytail: zero-size align
                    // field for max alignment if strict ABI needed).
                    let used_bits = abs_bit.saturating_sub(open.storage_start_bit);
                    if used_bits < open.storage_bits {
                        // Anonymous storages can be byte-precise (no bitfield
                        // access needed), named storages use power-of-two
                        // integer types to keep MIR simple (ponytail: byte
                        // array for named 17-24 would need MIR array handling).
                        let is_anon = !logical_fields.iter().any(|lf| {
                            matches!(
                                &lf.kind,
                                LogicalAdtFieldKind::Bitfield { storage_index, .. }
                                if *storage_index == open.index && !lf.name.is_empty()
                            )
                        });
                        if is_anon {
                            let bytes = (used_bits + 7) / 8;
                            if bytes > 0 {
                                let inner = HirTy::unsigned_ty(UintTy::U8, open.storage_ty.span);
                                let new_ty = HirTy::new_array(
                                    inner,
                                    HirTyConst::Literal(bytes),
                                    open.storage_ty.span,
                                );
                                emitted_fields[open.index].ty = new_ty;
                            }
                        } else if let Some(needed) = smallest_uint_bits_covering(used_bits) {
                            if needed < open.storage_bits {
                                let new_ty = unsigned_ty_for_bits(needed, open.storage_ty.span);
                                emitted_fields[open.index].ty = new_ty.clone();
                                for lf in logical_fields.iter_mut() {
                                    if let LogicalAdtFieldKind::Bitfield {
                                        storage_index,
                                        storage_ty,
                                        ..
                                    } = &mut lf.kind
                                    {
                                        if *storage_index == open.index {
                                            *storage_ty = new_ty.clone();
                                        }
                                    }
                                }
                            }
                        }
                    }
                    // Keep `abs_bit` at high water (GCC) — do not extend to
                    // `start + storage_bits`.
                }
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
                    .allocate_def_id(def, &DefData::ValueNs(name.clone()));
                emitted_fields.push(StructField {
                    id,
                    name: name.clone(),
                    ty: ty.clone(),
                    span: rust_span,
                    visibility: Visibility::Public,
                });
                if matches!(struct_kind, StructOrUnionKind::Union) {
                    abs_bit = 0;
                } else if let Some((member_size, member_align)) = self.bit_layout_of_ty(&ty) {
                    struct_max_align_bits = struct_max_align_bits.max(member_align);
                    abs_bit = round_up_bit(abs_bit, member_align) + member_size;
                }
                logical_fields.push(LogicalAdtFieldInfo {
                    name,
                    ty,
                    kind: LogicalAdtFieldKind::Direct { physical_index },
                });
                last_field_was_zero = false;
            }
        }

        // Pad for trailing :0 (e.g. `int :2; long :0;` -> size 8 not 1)
        if last_field_was_zero && !matches!(struct_kind, StructOrUnionKind::Union) {
            let needed_bytes = (abs_bit + 7) / 8;
            let current_bytes = (abs_before_last_zero + 7) / 8;
            // current emitted size may be smaller than current_bytes due to shrinking,
            // use max to be safe
            let mut emitted_bytes = 0usize;
            for f in &emitted_fields {
                if let Some((sz, al)) = self.bit_layout_of_ty(&f.ty) {
                    emitted_bytes = round_up_bit(emitted_bytes * 8, al) / 8 + sz / 8;
                }
            }
            let cur = emitted_bytes.max(current_bytes);
            if needed_bytes > cur {
                let pad_bytes = needed_bytes - cur;
                let inner = HirTy::unsigned_ty(UintTy::U8, _span);
                let pad_ty = HirTy::new_array(inner, HirTyConst::Literal(pad_bytes), _span);
                let id = self
                    .hir_ctx
                    .allocate_def_id(def, &DefData::ValueNs("__co2_trailing_pad".to_owned()));
                emitted_fields.push(StructField {
                    id,
                    name: "__co2_trailing_pad".to_owned(),
                    ty: pad_ty,
                    span: _span,
                    visibility: Visibility::Public,
                });
            }
        }

        // Shrink any trailing bitfield storage to its high-water size
        // (e.g. `unsigned a:3` followed by `char` + `unsigned b:1` should
        // use 1-byte storages, not 4-byte, to match GCC's tight packing).
        if let Some(open) = open_bitfield_storage.take() {
            let used = abs_bit.saturating_sub(open.storage_start_bit);
            if used < open.storage_bits {
                if let Some(needed) = smallest_uint_bits_covering(used) {
                    if needed < open.storage_bits {
                        let new_ty = unsigned_ty_for_bits(needed, open.storage_ty.span);
                        emitted_fields[open.index].ty = new_ty.clone();
                        for lf in logical_fields.iter_mut() {
                            if let LogicalAdtFieldKind::Bitfield {
                                storage_index,
                                storage_ty,
                                ..
                            } = &mut lf.kind
                            {
                                if *storage_index == open.index {
                                    *storage_ty = new_ty.clone();
                                }
                            }
                        }
                    }
                }
            }
        }

        // Preserve overall struct alignment when the bitfield storage
        // was shrunk to its high-water size: Rust would otherwise lower
        // the struct's alignment (e.g. 1-bit `unsigned` + `char` should
        // still be align 4).  Add a zero-sized array with the required
        // alignment to keep GCC's ABI without adding size unless padding
        // is needed.
        if !matches!(struct_kind, StructOrUnionKind::Union) && !emitted_fields.is_empty() {
            let effective_max = if let Some(pack) = self.struct_manager.current_pack {
                let pack_bits = (pack as usize) * 8;
                struct_max_align_bits.min(pack_bits)
            } else {
                struct_max_align_bits
            };
            let mut emitted_max = 8usize;
            for f in &emitted_fields {
                if let Some((_, a)) = self.bit_layout_of_ty(&f.ty) {
                    emitted_max = emitted_max.max(a);
                }
            }
            if effective_max > emitted_max {
                let elem_ty = unsigned_ty_for_bits(effective_max, _span);
                let phantom_ty = HirTy::new_array(elem_ty, HirTyConst::Literal(0), _span);
                let id = self
                    .hir_ctx
                    .allocate_def_id(def, &DefData::ValueNs("__co2_align_phantom".to_owned()));
                emitted_fields.push(StructField {
                    id,
                    name: "__co2_align_phantom".to_owned(),
                    ty: phantom_ty,
                    span: _span,
                    visibility: Visibility::Public,
                });
            }
        }

        let data = self.struct_manager.definitions.get_mut(&def).unwrap();
        if data.emitted_fields.is_some() {
            todo!()
        }
        data.emitted_fields = Some(emitted_fields);
        data.logical_fields = Some(logical_fields);
        data.pack_align = self.struct_manager.current_pack;
    }

    /// Returns `(size_bits, align_bits)` for a non-bitfield member type, when
    /// it can be determined statically. Used to advance the absolute bit
    /// position past a normal member so that following bit-fields are placed
    /// at GCC-compatible positions.
    fn bit_layout_of_ty(&self, ty: &HirTy) -> Option<(usize, usize)> {
        let (size_bits, align_bits) = match &ty.kind {
            HirTyKind::Bool | HirTyKind::Char => (8, 8),
            HirTyKind::Int(IntTy::I8) | HirTyKind::Uint(UintTy::U8) => (8, 8),
            HirTyKind::Int(IntTy::I16) | HirTyKind::Uint(UintTy::U16) => (16, 16),
            HirTyKind::Int(IntTy::I32) | HirTyKind::Uint(UintTy::U32) => (32, 32),
            HirTyKind::Int(IntTy::I64)
            | HirTyKind::Uint(UintTy::U64)
            | HirTyKind::Int(IntTy::Isize)
            | HirTyKind::Uint(UintTy::Usize) => (64, 64),
            HirTyKind::Int(IntTy::I128) | HirTyKind::Uint(UintTy::U128) => (128, 128),
            HirTyKind::Float(FloatTy::F16) => (16, 16),
            HirTyKind::Float(FloatTy::F32) => (32, 32),
            HirTyKind::Float(FloatTy::F64) => (64, 64),
            HirTyKind::Float(FloatTy::F128) => (128, 128),
            HirTyKind::RawPtr(..) | HirTyKind::Ref(..) | HirTyKind::FnPtr(..) => (64, 64),
            HirTyKind::Array(len, inner) => {
                let HirTyConst::Literal(n) = len else {
                    return None;
                };
                let (elem_size, elem_align) = self.bit_layout_of_ty(inner)?;
                (elem_size * n, elem_align)
            }
            HirTyKind::Adt(def, _) => {
                if self.is_enum_def(*def) {
                    (32, 32)
                } else if let Some(underlying) = self.typedef_tys.get(def) {
                    return self.bit_layout_of_ty(underlying);
                } else {
                    let data = self.struct_manager.definitions.get(def)?;
                    let fields = data.emitted_fields.as_ref()?;
                    let mut size_bits = 0usize;
                    let mut align_bits = 8usize;
                    for f in fields {
                        let (field_size, field_align) = self.bit_layout_of_ty(&f.ty)?;
                        align_bits = align_bits.max(field_align);
                        size_bits = round_up_bit(size_bits, field_align) + field_size;
                    }
                    (round_up_bit(size_bits, align_bits), align_bits)
                }
            }
            _ => return None,
        };
        Some((size_bits, align_bits))
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
    storage_bits: usize,
    /// Absolute bit position (relative to the start of the record) where this
    /// storage unit begins. Always byte-aligned.
    storage_start_bit: usize,
    /// Index into `logical_fields` of the first field that lives in this
    /// storage, so we can fix up `storage_ty` if the storage is upgraded.
    first_logical: usize,
}

fn round_up_bit(x: usize, align: usize) -> usize {
    (x + align - 1) / align * align
}

fn smallest_uint_bits_covering(bits: usize) -> Option<usize> {
    if bits <= 8 {
        Some(8)
    } else if bits <= 16 {
        Some(16)
    } else if bits <= 32 {
        Some(32)
    } else if bits <= 64 {
        Some(64)
    } else if bits <= 128 {
        Some(128)
    } else {
        None
    }
}

fn unsigned_ty_for_bits(bits: usize, span: Span) -> HirTy {
    match bits {
        8 => HirTy::unsigned_ty(UintTy::U8, span),
        16 => HirTy::unsigned_ty(UintTy::U16, span),
        32 => HirTy::unsigned_ty(UintTy::U32, span),
        64 => HirTy::unsigned_ty(UintTy::U64, span),
        128 => HirTy::unsigned_ty(UintTy::U128, span),
        _ => unreachable!("unsupported storage width {bits}"),
    }
}

fn bitfield_storage_ty(resolver: &LocalResolverBase, ty: &HirTy) -> Option<(HirTy, bool, usize)> {
    let (kind, is_signed, bits) = match ty.kind {
        HirTyKind::Bool | HirTyKind::Char | HirTyKind::Uint(UintTy::U8) => {
            (HirTyKind::Uint(UintTy::U8), false, 8)
        }
        HirTyKind::Int(IntTy::I8) => (HirTyKind::Uint(UintTy::U8), true, 8),
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
            if resolver.is_enum_def(def) {
                return Some((HirTy::unsigned_ty(UintTy::U32, ty.span), false, 32));
            }
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

#[allow(clippy::too_many_arguments)]
fn ensure_bitfield_storage(
    emitted_fields: &mut Vec<StructField>,
    open_storage: &mut Option<OpenBitfieldStorage>,
    storage_ty: &HirTy,
    def: DefId,
    base: &mut LocalResolverBase,
    span: Span,
    storage_bits: usize,
    requested_bits: usize,
    candidate: usize,
    logical_fields: &mut Vec<LogicalAdtFieldInfo>,
) -> (usize, usize) {
    if let Some(open) = open_storage.as_ref() {
        let rel = candidate - open.storage_start_bit;
        let span_bits = rel + requested_bits;
        if span_bits <= open.storage_bits {
            // The field fits in the current storage unit.
            return (open.index, rel);
        }

        // The field needs more room than the current unit provides.
        if candidate % storage_bits == 0 {
            // A fresh allocation unit of the field's own type starts exactly
            // at the field's position; reproduce GCC by closing the previous
            // unit and opening a new one of the field's type.
            *open_storage = None;
        } else {
            // The field must stay inside the current storage unit; grow the
            // unit in place if it can be upgraded to a wider type while still
            // sitting at its original (byte-aligned) offset.
            if let Some(needed) = smallest_uint_bits_covering(span_bits) {
                if needed > open.storage_bits && (open.storage_start_bit / 8) % (needed / 8) == 0 {
                    let new_ty = unsigned_ty_for_bits(needed, storage_ty.span);
                    emitted_fields[open.index].ty = new_ty.clone();
                    for lf in logical_fields.iter_mut().skip(open.first_logical) {
                        if let LogicalAdtFieldKind::Bitfield {
                            storage_index,
                            storage_ty,
                            ..
                        } = &mut lf.kind
                        {
                            if *storage_index == open.index {
                                *storage_ty = new_ty.clone();
                            }
                        }
                    }
                    let open = open_storage.as_mut().expect("storage must exist");
                    open.storage_ty = new_ty;
                    open.storage_bits = needed;
                    return (open.index, rel);
                }
            }
            // Cannot grow in place; close and start a new unit at the
            // (byte-aligned) field position.
            *open_storage = None;
        }
    }

    let name = format!("__co2_bitfield_storage_{}", emitted_fields.len());
    let id = base
        .hir_ctx
        .allocate_def_id(def, &DefData::ValueNs(name.clone()));
    let index = emitted_fields.len();
    emitted_fields.push(StructField {
        id,
        name,
        ty: storage_ty.clone(),
        span,
        visibility: Visibility::Public,
    });
    *open_storage = Some(OpenBitfieldStorage {
        index,
        storage_ty: storage_ty.clone(),
        storage_bits,
        storage_start_bit: candidate,
        first_logical: logical_fields.len(),
    });
    (index, 0usize)
}
