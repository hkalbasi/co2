//@ mode: c
//@ compile-fail

int main() {
    int x = 2;
    return x(3);
         //^ Type not callable
}
