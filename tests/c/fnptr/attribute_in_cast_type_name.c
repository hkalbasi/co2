//@ mode: c
//@ run-status: 0

// GNU `__attribute__((...))` inside a cast type-name, e.g.
// `((ATTR int (*)(void)) p)()` and `int (ATTR *)(void)`. The preprocessor
// rewrites the attribute to a `#[...]` marker; the parser used to reject it
// inside type-names with "expected expression, found #".
// Regression test: the attribute must be skipped and the cast parsed.

#define ATTR __attribute__((__noinline__))

static int actual_function(void) { return 42; }

int main(void) {
    void *function_pointer = &actual_function;
    int a = ((ATTR int (*)(void))function_pointer)();
    int b = ((int(ATTR *)(void))function_pointer)();
    return (a == 42 && b == 42) ? 0 : 1;
}
