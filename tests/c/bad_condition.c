//@ mode: c
//@ compile-fail

typedef struct { int x; } St;

int in_if() {
    St st = { 5 };
    if (st) {
    //  ^^ error: condition must be scalar-like, got co2(struct #0)
        return 3;
    }
}

int in_ternary() {
    St st = { 5 };
    return st ? 0 : 1;
       //  ^^ error: condition must be scalar-like, got co2(struct #0)
}

int in_logical_and() {
    St st = { 5 };
    // TODO: this is bad error span.
    return st && st.x == 1;
       //  ^^^^^^^^^^^^^^^ error: condition must be scalar-like, got co2(struct #0)
}

int main() {
    return 0;
}
