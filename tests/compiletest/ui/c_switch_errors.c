//@ mode: c
//@ compile-fail

void case_outside_switch(void) {
    int x = 1;
    case 1:;
//       ^ error: case label outside of switch body
}

void default_outside_switch(void) {
    int x = 1;
    default:
//  ^^^^^^^ error: default label outside of switch body
        x = 2;
}

struct S { int x; };

void switch_on_struct(void) {
    struct S s = {1};
    switch(s) {
//         ^ error: switch expression must be integer-like, got co2(struct S)
        case 1: break;
    }
}

void case_float_label(void) {
    int x = 1;
    switch(x) {
        case 1.0:;
//           ^^^ error: switch case expression must be integer-like, got f64
    }
}

void duplicate_default(void) {
    int x = 1;
    switch(x) {
        default:;
        default:;
    //  ^^^^^^^ duplicate `default` label in switch
    }
}

int main(void) {
    return 0;
}
