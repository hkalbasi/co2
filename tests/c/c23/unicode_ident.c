//@ mode: c
//@ run-status: 0
//@ run-stdout: FILE: unicode_ident.out

#include <stdio.h>

/* Greek */
constexpr int α = 1;
constexpr int β = 2;
constexpr int γ = α + β;

/* Mixed scripts */
int 日本 = 10;
int привет = 20;
int Ελληνικά = 30;
int عربي = 40;

/* Emoji (should fail if enabled) */
#if 0
int 😀 = 42;
#endif

/* Universal character names */
int \u03C0 = 314;
int \u03BB = 7;

/* Supplementary Plane */
int 𐐀 = 5;

/* Confusable identifiers */
int a = 1;
int а = 2; /* Cyrillic 'a' */

/* Long identifier */
int 我真的真的真的真的真的真的真的真的真的真的真的真的真的真的真的真的真的真的真的真的很长 = 42;

/* Many scripts */
int Д = 2;
int ש = 3;
int م = 4;
int अ = 5;
int あ = 6;
int 한 = 7;
int 字 = 8;
int ᚠ = 9;
int Ⴀ = 10;

/* Very long identifier */
int λλλλλλλλλλλλλλλλλλλλλλλλλλλλλλλλλλλλλλλλλλλλλλλλλλλλλλλλλλλλλλλλλλλλλλλλλλλλλλλλλλλλλλλλλλλλλλλλλλλ = 123;

/* Final boss */
int αα = 1;
int аа = 2;          /* Cyrillic */
int ա = 3;           /* Armenian */
int աα = 4;
int ππ = 5;
int λλ = 6;
int 변수 = 7;
int переменная = 8;
int متغير = 9;
int \u03B8 = 10;
int 𐐁 = 11;

int main(void)
{
    printf("Greek: %d\n", γ);

    printf("Mixed scripts: %d\n",
        日本 + привет + Ελληνικά + عربي);

    printf("UCN: %d %d\n", π, λ);

    printf("Supplementary: %d\n", 𐐀);

    printf("Confusable: %d %d\n", a, а);

    printf("Long identifier: %d\n",
        我真的真的真的真的真的真的真的真的真的真的真的真的真的真的真的真的真的真的真的真的很长);

    printf("Many scripts: %d\n",
        α + Д + ש + م + अ +
        あ + 한 + 字 + ᚠ + Ⴀ);

    printf("Long Greek identifier: %d\n",
        λλλλλλλλλλλλλλλλλλλλλλλλλλλλλλλλλλλλλλλλλλλλλλλλλλλλλλλλλλλλλλλλλλλλλλλλλλλλλλλλλλλλλλλλλλλλλλλλλλλ);

    printf("Final boss: %d\n",
        αα +
        аа +
        ա +
        աα +
        ππ +
        λλ +
        변수 +
        переменная +
        متغير +
        θ +
        𐐁);

    return 0;
}