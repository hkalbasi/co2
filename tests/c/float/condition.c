//@ mode: c
//@ run-status: 0

#include <assert.h>
#include <math.h>
#include <float.h>

int main(void)
{
    /* Basic truth values */
    assert((0.0 ? 0 : 1) == 1);
    assert((-0.0 ? 0 : 1) == 1);
    assert((1.0 ? 1 : 0) == 1);
    assert((-1.0 ? 1 : 0) == 1);

    /* Smallest values */
    assert((DBL_MIN ? 1 : 0) == 1);

#if __STDC_VERSION__ >= 201112L
    assert((DBL_TRUE_MIN ? 1 : 0) == 1);
#endif

    /* Infinity and NaN */
    assert((INFINITY ? 1 : 0) == 1);
    assert((-INFINITY ? 1 : 0) == 1);
    assert((NAN ? 1 : 0) == 1);

    /* if */
    {
        int x = 0;
        if (3.14)
            x = 1;
        assert(x == 1);

        x = 0;
        if (0.0)
            x = 1;
        else
            x = 2;
        assert(x == 2);
    }

    /* while */
    {
        int n = 0;
        while (2.0) {
            ++n;
            break;
        }
        assert(n == 1);

        n = 0;
        while (0.0)
            ++n;
        assert(n == 0);
    }

    /* do-while */
    {
        int n = 0;
        do {
            ++n;
        } while (0.0);
        assert(n == 1);

        n = 0;
        do {
            ++n;
        } while (2.0 && n < 3);
        assert(n == 3);
    }

    /* for */
    {
        int n = 0;
        for (; 1.0; ) {
            ++n;
            break;
        }
        assert(n == 1);

        n = 0;
        for (; 0.0; )
            ++n;
        assert(n == 0);
    }

    /* switch using ?: whose condition is floating-point */
    {
        switch (0.5 ? 10 : 20) {
        case 10:
            break;
        default:
            assert(0);
        }

        switch (0.0 ? 10 : 20) {
        case 20:
            break;
        default:
            assert(0);
        }

        switch (NAN ? 1 : 2) {
        case 1:
            break;
        default:
            assert(0);
        }
    }

    /* Nested conditional operators */
    assert((0.0 ? 1 : 2.0 ? 3 : 4) == 3);
    assert((0.0 ? 1 : 0.0 ? 3 : 4) == 4);
    assert((NAN ? 5 : 6) == 5);

    /* Volatile values (prevent over-optimization) */
    {
        volatile double z = 0.0;
        volatile double o = 1.0;

        assert((o ? 1 : 0) == 1);
        assert((z ? 1 : 0) == 0);
        assert(((o / z) ? 1 : 0) == 1); /* +inf */
        assert(((z / z) ? 1 : 0) == 1); /* NaN */
    }

    return 0;
}