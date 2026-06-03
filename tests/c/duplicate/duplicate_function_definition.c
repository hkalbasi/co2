//@ mode: c
//@ compile-fail

int f1() {
    return 1;
}

int f1() {
//  ^^ error: the name `f1` is defined multiple times
    return 2;
}

int f2();
extern int f2();

int f3() {
    return 1;
}

int f3 = 5;
//  ^^ error: the name `f3` is defined multiple times

int f4();
int f4;
//  ^^ error: the name `f4` is defined multiple times

int f5 = 3;
int f5();
//  ^^ error: the name `f5` is defined multiple times

int f6;
extern int f6();
       //  ^^ error: the name `f6` is defined multiple times

int main() {
    return 0;
}
