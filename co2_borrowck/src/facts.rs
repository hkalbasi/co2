use std::collections::BTreeSet;

use polonius_engine::{AllFacts, Atom, FactTypes};
use rustc_public_generative::rustc_public::mir::{
    Body, Operand, Place, ProjectionElem, RETURN_LOCAL, Rvalue, Statement, StatementKind,
    Terminator, TerminatorKind,
};
use rustc_public_generative::rustc_public::ty::{RigidTy, TyKind};

macro_rules! atom_newtype {
    ($(#[$meta:meta])* $name:ident) => {
        $(#[$meta])*
        #[derive(Copy, Clone, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
        pub struct $name(usize);

        impl From<usize> for $name {
            fn from(value: usize) -> Self {
                Self(value)
            }
        }

        impl From<$name> for usize {
            fn from(value: $name) -> Self {
                value.0
            }
        }

        impl Atom for $name {
            fn index(self) -> usize {
                self.0
            }
        }
    };
}

atom_newtype!(Point);
atom_newtype!(Origin);
atom_newtype!(Loan);
atom_newtype!(Variable);
atom_newtype!(Path);

#[derive(Copy, Clone, Debug)]
pub(crate) struct RustFacts;

impl FactTypes for RustFacts {
    type Origin = Origin;
    type Loan = Loan;
    type Point = Point;
    type Variable = Variable;
    type Path = Path;
}

/// Maps every statement (and terminator) of a basic block to a pair of
/// polonius points: `start` (on entry) and `mid` (after the statement).
pub(crate) struct LocationTable {
    statements_before_block: Vec<usize>,
}

impl LocationTable {
    pub(crate) fn new(body: &Body) -> Self {
        let mut num_points = 0;
        let statements_before_block = body
            .blocks
            .iter()
            .map(|block| {
                let base = num_points;
                num_points += (block.statements.len() + 1) * 2;
                base
            })
            .collect();
        Self {
            statements_before_block,
        }
    }

    pub(crate) fn start_index(&self, block: usize, statement_index: usize) -> Point {
        Point(self.statements_before_block[block] + statement_index * 2)
    }

    pub(crate) fn mid_index(&self, block: usize, statement_index: usize) -> Point {
        Point(self.statements_before_block[block] + statement_index * 2 + 1)
    }

    pub(crate) fn to_location(&self, point: Point) -> (usize, usize) {
        let point_index = point.index();
        let (block, &base) = self
            .statements_before_block
            .iter()
            .enumerate()
            .rfind(|&(_, &base)| base <= point_index)
            .unwrap();
        (block, (point_index - base) / 2)
    }
}

/// The facts and auxiliary events produced from a function body.
pub(crate) struct FactsOutput {
    pub(crate) facts: AllFacts<RustFacts>,
    /// Places that are borrowed (`&x`, `&mut x`, method receivers) at a point.
    /// Unlike a value read, borrowing a merely-uninitialized place is allowed
    /// (C-style `&x` before `*p = ...`); only borrowing a *moved* value is an
    /// error (use-after-move).
    pub(crate) borrows: Vec<(Path, Point)>,
    /// Writes through a pointer: `*p = ...` and writes the callee may perform
    /// through a pointer that escapes to a call. These initialize a local for
    /// move checking, but do not make it *own* anything.
    pub(crate) written: Vec<(Path, Point)>,
}

/// Declares, for each local and CFG edge, the polonius facts that the move
/// error analysis consumes:
/// - the entry: arguments are assigned (initialized), everything else is moved
///   (uninitialized),
/// - every `start -> mid -> start` CFG edge,
/// - the reads/writes of every statement and terminator, including writes
///   through raw pointers and pointers escaping to calls (via points-to),
/// - moves of a local into a call argument or another local,
/// - borrows of a local (for use-after-move).
pub(crate) fn generate_facts(body: &Body, location_table: &LocationTable) -> FactsOutput {
    let mut out = FactsOutput {
        facts: AllFacts::default(),
        borrows: Vec::new(),
        written: Vec::new(),
    };
    let facts = &mut out.facts;

    let arg_count = body.arg_locals().len();
    let entry = location_table.start_index(0, 0);
    for (local, _) in body.local_decls() {
        facts.path_is_var.push((Path(local), Variable(local)));
        if (1..=arg_count).contains(&local) {
            facts.path_assigned_at_base.push((Path(local), entry));
        } else {
            facts.path_moved_at_base.push((Path(local), entry));
        }
    }

    for (block, block_data) in body.blocks.iter().enumerate() {
        let n = block_data.statements.len();
        for i in 0..=n {
            facts.cfg_edge.push((
                location_table.start_index(block, i),
                location_table.mid_index(block, i),
            ));
        }
        for i in 0..n {
            facts.cfg_edge.push((
                location_table.mid_index(block, i),
                location_table.start_index(block, i + 1),
            ));
        }
        let terminator_mid = location_table.mid_index(block, n);
        for &succ in &block_data.terminator.successors() {
            facts
                .cfg_edge
                .push((terminator_mid, location_table.start_index(succ, 0)));
        }
    }

    let block_entry_points_to = compute_points_to(body);
    for (block, block_data) in body.blocks.iter().enumerate() {
        let mut points_to = block_entry_points_to[block].clone();
        for (i, stmt) in block_data.statements.iter().enumerate() {
            let mid = location_table.mid_index(block, i);
            emit_statement(&mut out, body, stmt, &mut points_to, mid);
        }
        let terminator_mid = location_table.mid_index(block, block_data.statements.len());
        emit_terminator(
            &mut out,
            body,
            &block_data.terminator,
            &mut points_to,
            terminator_mid,
        );
    }

    out
}

/// Flow-sensitive may-points-to state for each pointer-typed local: the set of
/// locals it may currently point to (empty = unknown / not a local).
type PointsTo = Vec<BTreeSet<usize>>;

pub(crate) fn local_count(body: &Body) -> usize {
    body.local_decls().count()
}

fn is_pointer_local(body: &Body, local: usize) -> bool {
    body.local_decls().nth(local).is_some_and(|(_, decl)| {
        matches!(
            decl.ty.kind(),
            TyKind::RigidTy(RigidTy::RawPtr(..) | RigidTy::Ref(..))
        )
    })
}

/// The local if `place` is a plain local (no projections).
fn as_local(place: &Place) -> Option<usize> {
    if place.projection.is_empty() {
        Some(place.local)
    } else {
        None
    }
}

/// The pointer local `p` if `place` is `(*p)` or a projection of it.
fn deref_pointer(place: &Place) -> Option<usize> {
    match place.projection.first() {
        Some(ProjectionElem::Deref) => Some(place.local),
        _ => None,
    }
}

fn operand_local(operand: &Operand) -> Option<usize> {
    match operand {
        Operand::Copy(place) | Operand::Move(place) => as_local(place),
        _ => None,
    }
}

fn operand_points_to(points_to: &PointsTo, operand: &Operand) -> BTreeSet<usize> {
    operand_local(operand)
        .map(|local| points_to[local].clone())
        .unwrap_or_default()
}

/// Applies the points-to transfer of `Assign(place, rvalue)`.
fn apply_assign_points_to(body: &Body, points_to: &mut PointsTo, place: &Place, rvalue: &Rvalue) {
    let Some(local) = as_local(place) else {
        return;
    };
    if !is_pointer_local(body, local) {
        return;
    }
    let new_targets = match rvalue {
        // `p = &q` (or `&*q`): `p` now points at `q` / wherever `q` points.
        Rvalue::AddressOf(_, pointee) | Rvalue::Ref(_, _, pointee) => {
            match deref_pointer(pointee) {
                Some(base) => points_to[base].clone(),
                None => BTreeSet::from([pointee.local]),
            }
        }
        // Copying or casting a pointer preserves what it points to.
        Rvalue::Use(operand, _) | Rvalue::Cast(_, operand, _) => {
            operand_points_to(points_to, operand)
        }
        // Loading a pointer from memory loses the points-to information.
        _ => BTreeSet::new(),
    };
    points_to[local] = new_targets;
}

/// Computes the may-points-to state at the entry of every basic block.
fn compute_points_to(body: &Body) -> Vec<PointsTo> {
    let empty = vec![BTreeSet::new(); local_count(body)];
    let mut entry = vec![empty.clone(); body.blocks.len()];
    // Seed with every block: predecessors that contribute nothing must not
    // prevent a block's own statements from being processed (e.g. pointer
    // setup in a later block after calls split the entry block).
    let mut worklist: Vec<usize> = (0..body.blocks.len()).collect();
    while let Some(block) = worklist.pop() {
        let mut out = entry[block].clone();
        for stmt in &body.blocks[block].statements {
            if let StatementKind::Assign(place, rvalue) = &stmt.kind {
                apply_assign_points_to(body, &mut out, place, rvalue);
            }
        }
        for &succ in &body.blocks[block].terminator.successors() {
            let merged: PointsTo = entry[succ].iter().zip(&out).map(|(a, b)| a | b).collect();
            if merged != entry[succ] {
                entry[succ] = merged;
                worklist.push(succ);
            }
        }
    }
    entry
}

fn emit_statement(
    out: &mut FactsOutput,
    body: &Body,
    stmt: &Statement,
    points_to: &mut PointsTo,
    mid: Point,
) {
    if let StatementKind::Assign(place, rvalue) = &stmt.kind {
        if is_direct_place(place) {
            out.facts
                .path_assigned_at_base
                .push((Path(place.local), mid));
        }
        // A write through a raw pointer initializes whatever it points to,
        // but does not make it own anything.
        if let Some(ptr) = deref_pointer(place) {
            for &target in &points_to[ptr] {
                out.written.push((Path(target), mid));
            }
        }
        emit_rvalue_uses(out, rvalue, mid);
        apply_assign_points_to(body, points_to, place, rvalue);
    }
}

fn emit_rvalue_uses(out: &mut FactsOutput, rvalue: &Rvalue, mid: Point) {
    let facts = &mut out.facts;
    match rvalue {
        Rvalue::Use(operand, _) | Rvalue::Cast(_, operand, _) => {
            emit_operand_read(facts, operand, mid);
            emit_operand_move(facts, operand, mid);
        }
        Rvalue::BinaryOp(_, lhs, rhs) | Rvalue::CheckedBinaryOp(_, lhs, rhs) => {
            emit_operand_read(facts, lhs, mid);
            emit_operand_read(facts, rhs, mid);
        }
        Rvalue::UnaryOp(_, operand) | Rvalue::Repeat(operand, _) => {
            emit_operand_read(facts, operand, mid);
        }
        Rvalue::Aggregate(_, operands) => {
            for operand in operands {
                emit_operand_read(facts, operand, mid);
                emit_operand_move(facts, operand, mid);
            }
        }
        Rvalue::CopyForDeref(place) | Rvalue::Discriminant(place) | Rvalue::Len(place) => {
            emit_place_read(facts, place, mid);
        }
        // Borrowing a place (`&x`, `&mut x`, method receivers) is a use for
        // move checking but not for uninitialized-value checking: `int* p = &x`
        // on a not-yet-initialized `x` is the C idiom for initializing through
        // a pointer. Only borrowing a moved value is an error.
        Rvalue::AddressOf(_, place) | Rvalue::Ref(_, _, place) | Rvalue::Reborrow(_, _, place) => {
            if is_direct_place(place) {
                out.borrows.push((Path(place.local), mid));
            }
        }
        Rvalue::ThreadLocalRef(_) => {}
    }
}

fn emit_operand_read(facts: &mut AllFacts<RustFacts>, operand: &Operand, mid: Point) {
    match operand {
        Operand::Copy(place) | Operand::Move(place) => emit_place_read(facts, place, mid),
        Operand::Constant(_) | Operand::RuntimeChecks(_) => {}
    }
}

/// Passing a local by value (`drop(v)`, `push(other_vec)`, `let y = x;`) moves
/// it: it becomes uninitialized and a later use is a use-after-move.
fn emit_operand_move(facts: &mut AllFacts<RustFacts>, operand: &Operand, mid: Point) {
    match operand {
        Operand::Move(place) if is_direct_place(place) => {
            facts.path_moved_at_base.push((Path(place.local), mid));
        }
        _ => {}
    }
}

fn emit_place_read(facts: &mut AllFacts<RustFacts>, place: &Place, mid: Point) {
    if is_direct_place(place) {
        facts.path_accessed_at_base.push((Path(place.local), mid));
    }
}

fn emit_terminator(
    out: &mut FactsOutput,
    body: &Body,
    terminator: &Terminator,
    points_to: &mut PointsTo,
    mid: Point,
) {
    let facts = &mut out.facts;
    match &terminator.kind {
        TerminatorKind::Call {
            args, destination, ..
        } => {
            if is_direct_place(destination) {
                facts
                    .path_assigned_at_base
                    .push((Path(destination.local), mid));
            }
            // A raw pointer passed to a call escapes to the callee, which may
            // write through it, so its pointees become possibly-initialized.
            for arg in args {
                if let Some(ptr) = operand_local(arg) {
                    for &target in &points_to[ptr] {
                        out.written.push((Path(target), mid));
                    }
                }
                emit_operand_read(facts, arg, mid);
                emit_operand_move(facts, arg, mid);
            }
            if let Some(dest) = as_local(destination) {
                points_to[dest] = BTreeSet::new();
            }
        }
        TerminatorKind::SwitchInt { discr, .. } => emit_operand_read(facts, discr, mid),
        TerminatorKind::Assert { cond, .. } => emit_operand_read(facts, cond, mid),
        TerminatorKind::InlineAsm { operands, .. } => {
            for operand in operands {
                if let Some(in_value) = &operand.in_value {
                    emit_operand_read(facts, in_value, mid);
                }
                if let Some(out_place) = &operand.out_place
                    && is_direct_place(out_place)
                {
                    facts
                        .path_assigned_at_base
                        .push((Path(out_place.local), mid));
                }
            }
        }
        TerminatorKind::Return if !body.ret_local().ty.kind().is_unit() => {
            facts.path_accessed_at_base.push((Path(RETURN_LOCAL), mid));
        }
        _ => {}
    }
}

fn is_direct_place(place: &Place) -> bool {
    !place
        .projection
        .iter()
        .any(|elem| matches!(elem, ProjectionElem::Deref))
}
