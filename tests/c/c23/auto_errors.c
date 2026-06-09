//@ mode: c
//@ compile-fail

void f1(void) {
    int x = 5;
    auto *p = &x;
       //^^ error: `auto` requires a plain identifier, possibly with attributes, as declarator
}

void f2(void) {
    auto p = {1, 2, 3};
       //^ error: `auto` cannot be used with initializer lists
}

void f3(void) {
    auto p;
       //^ error: `auto` requires an initializer
}

int main() {}