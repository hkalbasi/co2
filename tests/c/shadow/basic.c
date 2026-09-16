//@ mode: c
//@ run-status: 0
// Test for shadowing in C: inner declarations hide outer ones.

#include <stddef.h>
#include <assert.h>

// ------------------------------------------------------------
// Globals for shadowing tests
// ------------------------------------------------------------

int global_var = 1;

// Function that will be shadowed by a local variable
int shadowed_func(void) {
    return 42;
}

// Global typedef
typedef int my_type;

// Global enum constant
enum { ENUM_CONST = 100 };

// ------------------------------------------------------------
// Function with a parameter that shadows a global
// ------------------------------------------------------------

void param_shadow(int global_var) {
    // Parameter shadows the global
    assert(global_var == 10);
    // The global is not accessible here.
}

// ------------------------------------------------------------
// Main test
// ------------------------------------------------------------

int main(void) {
    // ------------------------------------------------------------
    // 1. Basic block shadowing
    // ------------------------------------------------------------
    int x = 1;
    assert(x == 1);
    {
        int x = 2;          // shadows outer x
        assert(x == 2);
        {
            int x = 3;      // shadows the x=2
            assert(x == 3);
        }
        assert(x == 2);     // back to middle x
    }
    assert(x == 1);         // back to outer x

    // ------------------------------------------------------------
    // 2. Shadowing a global variable with a local
    // ------------------------------------------------------------
    assert(global_var == 1);
    {
        int global_var = 5; // shadows global
        assert(global_var == 5);
    }
    assert(global_var == 1); // global restored

    // ------------------------------------------------------------
    // 3. Shadowing a function with a local variable
    // ------------------------------------------------------------
    // Before shadowing, we can call the function
    assert(shadowed_func() == 42);
    {
        int shadowed_func = 10; // shadows the function name
        assert(shadowed_func == 10);
        // Cannot call the function here.
    }
    // After the block, the function is visible again
    assert(shadowed_func() == 42);

    // ------------------------------------------------------------
    // 4. Shadowing a typedef with a variable
    // ------------------------------------------------------------
    {
        my_type t1 = 100;       // use the typedef
        assert(t1 == 100);

        int my_type = 200;      // variable shadows the typedef
        assert(my_type == 200);
        // The typedef is not accessible here.
    }
    // After block, typedef is visible again
    my_type t2 = 300;
    assert(t2 == 300);

    // ------------------------------------------------------------
    // 5. Shadowing an enum constant with a local variable
    // ------------------------------------------------------------
    assert(ENUM_CONST == 100);
    {
        int ENUM_CONST = 200;   // shadows the enum constant
        assert(ENUM_CONST == 200);
    }
    assert(ENUM_CONST == 100);  // enum constant visible again

    // ------------------------------------------------------------
    // 6. Shadowing in for-loop initializer
    // ------------------------------------------------------------
    int i = 10;
    for (int i = 0; i < 3; i++) {
        assert(i >= 0 && i < 3);    // inner i
    }
    assert(i == 10);                // outer i unchanged

    // ------------------------------------------------------------
    // 7. Shadowing in while-loop condition (using a block)
    // ------------------------------------------------------------
    int j = 5;
    {
        int j = 0;
        while (j < 2) {
            assert(j == 0 || j == 1);
            j++;
        }
    }
    assert(j == 5);

    // ------------------------------------------------------------
    // 8. Shadowing with different types
    // ------------------------------------------------------------
    {
        int a = 1;
        assert(a == 1);
        {
            double a = 3.14;    // shadows int a
            assert(a > 3.13 && a < 3.15);
        }
        assert(a == 1);
    }

    // ------------------------------------------------------------
    // 9. Parameter shadowing (call function)
    // ------------------------------------------------------------
    param_shadow(10);   // passes 10, asserts inside

    // ------------------------------------------------------------
    // 10. Shadowing of a label? Labels are separate namespace, so no shadowing.
    //     We skip labels.
    // ------------------------------------------------------------

    // ------------------------------------------------------------
    // 11. Shadowing of a struct tag? Tags have separate namespace, so no shadowing.
    //     We skip tags.
    // ------------------------------------------------------------

    // ------------------------------------------------------------
    // 12. Shadowing with same name in different scopes but no nesting (siblings)
    // ------------------------------------------------------------
    {
        int s = 1;
        assert(s == 1);
    }
    {
        int s = 2;
        assert(s == 2);
    }

    // ------------------------------------------------------------
    // 13. Shadowing a variable by a function parameter (in a nested function)
    //     Since C doesn't have nested functions (except GCC extension),
    //     we test with a separate function.
    // ------------------------------------------------------------
    // Already done in param_shadow.

    // ------------------------------------------------------------
    // 14. Shadowing with static and extern? Not needed.
    // ------------------------------------------------------------

    return 0;
}
