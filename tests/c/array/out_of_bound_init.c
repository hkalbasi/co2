//@ mode: c
//@ run-status: 0

int g1[3] = {1, 2, 3, 4};
                   // ^ warning: excess elements in array initializer

char g2[3] = "a";
char g3[3] = "ab";
char g4[3] = "abc";
char g5[3] = "abcd";
           //^^^^^^ warning: excess elements in array initializer

int main() {
    int l1[3] = {1, 2, 3, 4};
                       // ^ warning: excess elements in array initializer
    int l2[3] = {[0] = 1, [10] = 5};
                       // ^^^^ warning: initializer designator index 10 exceeds array bounds
    static int gl1[3] = {1, 2, 3, 4};
                               // ^ warning: excess elements in array initializer
    return 0;
}
