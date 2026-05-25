//@ mode: c
//@ run-status: 0

// Regression test: parsing sizeof(struct {...}) inside an array dimension
// previously caused "RefCell already borrowed" panic because the Ref guards
// from source_name/source borrow in register_array_len_const were still alive
// while parse_expression_tokens tried to borrow_mut to register the struct.

struct Pair { int x; int y; };

char g[sizeof(struct { int a; int b; })];

typedef struct {
    char buf[sizeof(struct Pair)];
} Wrapped;

int main(void) {
    if (sizeof(Wrapped) != sizeof(g) || sizeof(g) != sizeof(struct Pair)) {
        return 1;
    }
    return 0;
}
