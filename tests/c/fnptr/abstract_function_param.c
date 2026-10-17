//@ mode: c
//@ run-status: 0

// A parameter written as an abstract function-type declarator, e.g.
// `int (int)` or `int (int x)`, is valid C and decays to a function pointer
// (C11 6.7.6.3). The declaration must be compatible with the pointer
// definition, so a mis-parse would conflict. Regression test: the parser
// used to reject these declarations.

typedef int (*fptr)(int);

int apply(int (int), int);
int apply_named(int (int x), int);
int apply(int (*f)(int), int x) { return f(x); }
int apply_named(int (*f)(int), int x) { return f(x) + 1; }

static int inc(int x) { return x + 1; }

int main(void) {
    if (apply(inc, 41) != 42) {
        return 1;
    }
    if (apply_named(inc, 41) != 43) {
        return 2;
    }
    return 0;
}
