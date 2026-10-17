static int value;
static int calls;

static __attribute__((noinline)) int target(int x) { return x * 2 + 1; }

// GNU attribute between `*` and the declarator name: same `noinline`
// attribute, but written after the `*`. The parser must skip it there too.
// The increment keeps the call observable: without `noinline` this small
// body is inlined and the call disappears, so the call tests that the
// attribute was actually parsed.
static int *__attribute__((noinline)) get_ptr(void) { calls++; return &value; }

int main(int argc, char **argv) {
  (void)argv;
  *get_ptr() = argc;
  return target(argc) != 3 || value != argc || calls != 1;
}
