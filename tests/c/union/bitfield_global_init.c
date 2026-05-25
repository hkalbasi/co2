//@ mode: c
//@ run-status: 0

/* Regression test: global union with bitfield initializer used to panic
 * with "assertion failed: !adt_def.is_union()" in the MIR validator because
 * the HIR bitfield-init path emitted HirExprKind::Aggregate (multi-field) for
 * unions instead of HirExprKind::UnionAggregate (single active field). */

#include <stdint.h>

/* Single-bitfield union – simplest trigger case. */
union U1 {
    signed f0 : 1;
};
static union U1 g_u1 = {-1};

/* Union mixing direct fields and a bitfield. */
union U2 {
    uint32_t f0;
    uint32_t f1;
    const unsigned f2 : 6;
    uint32_t f3;
};
static union U2 g_u2 = {42};

int main(void) {
    /* 1-bit signed holds -1 as all-ones */
    if (g_u1.f0 != -1) return 1;

    /* Initializer sets first field (f0) to 42 */
    if (g_u2.f0 != 42) return 2;

    return 0;
}
