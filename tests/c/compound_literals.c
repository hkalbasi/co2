//@ mode: c
//@ run-status: 0

#include <stddef.h>
#include <assert.h>

int sidefx_counter = 0;

int sidefx(void) {
    sidefx_counter++;
    return 1;
}

// Helper structs for testing
struct point { int x; int y; };
struct nested { struct point p; int z; };

int main(void) {
    // ------------------------------------------------------------
    // basic scalar types
    // ------------------------------------------------------------
    {
        int a = (int){42};
        assert(a == 42);

        double b = (double){3.14};
        assert(b >= 3.13 && b <= 3.15);

        char c = (char){'A'};
        assert(c == 'A');

        _Bool d = (_Bool){1};
        assert(d == 1);
    }

    // ------------------------------------------------------------
    // pointer types
    // ------------------------------------------------------------
    {
        int x = 10;
        int *p = (int *){&x};
        assert(*p == 10);

        // pointer to compound literal itself (temporary)
        int *q = &(int){20};
        assert(*q == 20);
    }

    // ------------------------------------------------------------
    // arrays
    // ------------------------------------------------------------
    {
        int *arr = (int[]){1, 2, 3, 4};
        assert(arr[0] == 1);
        assert(arr[1] == 2);
        assert(arr[2] == 3);
        assert(arr[3] == 4);

        // sized array
        int (*sized)[4] = &(int[4]){5, 6, 7, 8};
        assert((*sized)[0] == 5);
        assert((*sized)[3] == 8);

        // multidimensional array
        int (*mat)[2][2] = &(int[2][2]){{1,2},{3,4}};
        assert((*mat)[0][0] == 1);
        assert((*mat)[1][1] == 4);

        // empty array (GCC extension, but valid if supported; we'll check size)
        // Leave this out for strict standards compliance.
    }

    // ------------------------------------------------------------
    // struct and union
    // ------------------------------------------------------------
    {
        struct point p = (struct point){10, 20};
        assert(p.x == 10);
        assert(p.y == 20);

        struct nested n = (struct nested){ .p = {1,2}, .z = 3 };
        assert(n.p.x == 1);
        assert(n.p.y == 2);
        assert(n.z == 3);

        // union (take first member)
        union some_union { int i; float f; } u = (union some_union){.i = 99};
        assert(u.i == 99);
    }

    // ------------------------------------------------------------
    // const and volatile qualifiers
    // ------------------------------------------------------------
    {
        const int ci = (const int){100};
        assert(ci == 100);

        volatile int vi = (volatile int){200};
        assert(vi == 200);

        // pointer to const
        const int *pc = (const int[]){1,2,3};
        assert(pc[0] == 1);
    }

    // ------------------------------------------------------------
    // scope and lifetime: block scope local
    // ------------------------------------------------------------
    {
        int *p;
        {
            p = (int[]){5, 6, 7}; // valid within this block
            assert(p[1] == 6);
        }
        // After block, the array's lifetime ends. Using p is UB, so we don't test.
    }

    // ------------------------------------------------------------
    // side effects in initialiser list (evaluated once, in order)
    // ------------------------------------------------------------
    {
        sidefx_counter = 0;
        int arr[4] = {0};
        // Use a compound literal in a context that forces evaluation
        // but compound literal inside expression: side effects occur.
        int *p = (int[2]){sidefx(), sidefx()};
        assert(p[0] == 1);
        assert(p[1] == 1);
        assert(sidefx_counter == 2); // two calls
    }

    // ------------------------------------------------------------
    // taking address and type
    // ------------------------------------------------------------
    {
        int (*p)[3] = &(int[3]){10,20,30};
        assert((*p)[1] == 20);

        // sizeof
        assert(sizeof(int[5]) == sizeof((int[5]){0}));

        // typeof (if compiler supports typeof, but we'll skip to stay C standard)
        // We'll just check that compound literal type is as expected.
    }

    // ------------------------------------------------------------
    // nested compound literals
    // ------------------------------------------------------------
    {
        struct point *pp = &(struct point){
            .x = (int){15},
            .y = (int){25}
        };
        assert(pp->x == 15);
        assert(pp->y == 25);

        // array of structs
        struct point *parr = (struct point[]){
            (struct point){1,2},
            (struct point){3,4}
        };
        assert(parr[0].x == 1);
        assert(parr[1].y == 4);
    }

    // ------------------------------------------------------------
    // assignments and modifications (array compound literals are lvalues)
    // ------------------------------------------------------------
    {
        int *p = (int[]){1,2,3};
        p[0] = 99;
        assert(p[0] == 99);
        assert(p[1] == 2);
    }

    // ------------------------------------------------------------
    // compound literal inside sizeof
    // ------------------------------------------------------------
    {
        size_t sz1 = sizeof(int[10]);
        size_t sz2 = sizeof((int[10]){1,2,3}); // same size even if not fully initialized
        assert(sz1 == sz2);
    }

    // ------------------------------------------------------------
    // compound literal in ternary / comma operators
    // ------------------------------------------------------------
    {
        int x = 5;
        int *p = x > 0 ? (int[]){1,2} : (int[]){3,4};
        assert(p[0] == 1);
    }

    // ------------------------------------------------------------
    // compound literal as function argument (pointer decay)
    // ------------------------------------------------------------
    {
        // Using a helper to test pointer pass
        // We'll test that the pointer points to the first element
        int first = ((int[]){7,8,9})[0];
        assert(first == 7);
    }

    return 0;
}
