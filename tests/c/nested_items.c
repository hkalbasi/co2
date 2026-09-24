//@ mode: c
//@ run-status: 7

int jq_regression(void) {
    static long x;
    typedef int verify[sizeof x == sizeof(long) ? 1 : -1];
    static verify x2[3];
    typedef int verify2[sizeof x2 == sizeof(int[3]) ? 1 : -1];
    extern long x3;
    typedef int verify3[sizeof x3 == sizeof(long) ? 1 : -1];
    return sizeof(verify2) + sizeof(verify3);
}

int main() {
    typedef int nested_type;
    nested_type x = 7;
    if (jq_regression() != 2 * sizeof(int)) {
        return 5;
    }
    return x;
}
