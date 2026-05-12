//@ mode: c
//@ run-status: 0

int ar1[12] = {};
int ar2[12 + 3] = {};
int ar3[12 / 3 + 4 * 5] = {};
int ar4[sizeof(int) + 3] = {};
int ar5[sizeof(ar3) + 3] = {};

enum Foo {
  Var1 = 5 * 3,
  Var2,
  Var3 = sizeof(ar5),
  Var4 = sizeof("hello"),
  Var5,
};

int ar6[Var1] = {};
int ar7[Var2] = {};
int ar8[Var1 * Var2] = {};
int ar9[Var1 * sizeof(ar3)] = {};
int ar10[Var3] = {};
int ar11[1 ? 20 : 30] = {};
int ar12[5 && 3] = {};
int ar13[0 || 7] = {};
int ar14[5 << 2] = {};
int ar15[20 >> 1] = {};
int ar16[5 | 2] = {};
int ar17[5 ^ 3] = {};
int ar18[!0] = {};
int ar19[+10] = {};
int ar20[- -10] = {};
int ar21[5 > 3] = {};
int ar22[5 == 5] = {};
int ar23[5 != 3 ? 100 : 0] = {};
int ar24[sizeof("hello") - 1] = {};
int ar25[(int)sizeof(short)] = {};
int ar26[sizeof(enum Foo)] = {};
int ar27[sizeof(ar26)] = {};
int ar28[12 * 3 + 6 / 2] = {};
int ar29[(12 + 3) * 2] = {};
int ar30[(int)(unsigned char)300] = {};
int ar31[sizeof(ar3) / sizeof(int) + 1] = {};
int ar32[100 - Var5] = {};
typedef int (*func_ptr)(int);
typedef union {
  func_ptr f;
  int i;
} U;
int ar33[sizeof(U)] = {};

int the_ar[] = {
    [12] = 1,
    [12 + 3] = 2,
    [12 / 3 + 4 * 5] = 3,
    [sizeof(int) + 3] = 4,
    [sizeof(ar3) + 3] = 5,
    [Var1] = 6,
    [Var2] = 7,
    [Var1 * Var2] = 8,
    [Var1 * sizeof(ar3)] = 9,
    [Var3] = 10,
    [1 ? 20 : 30] = 11,
    [5 && 3] = 12,
    [0 || 7] = 13,
    [5 << 2] = 14,
    [20 >> 1] = 15,
    [5 | 2] = 16,
    [5 ^ 3] = 17,
    [!0] = 18,
    [+10] = 19,
    [- -10] = 20,
    [5 > 3] = 21,
    [5 == 5] = 22,
    [5 != 3 ? 100 : 0] = 23,
    [sizeof("hello") - 1] = 24,
    [(int)sizeof(short)] = 25,
    [sizeof(enum Foo)] = 26,
    [sizeof(ar26)] = 27,
    [12 * 3 + 6 / 2] = 28,
    [(12 + 3) * 2] = 29,
    [(int)(unsigned char)300] = 30,
    [sizeof(ar3) / sizeof(int) + 1] = 31,
    [100 - Var5] = 32,
};

int main() {
  if (sizeof(ar1) / sizeof(int) != 12)
    return 1;
  if (sizeof(ar2) / sizeof(int) != 15)
    return 2;
  if (sizeof(ar3) / sizeof(int) != 24)
    return 3;
  if (sizeof(ar4) / sizeof(int) != sizeof(int) + 3)
    return 4;
  if (sizeof(ar5) / sizeof(int) != sizeof(ar3) + 3)
    return 5;
  if (sizeof(ar6) / sizeof(int) != Var1)
    return 6;
  if (sizeof(ar7) / sizeof(int) != Var2)
    return 7;
  if (sizeof(ar8) / sizeof(int) != 240)
    return 8;
  if (sizeof(ar9) / sizeof(int) != Var1 * sizeof(ar3))
    return 9;
  if (sizeof(ar10) / sizeof(int) != sizeof(ar5))
    return 10;

  if (sizeof(ar11) / sizeof(int) != 20)
    return 11;
  if (sizeof(ar12) / sizeof(int) != 1)
    return 12;
  if (sizeof(ar13) / sizeof(int) != 1)
    return 13;
  if (sizeof(ar14) / sizeof(int) != 20)
    return 14;
  if (sizeof(ar15) / sizeof(int) != 10)
    return 15;
  if (sizeof(ar16) / sizeof(int) != 7)
    return 16;
  if (sizeof(ar17) / sizeof(int) != 6)
    return 17;
  if (sizeof(ar18) / sizeof(int) != 1)
    return 18;
  if (sizeof(ar19) / sizeof(int) != 10)
    return 19;
  if (sizeof(ar20) / sizeof(int) != 10)
    return 20;
  if (sizeof(ar21) / sizeof(int) != 1)
    return 21;
  if (sizeof(ar22) / sizeof(int) != 1)
    return 22;
  if (sizeof(ar23) / sizeof(int) != 100)
    return 23;
  if (sizeof(ar24) / sizeof(int) != 5)
    return 24;
  if (sizeof(ar25) / sizeof(int) != (int)sizeof(short))
    return 25;
  if (sizeof(ar26) / sizeof(int) != sizeof(enum Foo))
    return 26;
  if (sizeof(ar27) / sizeof(int) != sizeof(ar26))
    return 27;
  if (sizeof(ar28) / sizeof(int) != 39)
    return 28;
  if (sizeof(ar29) / sizeof(int) != 30)
    return 29;
  if (sizeof(ar30) / sizeof(int) != 44)
    return 30;
  if (sizeof(ar31) / sizeof(int) != 25)
    return 31;
  if (sizeof(ar32) / sizeof(int) != 93)
    return 32;
  if (sizeof(ar33) / sizeof(int) != sizeof(func_ptr))
    return 33;

  if (the_ar[12] == 1 && the_ar[12 + 3] == 2 && the_ar[12 / 3 + 4 * 5] == 3 &&
      the_ar[sizeof(ar3) + 3] == 5 && the_ar[Var1 * Var2] == 8 &&
      the_ar[Var1 * sizeof(ar3)] == 9 && the_ar[5 ^ 3] == 17 &&
      the_ar[5 != 3 ? 100 : 0] == 23 && the_ar[(int)sizeof(short)] == 25 &&
      the_ar[sizeof(enum Foo)] == 26 && the_ar[sizeof(ar26)] == 27 &&
      the_ar[12 * 3 + 6 / 2] == 28 && the_ar[(12 + 3) * 2] == 29 &&
      the_ar[(int)(unsigned char)300] == 30 &&
      the_ar[sizeof(ar3) / sizeof(int) + 1] == 31 && the_ar[100 - Var5] == 32 &&
      the_ar[5 | 2] == 16 && the_ar[- -10] == 20 &&
      the_ar[sizeof("hello") - 1] == 24 && the_ar[5 == 5] == 22 &&
      the_ar[5 << 2] == 14) {
    return 33;
  }

  return 0;
}