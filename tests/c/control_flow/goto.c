//@ mode: c
//@ run-status: 0
// Test for goto and labels in C23 (ISO C only, no GNU extensions).
// Covers: forward/backward goto, goto into/out of blocks, loops and
// switch, skipping declarations, multiple labels, label namespace,
// function-scope labels, and the two C23 relaxations:
//   * label at end of compound statement (no trailing statement)
//   * label followed by a declaration
//   * [[maybe_unused]] on labels

#include <stddef.h>
#include <assert.h>

// ------------------------------------------------------------------
// 1. Basic forward goto
// ------------------------------------------------------------------
static int t01(void) {
    int x = 0;
    goto skip;
    x = 99;              // skipped
skip:
    x = 1;
    return x;            // 1
}

// ------------------------------------------------------------------
// 2. Backward goto (manual loop)
// ------------------------------------------------------------------
static int t02(void) {
    int i = 0, sum = 0;
loop:
    if (i < 5) {
        sum += i++;
        goto loop;
    }
    return sum;          // 10
}

// ------------------------------------------------------------------
// 3. goto out of nested blocks
// ------------------------------------------------------------------
static int t03(void) {
    int r = 0;
    {
        {
            {
                goto out;
            }
            r = 1;
        }
        r = 2;
    }
out:
    r = 3;
    return r;            // 3
}

// ------------------------------------------------------------------
// 4. goto into a nested block (past no declarations)
// ------------------------------------------------------------------
static int t04(void) {
    int r = 0;
    goto inner;
    {
        r = 1;
inner:
        r = 2;
    }
    return r;            // 2
}

// ------------------------------------------------------------------
// 5. goto skipping a non-VLA declaration
// ------------------------------------------------------------------
static int t05(void) {
    int r = 0;
    goto skip;
    {
        int x = 99;      // initialization skipped
        (void)x;         // x is in scope but indeterminate; not read
skip:
        r = 1;
    }
    return r;            // 1
}

// ------------------------------------------------------------------
// 6. Multiple labels on the same statement
// ------------------------------------------------------------------
static int t06(void) {
    int x = 0;
    goto b;
a:
b:
c:
    x = 1;
    return x;            // 1
}

// ------------------------------------------------------------------
// 7. Label at end of compound statement (C23, N2508)
// ------------------------------------------------------------------
static int t07(void) {
    int x = 0;
    {
        goto end;
        x = 99;
end:                     // no statement required after the label
    }
    return x;            // 0
}

// ------------------------------------------------------------------
// 8. Label followed by a declaration (C23, N2508)
// ------------------------------------------------------------------
static int t08(void) {
    int x = 0;
    goto label;
    {
label:
        int y = 5;       // declaration directly after label
        x = y;
    }
    return x;            // 5
}

// ------------------------------------------------------------------
// 9. [[maybe_unused]] on labels (C23)
// ------------------------------------------------------------------
static int t09(void) {
    [[maybe_unused]] unused_label:
    goto used_label;
    [[maybe_unused]] used_label:
    return 42;
}

// ------------------------------------------------------------------
// 10. Labels share names with variables, typedefs, enum constants
//     (labels live in their own namespace)
// ------------------------------------------------------------------
typedef int my_type;
enum { MY_ENUM = 100 };

static int t10(void) {
    int my_type = 0;     // variable named my_type
    (void)my_type;

    goto my_type;        // jump to label named my_type
    goto MY_ENUM;        // jump to label named MY_ENUM

    return 0;

my_type:
    return 1;
MY_ENUM:
    return 2;
}

// ------------------------------------------------------------------
// 11. Labels have function scope: same label name in two functions
// ------------------------------------------------------------------
static int t11a(void) { goto end; end: return 1; }
static int t11b(void) { goto end; end: return 2; }

// ------------------------------------------------------------------
// 12. goto out of a switch
// ------------------------------------------------------------------
static int t12(void) {
    int r = 0;
    switch (1) {
        case 1:
            goto out;
    }
    r = 1;
out:
    r = 2;
    return r;            // 2
}

// ------------------------------------------------------------------
// 13. goto into a switch from outside
// ------------------------------------------------------------------
static int t13(void) {
    int x = 0;
    goto in_switch;
    switch (1) {
        case 1:
in_switch:
            x = 7;
            break;
        default:
            x = -1;
    }
    return x;            // 7
}

// ------------------------------------------------------------------
// 14. goto out of a for loop
// ------------------------------------------------------------------
static int t14(void) {
    int sum = 0;
    for (int i = 0; i < 100; i++) {
        if (i == 5) goto out;
        sum += i;
    }
    sum = -1;
out:
    return sum;          // 10
}

// ------------------------------------------------------------------
// 15. goto into a for loop body (loop variable declared outside)
// ------------------------------------------------------------------
static int t15(void) {
    int i = 0, sum = 0;
    goto inside;
    for (i = 0; i < 5; i++) {
inside:
        sum += i;
    }
    return sum;          // 10
}

// ------------------------------------------------------------------
// 16. goto out of a while loop
// ------------------------------------------------------------------
static int t16(void) {
    int i = 0, sum = 0;
    while (i < 10) {
        if (i == 3) goto out;
        sum += i++;
    }
out:
    return sum;          // 3
}

// ------------------------------------------------------------------
// 17. goto into a while loop body
// ------------------------------------------------------------------
static int t17(void) {
    int i = 0, sum = 0;
    goto inside;
    while (i < 3) {
inside:
        sum += i;
        i++;
    }
    return sum;          // 3
}

// ------------------------------------------------------------------
// 18. goto out of a do-while loop
// ------------------------------------------------------------------
static int t18(void) {
    int i = 0, sum = 0;
    do {
        if (i == 4) goto out;
        sum += i++;
    } while (i < 10);
out:
    return sum;          // 6
}

// ------------------------------------------------------------------
// 19. goto into a do-while loop body
// ------------------------------------------------------------------
static int t19(void) {
    int i = 0, sum = 0;
    goto inside;
    do {
inside:
        sum += i;
        i++;
    } while (i < 3);
    return sum;          // 3
}

// ------------------------------------------------------------------
// 20. Empty statement after label
// ------------------------------------------------------------------
static int t20(void) {
    int x = 0;
    goto empty;
    x = 99;
empty: ;
    return x;            // 0
}

// ------------------------------------------------------------------
// 21. Label inside an else branch, jumped to from the if branch
// ------------------------------------------------------------------
static int t21(int flag) {
    int x = 0;
    if (flag) {
        goto else_branch;
    } else {
else_branch:
        x = 1;
    }
    return x;            // 1 for both flag values
}

// ------------------------------------------------------------------
// 22. goto into an if block from outside
// ------------------------------------------------------------------
static int t22(int flag) {
    int x = 0;
    goto inside_if;
    if (flag) {
inside_if:
        x = 1;
    }
    return x;            // 1
}

// ------------------------------------------------------------------
// 23. Label before a case label (goto label and case label on same stmt)
// ------------------------------------------------------------------
static int t23(int n) {
    int r = 0;
    switch (n) {
        goto case2;
        case 1:
            r = 1;
            break;
case2:
        case 2:
            r = 2;
            break;
        default:
            r = -1;
    }
    return r;            // 2 for n == 2
}

// ------------------------------------------------------------------
// 24. goto out of a nested switch
// ------------------------------------------------------------------
static int t24(int n) {
    int r = 0;
    switch (n) {
        case 1:
            switch (n) {
                case 1:
                    goto out;
            }
            r = 1;
            break;
        default:
            r = -1;
    }
out:
    r = 2;
    return r;            // 2 for n == 1
}

// ------------------------------------------------------------------
// 25. Label alone at end of block (C23), no trailing statement
// ------------------------------------------------------------------
static int t25(void) {
    int x = 0;
    {
        goto done;
done:                     // end of compound statement
    }
    x = 1;
    return x;            // 1
}

// ------------------------------------------------------------------
// 26. static local initialization is not skipped by goto
// ------------------------------------------------------------------
static int t26(void) {
    static int s = 100;
    goto skip;
    s = 200;
skip:
    return s;            // 100 (static init runs before main)
}

// ------------------------------------------------------------------
// 27. goto as idiomatic multi-loop break-out
// ------------------------------------------------------------------
static int t27(void) {
    int count = 0;
    for (int i = 0; i < 10; i++) {
        for (int j = 0; j < 10; j++) {
            for (int k = 0; k < 10; k++) {
                count++;
                if (i == 1 && j == 2 && k == 3) {
                    goto done;
                }
            }
        }
    }
done:
    return count;        // 124
}

// ------------------------------------------------------------------
// 28. Label before the first case label of a switch
// ------------------------------------------------------------------
static int t28(int n) {
    int r = 0;
    goto first;
    switch (n) {
first:
        case 1:
            r = 1;
            break;
        default:
            r = -1;
    }
    return r;            // 1 for any n
}

// ------------------------------------------------------------------
// 29. Label before the default case label
// ------------------------------------------------------------------
static int t29() {
    int r = 1;
    switch (r) {
        case 42:
            r = 1;
            break;
last:
        default:
            r = -1;
    }
    if (r == 1) goto last;    // jump to default case
    return r;
}

// ------------------------------------------------------------------
// 30. Labels are in their own namespace relative to struct/union tags
// ------------------------------------------------------------------
struct my_type { int a; };

static int t30(void) {
    goto my_type;
    struct my_type s = { 0 };  // uses the struct tag, not the label
    (void)s;
    return 0;
my_type:
    return 7;
}

// ------------------------------------------------------------------
// main
// ------------------------------------------------------------------
int main(void) {
    assert(t01() == 1);
    assert(t02() == 10);
    assert(t03() == 3);
    assert(t04() == 2);
    assert(t05() == 1);
    assert(t06() == 1);
    assert(t07() == 0);
    assert(t08() == 5);
    assert(t09() == 42);
    assert(t10() == 1);
    assert(t11a() == 1);
    assert(t11b() == 2);
    assert(t12() == 2);
    assert(t13() == 7);
    assert(t14() == 10);
    assert(t15() == 10);
    assert(t16() == 3);
    assert(t17() == 3);
    assert(t18() == 6);
    assert(t19() == 3);
    assert(t20() == 0);
    assert(t21(0) == 1);
    assert(t21(1) == 1);
    assert(t22(0) == 1);
    assert(t22(1) == 1);
    assert(t23(2) == 2);
    assert(t24(1) == 2);
    assert(t25() == 1);
    assert(t26() == 100);
    assert(t27() == 124);
    assert(t28(0) == 1);
    assert(t28(1) == 1);
    assert(t29() == -1);
    assert(t30() == 7);
    return 0;
}
