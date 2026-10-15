//! GNU extended asm lowering to stable MIR `InlineAsm`.
//! This file is completely AI generated, since I had neither time nor expertise to handle it.
//!
//! Stable MIR only exposes `template: String`, `operands`, `options: String`,
//! `line_spans: String`; the real `rustc_middle` values are reconstructed in
//! `rustc_public_generative::internal` from the encodings defined here:
//! - template: pieces joined by `\x1f`, `s<text>` for literals,
//!   `p<idx>:<mod|_>` for placeholders (`mod` is a Rust placeholder
//!   modifier: `l x e r`, `_` for none).
//! - options: decimal bits matching `rustc_ast::InlineAsmOptions`
//!   (`ATT_SYNTAX = 64`, `PRESERVES_FLAGS = 8`).
//! - operand `raw_rpr`: `I:<reg>` (in), `O:<late>:<reg>` (out),
//!   `N:<late>:<reg>` (inout), `C:<s|u><bytes>:<bits>` (const immediate),
//!   where `<reg>` is `r` (reg class), `q` (reg_byte class) or `=<name>`
//!   (fixed register, clobbers only). Const operands additionally carry the
//!   `Constant` in `in_value`, converted by the standard operand path.

use co2_hir::{HirAsm, HirExpr};
use rustc_public_generative::rustc_public::{
    mir::{
        InlineAsmOperand, Mutability, Operand as MirOperand, Place as MirPlace, TerminatorKind,
        UnwindAction,
    },
    ty::{IntTy, RigidTy, Span as RustSpan, Ty, TyKind, UintTy},
};

use crate::{build::Builder, place::place, rvalue::int_literal_bits};

/// Rust placeholder modifier for a GNU size letter on a multi-byte operand
/// (GNU `b w k q` denote the same widths as LLVM `b w k q`, which Rust
/// spells `l x e r`).
fn gnu_modifier_to_rust(gnu: char) -> Option<char> {
    match gnu {
        'b' => Some('l'),
        'w' => Some('x'),
        'k' => Some('e'),
        'q' => Some('r'),
        _ => None,
    }
}

/// Default Rust placeholder modifier for an operand byte size (x86-64).
/// 1-byte operands use `reg_byte`, which renders 8-bit with no modifier.
fn default_modifier(size: usize) -> Option<char> {
    match size {
        2 => Some('x'),
        4 => Some('e'),
        8 => Some('r'),
        _ => None,
    }
}

enum TemplatePiece {
    Str(String),
    Placeholder(usize, Option<char>),
}

fn encode_template(pieces: &[TemplatePiece]) -> String {
    let mut out = String::new();
    for (i, piece) in pieces.iter().enumerate() {
        if i > 0 {
            out.push('\x1f');
        }
        match piece {
            TemplatePiece::Str(text) => {
                out.push('s');
                out.push_str(text);
            }
            TemplatePiece::Placeholder(idx, modifier) => {
                out.push('p');
                out.push_str(&idx.to_string());
                out.push(':');
                out.push(modifier.unwrap_or('_'));
            }
        }
    }
    out
}

enum InputKind {
    Direct(String),
    Match(usize),
}

struct ClassifiedOperand {
    /// GNU operand index (outputs first, then inputs).
    gnu_index: usize,
    is_output: bool,
    is_inout: bool,
    late: bool,
    /// `true` for `"m"` (memory round-trip through a temp).
    is_mem: bool,
    is_const: bool,
    size: usize,
    /// For matching inputs (`"0"`): the output GNU index they copy into.
    matches: Option<usize>,
    expr: HirExpr,
}

impl Builder<'_, '_> {
    pub(crate) fn lower_asm_stmt(&mut self, asm: &HirAsm) {
        let span = asm.span;
        let mut ops: Vec<ClassifiedOperand> = Vec::new();
        let mut name_to_gnu: Vec<(String, usize)> = Vec::new();

        for op in &asm.outputs {
            let gnu_index = ops.len();
            let (constraint, is_inout, late) = self.parse_output_constraint(&op.constraint, span);
            if let Some(name) = &op.name {
                self.register_asm_name(&mut name_to_gnu, name, gnu_index, span);
            }
            let size = self.asm_operand_size(op.expr.ty, span);
            ops.push(ClassifiedOperand {
                gnu_index,
                is_output: true,
                is_inout,
                late,
                is_mem: constraint == "m",
                is_const: false,
                size,
                matches: None,
                expr: op.expr.clone(),
            });
        }
        for op in &asm.inputs {
            let gnu_index = ops.len();
            if let Some(name) = &op.name {
                self.register_asm_name(&mut name_to_gnu, name, gnu_index, span);
            }
            match self.parse_input_constraint(&op.constraint, &name_to_gnu, gnu_index, span) {
                InputKind::Direct(constraint) => {
                    let size = self.asm_operand_size(op.expr.ty, span);
                    if constraint == "i" {
                        self.check_const_bits(&op.expr, span);
                    }
                    ops.push(ClassifiedOperand {
                        gnu_index,
                        is_output: false,
                        is_inout: false,
                        late: false,
                        is_mem: constraint == "m",
                        is_const: constraint == "i",
                        size,
                        matches: None,
                        expr: op.expr.clone(),
                    });
                }
                InputKind::Match(out_gnu) => {
                    if out_gnu >= asm.outputs.len() {
                        self.terminate_with_error(
                            span,
                            "asm matching constraint can only refer to an output operand",
                        );
                    }
                    ops.push(ClassifiedOperand {
                        gnu_index,
                        is_output: false,
                        is_inout: false,
                        late: false,
                        is_mem: false,
                        is_const: false,
                        size: 0,
                        matches: Some(out_gnu),
                        expr: op.expr.clone(),
                    });
                }
            }
        }

        // Lower places/operands, build MIR operands in GNU order. Matching
        // inputs become pre-copies into the matched output (GCC semantics).
        let mut mir_operands: Vec<InlineAsmOperand> = Vec::new();
        // (mem output temp, destination place) for post-asm store-backs.
        let mut store_backs: Vec<(MirPlace, MirPlace)> = Vec::new();
        // GNU index -> MIR operand index (matching inputs alias outputs).
        let mut gnu_to_mir: Vec<usize> = vec![usize::MAX; ops.len()];
        // GNU index -> rendered as an immediate (gets a GCC-style `$` prefix).
        let mut gnu_is_imm: Vec<bool> = vec![false; ops.len()];
        // Output GNU index -> place for matching-input pre-copies.
        let mut out_places: Vec<MirPlace> = vec![place(usize::MAX); asm.outputs.len()];

        for op in &ops {
            if op.matches.is_some() {
                continue;
            }
            let mir_idx = mir_operands.len();
            gnu_to_mir[op.gnu_index] = mir_idx;
            if op.is_output {
                let dst = self.lower_expr_to_place(&op.expr).unwrap_or_else(|| {
                    self.terminate_with_error(
                        span,
                        "unsupported asm output lvalue (only variables, dereferences and fields are supported)",
                    )
                });
                if op.is_inout {
                    let in_value = self.asm_value_for_reg(&op.expr, span);
                    if op.is_mem {
                        let tmp = self.new_temp(op.expr.ty, Mutability::Mut, span);
                        self.emit_assign_use(place(tmp), in_value, span);
                        mir_operands.push(self.mk_inout(
                            op.late,
                            op.size,
                            MirOperand::Copy(place(tmp)),
                            place(tmp),
                        ));
                        store_backs.push((place(tmp), dst));
                        out_places[op.gnu_index] = place(tmp).clone();
                    } else {
                        out_places[op.gnu_index] = dst.clone();
                        mir_operands.push(self.mk_inout(op.late, op.size, in_value, dst));
                    }
                } else if op.is_mem {
                    let tmp = self.new_temp(op.expr.ty, Mutability::Mut, span);
                    mir_operands.push(self.mk_out(op.late, op.size, Some(place(tmp))));
                    store_backs.push((place(tmp), dst));
                    out_places[op.gnu_index] = place(tmp).clone();
                } else {
                    out_places[op.gnu_index] = dst.clone();
                    mir_operands.push(self.mk_out(op.late, op.size, Some(dst)));
                }
            } else if op.is_const {
                // `"i"` immediates stay immediates when the lowered operand
                // is already a constant; otherwise (e.g. signed types, which
                // lower through a cast) the value rides in a register, which
                // is observationally identical.
                match self.lower_expr_to_operand(&op.expr) {
                    MirOperand::Constant(c) => {
                        let (sign, bytes, bits) = self.asm_const_bits(&op.expr, span);
                        gnu_is_imm[op.gnu_index] = true;
                        mir_operands.push(InlineAsmOperand {
                            in_value: Some(MirOperand::Constant(c)),
                            out_place: None,
                            raw_rpr: format!("C:{sign}{bytes}:{bits}"),
                        });
                    }
                    in_value => {
                        let in_value = self.asm_widen_bool(in_value, op.expr.ty, span);
                        let size = op.size;
                        let ty = op.expr.ty;
                        let in_value = self.asm_spill_to_reg(in_value, ty, span);
                        mir_operands.push(self.mk_in(size, in_value));
                    }
                }
            } else {
                // `"m"` inputs ride in a register too: observationally
                // identical values, and every GPR remains addressable.
                let in_value = self.asm_value_for_reg(&op.expr, span);
                mir_operands.push(self.mk_in(op.size, in_value));
            }
        }

        for op in &ops {
            if let Some(out_gnu) = op.matches {
                gnu_to_mir[op.gnu_index] = gnu_to_mir[out_gnu];
                let in_value = self.lower_expr_to_operand(&op.expr);
                self.emit_assign_use(out_places[out_gnu].clone(), in_value, span);
            }
        }

        // Fixed-register clobbers become discarded late outputs.
        for clobber in &asm.clobbers {
            if clobber == "memory" || clobber.eq_ignore_ascii_case("cc") {
                continue;
            }
            if rustc_target::asm::X86InlineAsmReg::parse(&clobber.to_lowercase()).is_err() {
                self.terminate_with_error(
                    span,
                    &format!("unsupported asm clobber `{clobber}`"),
                );
            }
            mir_operands.push(InlineAsmOperand {
                in_value: None,
                out_place: None,
                raw_rpr: format!("O:1:={clobber}"),
            });
        }

        let template = self.rewrite_asm_template(
            &asm.template,
            &ops,
            &name_to_gnu,
            &gnu_to_mir,
            &gnu_is_imm,
            span,
        );
        // `ATT_SYNTAX` always (GNU templates are AT&T); `PRESERVES_FLAGS`
        // unless `cc` is clobbered.
        let mut options: u16 = 64;
        if !asm.clobbers.iter().any(|c| c.eq_ignore_ascii_case("cc")) {
            options |= 8;
        }

        let next = self.blocks.len() + 1;
        self.push_terminator(
            TerminatorKind::InlineAsm {
                template,
                operands: mir_operands,
                options: options.to_string(),
                line_spans: String::new(),
                destination: Some(next),
                unwind: UnwindAction::Continue,
            },
            span,
        );
        for (tmp, dst) in store_backs {
            self.emit_assign_use(dst, MirOperand::Copy(tmp), span);
        }
    }

    fn register_asm_name(
        &self,
        name_to_gnu: &mut Vec<(String, usize)>,
        name: &str,
        gnu_index: usize,
        span: RustSpan,
    ) {
        if name_to_gnu.iter().any(|(n, _)| n == name) {
            self.terminate_with_error(
                span,
                &format!("duplicate asm operand name `{name}`"),
            );
        }
        name_to_gnu.push((name.to_string(), gnu_index));
    }

    /// Split an output constraint into (class letters, is `+`, is `&`).
    fn parse_output_constraint(&self, constraint: &str, span: RustSpan) -> (String, bool, bool) {
        let mut c = constraint;
        let mut is_inout = false;
        if let Some(rest) = c.strip_prefix('+') {
            is_inout = true;
            c = rest;
        } else if let Some(rest) = c.strip_prefix('=') {
            c = rest;
        } else {
            self.terminate_with_error(
                span,
                &format!("asm output constraint must start with `=` or `+`, got `{constraint}`"),
            );
        }
        let mut late = false;
        if let Some(rest) = c.strip_prefix('&') {
            late = true;
            c = rest;
        }
        if c != "r" && c != "m" && c != "g" {
            self.terminate_with_error(
                span,
                &format!(
                    "unsupported asm output constraint `{constraint}` (only `r`, `m`, `g` are supported)"
                ),
            );
        }
        (c.to_string(), is_inout, late)
    }

    /// Split an input constraint: direct class or match (`"0"` / `"[name]"`).
    /// `operand_count` is the number of operands before this one (outputs +
    /// earlier inputs); matching digits must be below it.
    fn parse_input_constraint(
        &self,
        constraint: &str,
        name_to_gnu: &[(String, usize)],
        operand_count: usize,
        span: RustSpan,
    ) -> InputKind {
        if constraint.starts_with('=') || constraint.starts_with('+') {
            self.terminate_with_error(
                span,
                &format!(
                    "asm input constraint must not start with `=` or `+`, got `{constraint}`"
                ),
            );
        }
        if let Some(name) = constraint
            .strip_prefix('[')
            .and_then(|s| s.strip_suffix(']'))
        {
            let Some(&(_, gnu)) = name_to_gnu.iter().find(|(n, _)| n == name) else {
                self.terminate_with_error(
                    span,
                    &format!("asm matching constraint `[{name}]` refers to an unknown operand"),
                );
            };
            return InputKind::Match(gnu);
        }
        if !constraint.is_empty() && constraint.chars().all(|c| c.is_ascii_digit()) {
            let idx: usize = constraint.parse().unwrap_or(usize::MAX);
            if idx >= operand_count {
                self.terminate_with_error(
                    span,
                    &format!("asm matching constraint `{constraint}` is out of range"),
                );
            }
            return InputKind::Match(idx);
        }
        if constraint != "r" && constraint != "m" && constraint != "i" && constraint != "g" {
            self.terminate_with_error(
                span,
                &format!(
                    "unsupported asm input constraint `{constraint}` (only `r`, `m`, `i`, `g` are supported)"
                ),
            );
        }
        InputKind::Direct(constraint.to_string())
    }

    /// Byte size of an integer-like operand type; errors otherwise.
    fn asm_operand_size(&self, ty: Ty, span: RustSpan) -> usize {
        match ty.kind() {
            TyKind::RigidTy(RigidTy::Int(i)) => match i {
                IntTy::I8 => 1,
                IntTy::I16 => 2,
                IntTy::I32 => 4,
                IntTy::I64 => 8,
                _ => self.terminate_with_error(span, "unsupported asm operand integer type"),
            },
            TyKind::RigidTy(RigidTy::Uint(u)) => match u {
                UintTy::U8 => 1,
                UintTy::U16 => 2,
                UintTy::U32 => 4,
                UintTy::U64 => 8,
                _ => self.terminate_with_error(span, "unsupported asm operand integer type"),
            },
            TyKind::RigidTy(RigidTy::Bool) => 1,
            TyKind::RigidTy(RigidTy::RawPtr(_, _)) => 8,
            TyKind::RigidTy(RigidTy::Ref(_, _, _)) => 8,
            TyKind::RigidTy(RigidTy::FnDef(_, _) | RigidTy::FnPtr(_)) => 8,
            _ => self.terminate_with_error(
                span,
                &format!("unsupported asm operand type {}", self.format_ty(ty)),
            ),
        }
    }

    /// Reject `"i"` operands wider than 32 bits (matches the `I` operand
    /// encoding below, which carries at most 32 bits of payload).
    fn check_const_bits(&self, expr: &HirExpr, span: RustSpan) {
        self.asm_const_bits(expr, span);
    }

    /// Operand value for the `r` class: `_Bool` (Rust `bool`) widens to `u8`
    /// first since LLVM has no `i1` register; constants spill to a temp.
    fn asm_value_for_reg(&mut self, expr: &HirExpr, span: RustSpan) -> MirOperand {
        let value = self.lower_expr_to_operand(expr);
        let value = self.asm_widen_bool(value, expr.ty, span);
        self.asm_spill_to_reg(value, self.const_temp_ty(expr.ty), span)
    }

    /// Widen `_Bool` to `u8` (LLVM has no `i1` register class).
    fn asm_widen_bool(&mut self, value: MirOperand, ty: Ty, span: RustSpan) -> MirOperand {
        if matches!(ty.kind(), TyKind::RigidTy(RigidTy::Bool)) {
            self.lower_cast(value, ty, Ty::unsigned_ty(UintTy::U8), span)
        } else {
            value
        }
    }

    /// Spill a constant operand to a temp (LLVM `r` needs a register).
    fn asm_spill_to_reg(&mut self, value: MirOperand, ty: Ty, span: RustSpan) -> MirOperand {
        if matches!(value, MirOperand::Constant(_)) {
            let tmp = self.new_temp(ty, Mutability::Not, span);
            self.emit_assign_use(place(tmp), value, span);
            MirOperand::Copy(place(tmp))
        } else {
            value
        }
    }

    /// Temp type for a spilled constant: `bool` widens to `u8`.
    fn const_temp_ty(&self, ty: Ty) -> Ty {
        if matches!(ty.kind(), TyKind::RigidTy(RigidTy::Bool)) {
            Ty::unsigned_ty(UintTy::U8)
        } else {
            ty
        }
    }

    /// `(sign, bytes, bits)` for an `"i"` constant operand.
    fn asm_const_bits(&self, expr: &HirExpr, span: RustSpan) -> (char, usize, u128) {
        let value = match expr.kind {
            co2_hir::HirExprKind::ConstInt(v) => v,
            _ => self.terminate_with_error(span, "asm \"i\" operand must be an integer constant"),
        };
        match expr.ty.kind() {
            TyKind::RigidTy(RigidTy::Int(i)) => {
                let bytes = match i {
                    IntTy::I8 => 1,
                    IntTy::I16 => 2,
                    IntTy::I32 => 4,
                    _ => self.terminate_with_error(
                        span,
                        "asm `i` operand wider than 32 bits is not supported",
                    ),
                };
                let (_, bits) = int_literal_bits(value, expr.ty);
                ('s', bytes, bits)
            }
            TyKind::RigidTy(RigidTy::Uint(u)) => {
                let bytes = match u {
                    UintTy::U8 => 1,
                    UintTy::U16 => 2,
                    UintTy::U32 => 4,
                    _ => self.terminate_with_error(
                        span,
                        "asm `i` operand wider than 32 bits is not supported",
                    ),
                };
                let (_, bits) = int_literal_bits(value, expr.ty);
                ('u', bytes, bits)
            }
            TyKind::RigidTy(RigidTy::Bool) => ('u', 1, (value != 0) as u128),
            _ => self.terminate_with_error(span, "asm \"i\" operand must be an integer constant"),
        }
    }

    fn mk_in(&self, size: usize, in_value: MirOperand) -> InlineAsmOperand {
        InlineAsmOperand {
            in_value: Some(in_value),
            out_place: None,
            raw_rpr: format!("I:{}", self.asm_reg_spec(size)),
        }
    }

    fn mk_out(&self, late: bool, size: usize, out_place: Option<MirPlace>) -> InlineAsmOperand {
        InlineAsmOperand {
            in_value: None,
            out_place,
            raw_rpr: format!("O:{}:{}", late as u8, self.asm_reg_spec(size)),
        }
    }

    fn mk_inout(
        &self,
        late: bool,
        size: usize,
        in_value: MirOperand,
        out_place: MirPlace,
    ) -> InlineAsmOperand {
        InlineAsmOperand {
            in_value: Some(in_value),
            out_place: Some(out_place),
            raw_rpr: format!("N:{}:{}", late as u8, self.asm_reg_spec(size)),
        }
    }

    fn asm_reg_spec(&self, size: usize) -> &'static str {
        match size {
            1 => "q",
            _ => "r",
        }
    }

    /// Rewrite GNU `%` references to Rust `{idx:mod}` placeholders and
    /// encode the template. `gnu_to_mir` maps GNU operand indices (matching
    /// inputs alias their output) to MIR operand indices.
    fn rewrite_asm_template(
        &self,
        template: &str,
        ops: &[ClassifiedOperand],
        name_to_gnu: &[(String, usize)],
        gnu_to_mir: &[usize],
        gnu_is_imm: &[bool],
        span: RustSpan,
    ) -> String {
        let bytes = template.as_bytes();
        let mut i = 0;
        let mut current = String::new();
        let mut pieces: Vec<TemplatePiece> = Vec::new();
        while i < bytes.len() {
            if bytes[i] != b'%' {
                current.push(bytes[i] as char);
                i += 1;
                continue;
            }
            i += 1;
            if i >= bytes.len() {
                self.terminate_with_error(span, "stray `%` at end of asm template");
            }
            match bytes[i] {
                b'%' => {
                    current.push('%');
                    i += 1;
                }
                b'=' => {
                    self.terminate_with_error(
                        span,
                        "asm template `%=` (unique number) is not supported",
                    );
                }
                b'[' => {
                    let (name, next) = self.asm_bracket_name(template, bytes, i + 1, span);
                    i = next;
                    self.push_asm_placeholder(
                        &mut current,
                        &mut pieces,
                        ops,
                        gnu_to_mir,
                        gnu_is_imm,
                        self.lookup_asm_name(&name, name_to_gnu, span),
                        None,
                        span,
                    );
                }
                c if c.is_ascii_digit() => {
                    let start = i;
                    while i < bytes.len() && bytes[i].is_ascii_digit() {
                        i += 1;
                    }
                    let idx: usize = template[start..i].parse().unwrap_or(usize::MAX);
                    self.push_asm_placeholder(
                        &mut current,
                        &mut pieces,
                        ops,
                        gnu_to_mir,
                        gnu_is_imm,
                        idx,
                        None,
                        span,
                    );
                }
                c if c.is_ascii_alphabetic() => {
                    // GNU size modifier (`b/w/k/q`, …) + operand.
                    let gnu_mod = c as char;
                    i += 1;
                    let gnu = if i < bytes.len() && bytes[i] == b'[' {
                        let (name, next) = self.asm_bracket_name(template, bytes, i + 1, span);
                        i = next;
                        self.lookup_asm_name(&name, name_to_gnu, span)
                    } else {
                        let start = i;
                        while i < bytes.len() && bytes[i].is_ascii_digit() {
                            i += 1;
                        }
                        if start == i {
                            self.terminate_with_error(
                                span,
                                &format!(
                                    "unsupported asm template modifier `%{gnu_mod}`"
                                ),
                            );
                        }
                        template[start..i].parse().unwrap_or(usize::MAX)
                    };
                    self.push_asm_placeholder(
                        &mut current,
                        &mut pieces,
                        ops,
                        gnu_to_mir,
                        gnu_is_imm,
                        gnu,
                        Some(gnu_mod),
                        span,
                    );
                }
                other => {
                    self.terminate_with_error(
                        span,
                        &format!("unsupported asm template escape `%{}`", other as char),
                    );
                }
            }
        }
        if !current.is_empty() {
            pieces.push(TemplatePiece::Str(current));
        }
        encode_template(&pieces)
    }

    fn asm_bracket_name(
        &self,
        template: &str,
        bytes: &[u8],
        start: usize,
        span: RustSpan,
    ) -> (String, usize) {
        let mut end = start;
        while end < bytes.len() && bytes[end] != b']' {
            end += 1;
        }
        if end >= bytes.len() {
            self.terminate_with_error(span, "unterminated `%[name]` in asm template");
        }
        (template[start..end].to_string(), end + 1)
    }

    fn push_asm_placeholder(
        &self,
        current: &mut String,
        pieces: &mut Vec<TemplatePiece>,
        ops: &[ClassifiedOperand],
        gnu_to_mir: &[usize],
        gnu_is_imm: &[bool],
        gnu: usize,
        gnu_mod: Option<char>,
        span: RustSpan,
    ) {
        if gnu >= ops.len() {
            self.terminate_with_error(
                span,
                &format!("asm template refers to missing operand %{gnu}"),
            );
        }
        // Matching inputs alias their output: render the output.
        let target = ops[gnu].matches.unwrap_or(gnu);
        let mir = gnu_to_mir[gnu];
        if mir == usize::MAX {
            self.terminate_with_error(span, "internal asm error: unmapped operand");
        }
        let modifier = self.asm_placeholder_modifier(&ops[target], gnu_mod, span);
        if !current.is_empty() {
            pieces.push(TemplatePiece::Str(std::mem::take(current)));
        }
        // GCC prints immediates with a `$` prefix (`movl %1, %0` becomes
        // `movl $42, %eax`); Rust injects the bare number, so add it here.
        // Only for operands actually rendered as immediates (`"i"`
        // constants that survived as constants, not reg-spilled ones).
        if gnu_is_imm[gnu] {
            pieces.push(TemplatePiece::Str("$".to_string()));
        }
        pieces.push(TemplatePiece::Placeholder(mir, modifier));
    }

    fn lookup_asm_name(
        &self,
        name: &str,
        name_to_gnu: &[(String, usize)],
        span: RustSpan,
    ) -> usize {
        name_to_gnu
            .iter()
            .find(|(n, _)| n == name)
            .map(|(_, gnu)| *gnu)
            .unwrap_or_else(|| {
                self.terminate_with_error(
                    span,
                    &format!("asm template refers to unknown operand `%[{name}]`"),
                )
            })
    }

    /// Rust placeholder modifier for an operand reference with an optional
    /// GNU size letter. Never called for matching inputs (they resolve to
    /// their output first).
    fn asm_placeholder_modifier(
        &self,
        op: &ClassifiedOperand,
        gnu_mod: Option<char>,
        span: RustSpan,
    ) -> Option<char> {
        if op.matches.is_some() {
            self.terminate_with_error(span, "internal asm error: placeholder for matched input");
        }
        match gnu_mod {
            None => default_modifier(op.size),
            Some(g) => {
                let rust = gnu_modifier_to_rust(g).unwrap_or_else(|| {
                    self.terminate_with_error(
                        span,
                        &format!(
                            "unsupported asm operand modifier `%{g}` (only `b`, `w`, `k`, `q` are supported)"
                        ),
                    )
                });
                // Byte operands render 8-bit with no modifier; anything wider
                // on them is a type error.
                if op.size == 1 {
                    if rust != 'l' {
                        self.terminate_with_error(
                            span,
                            &format!("asm modifier `%{g}` does not match 1-byte operand"),
                        );
                    }
                    None
                } else {
                    Some(rust)
                }
            }
        }
    }
}
