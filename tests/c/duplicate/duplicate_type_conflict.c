//@ mode: c
//@ compile-fail

typedef int T1;

int T1;
//  ^^ error: the name `T1` is defined multiple times

typedef int T2;

int T2() {
//  ^^ error: the name `T2` is defined multiple times
    return 1;
}

typedef int T3;

T1 T3;
// ^^ error: the name `T3` is defined multiple times

T1 T4;
typedef int T4;
         // ^^ error: the name `T4` is defined multiple times

int main() {
    return 0;
}
