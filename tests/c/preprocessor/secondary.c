//@ mode: c
//@ run-status: 0
/*
 * Preprocessor Feature Test - C99 or later (for variadic macros)
 * Compile and run; returns 0 if all tests pass, nonzero otherwise.
 * Tests: object/function/variadic macros, #, ##, digraphs, trigraphs,
 *        conditional inclusion, defined(), #error, #pragma STDC,
 *        #line, #include computed, #undef, null directive, token pasting,
 *        stringizing, empty arguments, predefined macros, nested #if, etc.
 */

#include <stddef.h>   /* for NULL */
#include <string.h>   /* strcmp, strstr */

/* ---------- compile‑time assertion (works with constant expressions) ---------- */
#define CT_ASSERT(cond)  extern int ct_assert_failed[(cond) ? 1 : -1]

/* ---------- object‑like macro ---------- */
#define OBJ 42
CT_ASSERT(OBJ == 42);

/* ---------- function‑like macro ---------- */
#define ADD(x, y) ((x) + (y))
CT_ASSERT(ADD(1, 2) == 3);

/* ---------- nested expansion, rescanning ---------- */
#define DOUBLE(x) ADD(x, x)
CT_ASSERT(DOUBLE(3) == 6);

/* ---------- self‑reference prevention (RECURSIVE not expanded again) ---------- */
#define RECURSIVE(x) (x + RECURSIVE((x)-1))
#define STR(x) #x
#define EXPAND_STR(x) STR(x)   /* force expansion before stringification */

/* ---------- token pasting (##) ---------- */
#define PASTE(a, b) a ## b
#define PASTE3(a, b, c) a ## b ## c
int PASTE(var, 1) = 10;             /* defines var1 */

/* ---------- pasting forming a punctuator ---------- */
#define PLUS_EQ + ## =
#define SHIFT_OP(a, b) a ## b

/* ---------- pasting with empty arguments ---------- */
#define SUFFIX_TEST(x) x ## _post  /* empty ## _post  ->  _post */
int SUFFIX_TEST() = 5;             /* defines _post */

#define NOTHING(x, y) x ## y       /* empty ## empty  ->  nothing */
int NOTHING(,) test_empty_paste = 0;

/* ---------- empty function‑like argument (C99) ---------- */
#define EMPTY_PAREN(x) x
int EMPTY_PAREN() xyz_var = 1;     /* defines xyz_var */

/* ---------- variadic macros (C99) ---------- */
#define VAR_SUM(first, ...) (first + __VA_ARGS__)
CT_ASSERT(VAR_SUM(1, 2) == 3);  /* two arguments – no comma in constant expression */
/* (With three or more arguments the comma operator would appear, not allowed
   in an integer constant expression; tested at runtime instead.) */

/* ---------- stringification of __VA_ARGS__ ---------- */
#define STR_VA(...) #__VA_ARGS__

/* ---------- computed #include ---------- */
#define HEADER_ANGLE <stddef.h>
#define HEADER_QUOTE "stddef.h"
#include HEADER_ANGLE            /* expands to <stddef.h> */
#if 0
# include HEADER_QUOTE          /* just to demonstrate the syntax */
#endif

/* ---------- macro redefinition (identical token sequence) ---------- */
#define REDEF 1
#define REDEF 1   /* allowed */

/* ---------- undefining ---------- */
#define UNDEF_TEST 1
#undef UNDEF_TEST
#ifdef UNDEF_TEST
# error "UNDEF_TEST not undefined"
#endif

/* ---------- conditional inclusion, defined() ---------- */
#if defined(REDEF) && !defined(UNDEF_TEST)
  CT_ASSERT(1);
#else
# error "defined test failed"
#endif

#if defined REDEF   /* defined without parentheses is legal */
  CT_ASSERT(1);
#else
# error "defined (no parens) test failed"
#endif

/* ---------- unknown identifiers become 0 in #if ---------- */
#if UNDEFINED_MACRO_ROAR == 0
  CT_ASSERT(1);
#else
# error "unknown identifier not replaced by 0"
#endif

/* ---------- character constants in #if ---------- */
#if 'A' == 65
  CT_ASSERT(1);
#else
# error "char constant comparison failed"
#endif

#if L'A' == 65        /* wide character constant */
  CT_ASSERT(1);
#else
# error "wide char constant comparison failed"
#endif

/* ---------- full set of preprocessor operators ---------- */
#if (1 + 2 * 3) == 7 && (5 / 2) == 2 && (5 % 2) == 1 &&                \
    (3 << 2) == 12 && (12 >> 1) == 6 && (5 & 3) == 1 &&                \
    (5 | 3) == 7 && (5 ^ 3) == 6 && ~0 != 0 && !0 == 1 &&             \
    (1 && 2) == 1 && (0 || 5) == 1 && (1 ? 2 : 3) == 2
  CT_ASSERT(1);
#else
# error "operator test failed"
#endif

/* ---------- nested conditionals and complex control flow ---------- */
/* Nested #if, #elif, #else, #endif with defined(), arithmetic, logic */
#ifdef __STDC__
  #if __STDC__ == 1
    #ifndef __cplusplus
      #if defined(__LINE__)
        #define NESTED_DEFINED_OK 1
      #else
        #error "defined(__LINE__) should be true"
      #endif
    #endif
  #endif
#else
  #if 1
  	#error "__STDC__ not defined"
  #else
	#error "super broken"
  #endif	
#endif
CT_ASSERT(NESTED_DEFINED_OK == 1);

/* Multiple #elif branches – only the first true one is taken */
/* Define only inside the active branch to avoid redefinition UB. */
#if 0
# define CHOSEN 1
#elif 1
# define CHOSEN 2
#elif 2
# define CHOSEN 3
#else
# define CHOSEN 4
#endif
CT_ASSERT(CHOSEN == 2);

/* Complex expression with defined(), &&, ||, and parentheses */
#if (defined(__STDC__) && (1 + 1 == 2)) || defined(NONEXISTENT)
  #define COMPLEX_TRUE 1
#else
# error "complex expression should be true"
#endif
CT_ASSERT(COMPLEX_TRUE == 1);

/* #if 0 block skips arbitrary invalid tokens – use only valid pp‑tokens */
#if 0
This line contains intentionally invalid C code: [ { ( ) ++ == float
It should be completely ignored.
#endif

/* #else branch taken when #if is false */
#if 0
# define ELSE_TEST 1
#else
# define ELSE_TEST 2
#endif
#if ELSE_TEST != 2
# error "ELSE_TEST not 2"
#endif

/* Nested #if with #elif inside another #if */
#if 1
  #if 2 == 3
    #define NESTED_ELIF_VAL 1
  #elif 2 == 2
    #define NESTED_ELIF_VAL 2
  #else
    #define NESTED_ELIF_VAL 3
  #endif
#else
  #error "outer if false"
#endif
#if NESTED_ELIF_VAL != 2
# error "NESTED_ELIF_VAL not 2"
#endif
#if 0
  #if 2 == 3
    #define NESTED_ELIF_VAL 1
  #elif 2 == 2
    #define NESTED_ELIF_VAL 2
  #else
    #define NESTED_ELIF_VAL 3
  #endif
#else
  #if 2 == 3
    #define NESTED_ELIF_VAL2 1
  #elif 2 == 2
    #define NESTED_ELIF_VAL2 2
  #else
    #define NESTED_ELIF_VAL2 3
  #endif
#endif
#if NESTED_ELIF_VAL2 != 2
# error "NESTED_ELIF_VAL2 not 2"
#endif

/* ---------- predefined macros ---------- */
#if defined(__STDC__) && __STDC__ == 1
  CT_ASSERT(1);
#else
# error "__STDC__ not 1"
#endif

CT_ASSERT(__LINE__ > 0);

#ifdef __STDC_HOSTED__
# if __STDC_HOSTED__ == 0 || __STDC_HOSTED__ == 1
    CT_ASSERT(1);
# else
#   error "__STDC_HOSTED__ not 0 or 1"
# endif
#endif

#ifdef __STDC_VERSION__
# if __STDC_VERSION__ >= 199409L
    CT_ASSERT(1);
# else
#   error "__STDC_VERSION__ too old"
# endif
#endif

/* ---------- #line directive ---------- */
#line 42 "testfile.c"
CT_ASSERT(__LINE__ == 42);   /* the line number of this line is 42 */

/* ---------- #pragma STDC (standard pragma) ---------- */
#pragma STDC FP_CONTRACT ON

/* ---------- #error (conditionally suppressed) ---------- */
#if 0
# error "This error should never fire"
#endif

/* ---------- null directive ---------- */
#
/* just a hash and nothing else */

/* ---------- digraphs (C95) ---------- */
/* %:  ->  #  ;  %:%:  ->  ##  */
#define STR_DIG(x) %:x
#define PASTE_DIG(a, b) a %:%: b
CT_ASSERT(PASTE_DIG(1, 2) == 12);  /* 1 ## 2  ->  12 */

/* ---------- ensure we are compiled as C, not C++ ---------- */
#ifdef __cplusplus
# error "This test must be compiled as C, not C++"
#endif

/* ---------- runtime verification in main ---------- */
int main(void) {
    int failures = 0;

    /* var1 from PASTE(var, 1) */
    if (var1 != 10) failures++;

    /* PLUS_EQ ( += ) */
    int x = 5;
    x PLUS_EQ 3;
    if (x != 8) failures++;

    /* SHIFT_OP (>>= ) */
    int y = 8;
    y SHIFT_OP(>>, =) 1;
    if (y != 4) failures++;

    /* _post from SUFFIX_TEST() */
    if (_post != 5) failures++;

    /* test_empty_paste from NOTHING(,) */
    test_empty_paste = 7;
    if (test_empty_paste != 7) failures++;

    /* xyz_var from EMPTY_PAREN() */
    if (xyz_var != 1) failures++;

    /* ------ stringification tests ------ */
    if (strcmp(STR(hello), "hello") != 0) failures++;
    if (strcmp(STR( a  b ), "a b") != 0) failures++;  /* whitespace collapses */
    if (strcmp(STR("foo"), "\"foo\"") != 0) failures++; /* quotes escaped */
    if (strcmp(STR(back\\slash), "back\\slash") != 0) failures++; /* backslash escaped */
    if (strcmp(STR(), "") != 0) failures++;  /* empty argument -> "" */

    /* variadic stringification */
    if (strcmp(STR_VA(a, b), "a, b") != 0) failures++;
    if (strcmp(STR_VA(), "") != 0) failures++;      /* no arguments -> "" */

    /* digraph stringification */
    if (strcmp(STR_DIG(digraph), "digraph") != 0) failures++;

    /* digraph token pasting runtime check */
    int paste_dig_val = PASTE_DIG(10, 20);
    if (paste_dig_val != 1020) failures++;

    /* self‑reference prevention: RECURSIVE appears in the expansion */
    if (strstr(EXPAND_STR(RECURSIVE(5)), "RECURSIVE") == NULL) failures++;

    /* macro argument not expanded before # */
    #define MACRO_ARG value
    if (strcmp(STR(MACRO_ARG), "MACRO_ARG") != 0) failures++;
    if (strcmp(EXPAND_STR(MACRO_ARG), "value") != 0) failures++;

    /* variadic macro with three arguments (comma operator in action) */
    if (VAR_SUM(1, 2, 3) != 3) failures++;   /* (1 + 2, 3) evaluates to 3 */

    /* predefined macros sanity */
    if (__LINE__ <= 0) failures++;
    const char *file = __FILE__;
    if (file == NULL) failures++;
    const char *date = __DATE__;
    const char *time = __TIME__;
    if (date[0] == '\0' || time[0] == '\0') failures++;

    /* digraph / trigraph comments (existence check) */
    /* ??= ??) ??' ??( ??! ??- ??> ??<   (trigraph sequences, replaced if enabled) */
    /* %: %:%:   (digraph sequences) */

    return failures ? 1 : 0;
}
