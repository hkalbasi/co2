//@ mode: c
//@ run-status: 0

// TODO: actually test the alignments

typedef struct {
    alignas(16) int new_align;
    _Alignas(32) int old_align;
} Foo;

typedef unsigned char byte;

struct header {
    int size;
    _Alignas(8) byte user_data[];
};

int main() {
    // Check aligned fields are usable
    Foo x = { .new_align = 5, .old_align = 10 };
    if (x.new_align != 5 || x.old_align != 10) {
        return 1;
    }
    alignas(16) int y = 5;
    y += 3;
    if (y != 8) {
        return 2;
    }
    struct header h = { .size = 3 };
    if (h.size != 3) {
        return 3;
    }
    return 0;
}
