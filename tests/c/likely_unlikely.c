//@ mode: c
//@ run-status: 0

#define likely(x)   __builtin_expect(!!(x), 1)
#define unlikely(x) __builtin_expect(!!(x), 0)

int main() {
    int x = 5;
    if (likely(x > 0)) {
        return 0;
    }
    if (unlikely(x < 0)) {
        return 1;
    }
    return 0;
}