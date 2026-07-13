//@ mode: c
//@ compile-fail

int f1() {
    int x = 0x;
         // ^^ error: Invalid hexadecimal int literal
}

int f2() {
    double x = 0e;
            // ^^ error: Invalid float literal
}

int f3() {
    char x = '\x';
          // ^^^^ error: Invalid character constant
}

int main() {}
