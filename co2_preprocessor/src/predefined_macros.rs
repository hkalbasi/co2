//! Predefined macros and target configuration.
//!
//! Contains the predefined macro tables (standard C, platform, GCC compat,
//! type limits, float characteristics, etc.) and architecture-specific
//! setup for aarch64 and riscv64.

use super::macro_defs::macro_def_from_parts;
use super::pipeline::Preprocessor;

impl Preprocessor {
    /// Define standard predefined macros.
    ///
    /// Object-like macros are defined via a static table to keep this compact.
    /// Function-like macros (with parameters) are defined individually below.
    pub(super) fn define_predefined_macros(&mut self) {
        // All object-like predefined macros as (name, body) pairs.
        // Grouped by category; order matches GCC's predefined macro output.
        const PREDEFINED_OBJECT_MACROS: &[(&str, &str)] = &[
            // Standard C
            ("__STDC__", "1"),
            ("__STDC_VERSION__", "202311L"), // C23
            ("__STDC_HOSTED__", "1"),
            ("__STDC_VERSION_UCHAR_H__", "202311L"),
            ("__STDC_VERSION_STDCKDINT_H__", "202311L"),
            ("__STDC_VERSION_STDBIT_H__", "202311L"),
            ("__STDC_ENDIAN_LITTLE__", "1234"),
            ("__STDC_ENDIAN_BIG__", "4321"),
            ("__STDC_ENDIAN_NATIVE__", "1234"),
            ("nullptr", "((void*)0)"),
            // Platform
            ("__linux__", "1"),
            ("__linux", "1"),
            ("linux", "1"),
            ("__gnu_linux__", "1"),
            ("__unix__", "1"),
            ("__unix", "1"),
            ("unix", "1"),
            ("__LP64__", "1"),
            ("_LP64", "1"),
            // Default arch: x86_64 (overridden by set_target)
            ("__x86_64__", "1"),
            ("__x86_64", "1"),
            ("__amd64__", "1"),
            ("__amd64", "1"),
            // GCC compat: claim GCC 14.2.0. This is:
            //  - >= 5.1 (Linux kernel minimum requirement)
            //  - >= 7.4 (QEMU minimum requirement)
            //  - A modern version that satisfies most project requirements.
            // For glibc's _Float* types (expected native for GCC >= 7), we define
            // them as macros mapping to standard C types below.
            ("__GNUC__", "14"),
            ("__GNUC_MINOR__", "2"),
            ("__GNUC_PATCHLEVEL__", "0"),
            ("__VERSION__", "\"14.2.0\""),
            // C99 inline semantics: tells gnulib and other libraries that
            // plain `inline` provides an inline definition only (no external symbol).
            ("__GNUC_STDC_INLINE__", "1"),
            // sizeof macros
            ("__SIZEOF_POINTER__", "8"),
            ("__SIZEOF_INT__", "4"),
            ("__SIZEOF_LONG__", "8"),
            ("__SIZEOF_LONG_LONG__", "8"),
            ("__SIZEOF_SHORT__", "2"),
            ("__SIZEOF_FLOAT__", "4"),
            ("__SIZEOF_DOUBLE__", "8"),
            ("__SIZEOF_SIZE_T__", "8"),
            ("__SIZEOF_PTRDIFF_T__", "8"),
            ("__SIZEOF_WCHAR_T__", "4"),
            ("__SIZEOF_INT128__", "16"),
            ("__SIZEOF_WINT_T__", "4"),
            // Byte order
            ("__BYTE_ORDER__", "__ORDER_LITTLE_ENDIAN__"),
            ("__ORDER_LITTLE_ENDIAN__", "1234"),
            ("__ORDER_BIG_ENDIAN__", "4321"),
            // Type limits
            ("__CHAR_BIT__", "8"),
            ("__INT_MAX__", "2147483647"),
            ("__LONG_MAX__", "9223372036854775807L"),
            ("__LONG_LONG_MAX__", "9223372036854775807LL"),
            ("__SCHAR_MAX__", "127"),
            ("__SHRT_MAX__", "32767"),
            ("__SIZE_MAX__", "18446744073709551615UL"),
            ("__PTRDIFF_MAX__", "9223372036854775807L"),
            ("__WCHAR_MAX__", "2147483647"),
            ("__WCHAR_MIN__", "(-2147483647-1)"),
            ("__WINT_MAX__", "4294967295U"),
            ("__WINT_MIN__", "0U"),
            ("__SIG_ATOMIC_MAX__", "2147483647"),
            ("__SIG_ATOMIC_MIN__", "(-2147483647-1)"),
            // Type names
            ("__SIZE_TYPE__", "long unsigned int"),
            ("__PTRDIFF_TYPE__", "long int"),
            ("__WCHAR_TYPE__", "int"),
            ("__WINT_TYPE__", "unsigned int"),
            ("__CHAR16_TYPE__", "short unsigned int"),
            ("__CHAR32_TYPE__", "unsigned int"),
            ("__INTMAX_TYPE__", "long int"),
            ("__UINTMAX_TYPE__", "long unsigned int"),
            ("__INT8_TYPE__", "signed char"),
            ("__INT16_TYPE__", "short int"),
            ("__INT32_TYPE__", "int"),
            ("__INT64_TYPE__", "long int"),
            ("__UINT8_TYPE__", "unsigned char"),
            ("__UINT16_TYPE__", "unsigned short int"),
            ("__UINT32_TYPE__", "unsigned int"),
            ("__UINT64_TYPE__", "long unsigned int"),
            ("__INTPTR_TYPE__", "long int"),
            ("__UINTPTR_TYPE__", "long unsigned int"),
            ("__INT_LEAST8_TYPE__", "signed char"),
            ("__INT_LEAST16_TYPE__", "short int"),
            ("__INT_LEAST32_TYPE__", "int"),
            ("__INT_LEAST64_TYPE__", "long int"),
            ("__UINT_LEAST8_TYPE__", "unsigned char"),
            ("__UINT_LEAST16_TYPE__", "unsigned short int"),
            ("__UINT_LEAST32_TYPE__", "unsigned int"),
            ("__UINT_LEAST64_TYPE__", "long unsigned int"),
            ("__INT_FAST8_TYPE__", "signed char"),
            ("__INT_FAST16_TYPE__", "long int"),
            ("__INT_FAST32_TYPE__", "long int"),
            ("__INT_FAST64_TYPE__", "long int"),
            ("__UINT_FAST8_TYPE__", "unsigned char"),
            ("__UINT_FAST16_TYPE__", "long unsigned int"),
            ("__UINT_FAST32_TYPE__", "unsigned int"),
            ("__UINT_FAST64_TYPE__", "long unsigned int"),
            // FLT characteristics
            ("__FLT_MANT_DIG__", "24"),
            ("__FLT_DIG__", "6"),
            ("__FLT_MIN_EXP__", "(-125)"),
            ("__FLT_MIN_10_EXP__", "(-37)"),
            ("__FLT_MAX_EXP__", "128"),
            ("__FLT_MAX_10_EXP__", "38"),
            ("__FLT_MAX__", "3.40282346638528859811704183484516925e+38F"),
            ("__FLT_MIN__", "1.17549435082228750796873653722224568e-38F"),
            (
                "__FLT_EPSILON__",
                "1.19209289550781250000000000000000000e-7F",
            ),
            ("__FLT_RADIX__", "2"),
            (
                "__FLT_DENORM_MIN__",
                "1.40129846432481707092372958328991613e-45F",
            ),
            // DBL characteristics
            ("__DBL_MANT_DIG__", "53"),
            ("__DBL_DIG__", "15"),
            ("__DBL_MIN_EXP__", "(-1021)"),
            ("__DBL_MIN_10_EXP__", "(-307)"),
            ("__DBL_MAX_EXP__", "1024"),
            ("__DBL_MAX_10_EXP__", "308"),
            ("__DBL_MAX__", "1.79769313486231570814527423731704357e+308"),
            ("__DBL_MIN__", "2.22507385850720138309023271733240406e-308"),
            (
                "__DBL_EPSILON__",
                "2.22044604925031308084726333618164062e-16",
            ),
            (
                "__DBL_DENORM_MIN__",
                "4.94065645841246544176568792868221372e-324",
            ),
            // LDBL characteristics
            ("__LDBL_MANT_DIG__", "64"),
            ("__LDBL_DIG__", "18"),
            ("__LDBL_MIN_EXP__", "(-16381)"),
            ("__LDBL_MIN_10_EXP__", "(-4931)"),
            ("__LDBL_MAX_EXP__", "16384"),
            ("__LDBL_MAX_10_EXP__", "4932"),
            (
                "__LDBL_MAX__",
                "1.18973149535723176502126385303097021e+4932L",
            ),
            (
                "__LDBL_MIN__",
                "3.36210314311209350626267781732175260e-4932L",
            ),
            (
                "__LDBL_EPSILON__",
                "1.08420217248550443400745280086994171e-19L",
            ),
            (
                "__LDBL_DENORM_MIN__",
                "3.64519953188247460252840593361941982e-4951L",
            ),
            ("__SIZEOF_LONG_DOUBLE__", "16"),
            // Float feature flags
            ("__FLT_HAS_INFINITY__", "1"),
            ("__FLT_HAS_QUIET_NAN__", "1"),
            ("__FLT_HAS_DENORM__", "1"),
            ("__DBL_HAS_INFINITY__", "1"),
            ("__DBL_HAS_QUIET_NAN__", "1"),
            ("__DBL_HAS_DENORM__", "1"),
            ("__LDBL_HAS_INFINITY__", "1"),
            ("__LDBL_HAS_QUIET_NAN__", "1"),
            ("__LDBL_HAS_DENORM__", "1"),
            ("__FLT_DECIMAL_DIG__", "9"),
            ("__DBL_DECIMAL_DIG__", "17"),
            ("__LDBL_DECIMAL_DIG__", "21"),
            ("__DECIMAL_DIG__", "21"),
            // GCC extensions
            ("__GNUC_VA_LIST", "1"),
            ("__extension__", ""),
            // NOTE: GNU keyword aliases (__inline__, __volatile__, __asm__, __const__,
            // __restrict__, __signed__, __typeof__) are handled as keyword tokens in
            // the lexer (token.rs), not as macros, because GCC treats them as reserved
            // keywords immune to #define redefinition.
            // __alignof/__alignof__ are handled as keyword tokens (GnuAlignof)
            // in the lexer, not as macros - they return preferred alignment,
            // which differs from C11 _Alignof on i686.
            // Named address spaces (Linux kernel): __seg_gs/__seg_fs are handled
            // as keyword tokens in the lexer (token.rs), not as macros.
            // __float128 -> long double (glibc compat)
            ("__float128", "long double"),
            ("__SIZEOF_FLOAT128__", "16"),
            // _Float* types: For GCC >= 7, glibc expects the compiler to provide these
            // natively. We define them as macros to the corresponding standard C types.
            // TODO: Implement _Float* as proper builtin types with correct semantics
            // (e.g., _Float128 should be true IEEE binary128, not 80-bit long double
            // on x86-64). The macro approach works for glibc header compatibility but
            // loses precision for _Float128 operations on x86-64.
            ("_Float128", "long double"),
            ("_Float32", "float"),
            ("_Float64", "double"),
            ("_Float32x", "double"),
            ("_Float64x", "long double"),
            // MSVC integer type specifiers
            ("__int8", "char"),
            ("__int16", "short"),
            ("__int32", "int"),
            ("__int64", "long long"),
            // ELF ABI
            ("__USER_LABEL_PREFIX__", ""),
            // GNU C attribute macros (strip)
            ("__LEAF", ""),
            ("__LEAF_ATTR", ""),
            ("__wur", ""),
            // Date/time
            ("__DATE__", "\"Jan  1 2025\""),
            ("__TIME__", "\"00:00:00\""),
            // GCC atomic lock-free macros
            ("__GCC_ATOMIC_BOOL_LOCK_FREE", "2"),
            ("__GCC_ATOMIC_CHAR_LOCK_FREE", "2"),
            ("__GCC_ATOMIC_CHAR16_T_LOCK_FREE", "2"),
            ("__GCC_ATOMIC_CHAR32_T_LOCK_FREE", "2"),
            ("__GCC_ATOMIC_WCHAR_T_LOCK_FREE", "2"),
            ("__GCC_ATOMIC_SHORT_LOCK_FREE", "2"),
            ("__GCC_ATOMIC_INT_LOCK_FREE", "2"),
            ("__GCC_ATOMIC_LONG_LOCK_FREE", "2"),
            ("__GCC_ATOMIC_LLONG_LOCK_FREE", "2"),
            ("__GCC_ATOMIC_POINTER_LOCK_FREE", "2"),
            // ELF
            ("__ELF__", "1"),
            // Note: __PIC__/__pic__ are conditionally defined via set_pic(),
            // not here, so they are only present when -fPIC is active.
            // CET (Control-flow Enforcement Technology) - match GCC's default
            // This is x86_64-only; removed for other targets in set_target().
            // Value 3 = IBT (bit 0) + SHSTK (bit 1), matching GCC's default.
            // Critical: must match the GCC that assembles .S files, because
            // libffi's trampoline sizes depend on ENDBR_PRESENT which checks __CET__.
            ("__CET__", "3"),
            // SSE/MMX feature macros: SSE2 is baseline for x86_64.
            // Removed for non-x86_64 targets in set_target().
            // Many projects (dr_libs, minimp3, stb_image, etc.) use #ifdef __SSE2__
            // to enable SIMD code paths.
            ("__SSE__", "1"),
            ("__SSE2__", "1"),
            ("__MMX__", "1"),
            ("__SSE_MATH__", "1"),
            ("__SSE2_MATH__", "1"),
            // Pragma support flags
            ("__PRAGMA_REDEFINE_EXTNAME", "1"),
        ];

        // Function-like predefined macros: (name, params, body)
        // Note: __builtin_expect is handled as a real builtin (not a macro)
        // to properly evaluate side effects in the second argument.
        // __builtin_offsetof is also handled by the lexer/parser directly so
        // static initializers can lower it without preprocessor macro expansion.
        const PREDEFINED_FUNC_MACROS: &[(&str, &[&str], &str)] = &[
            // __has_builtin, __has_attribute, __has_feature, __has_extension,
            // __has_include, and __has_include_next are NOT defined as macros.
            // They are handled as special preprocessor operators:
            // - #ifdef checks use is_defined() which special-cases them
            // - #if evaluation uses resolve_defined_in_expr() in expr_eval.rs
            // C23 checked arithmetic: map __builtin_*_overflow to a statement
            // expression that uses __int128 for overflow detection. The
            // 3-argument form __builtin_add_overflow(a,b,&res) is what
            // <stdckdint.h> expands ckd_add etc. to.
            (
                "__builtin_add_overflow",
                &["a", "b", "c"],
                "({ __typeof__(a) __a = (a); __typeof__(b) __b = (b); __typeof__(*c) *__p = (c); long long __a128 = __a; long long __b128 = __b; *__p = (__typeof__(*__p))((unsigned long long)__a + (unsigned long long)__b); long long __p128 = *__p; (__a128 + __b128 != __p128); })",
            ),
            (
                "__builtin_sub_overflow",
                &["a", "b", "c"],
                "({ __typeof__(a) __a = (a); __typeof__(b) __b = (b); __typeof__(*c) *__p = (c); long long __a128 = __a; long long __b128 = __b; *__p = (__typeof__(*__p))((unsigned long long)__a - (unsigned long long)__b); long long __p128 = *__p; (__a128 - __b128 != __p128); })",
            ),
            (
                "__builtin_mul_overflow",
                &["a", "b", "c"],
                "({ __typeof__(a) __a = (a); __typeof__(b) __b = (b); __typeof__(*c) *__p = (c); long long __a128 = __a; long long __b128 = __b; *__p = (__typeof__(*__p))((unsigned long long)__a * (unsigned long long)__b); long long __p128 = *__p; (__a128 * __b128 != __p128); })",
            ),
            (
                "__builtin_trap",
                &[],
                "({ extern void abort(void); (abort)(); })",
            ),
        ];

        // Library builtins (https://gcc.gnu.org/onlinedocs/gcc/Library-Builtins.html):
        // `__builtin_<name>(args)` forwards to the libc function, declaring it
        // locally so no header is needed. The loop below generates one macro
        // per table row; the declaration and call names are parenthesized so
        // header macros (fortify wrappers, ctype tables, glibc redirects)
        // cannot hijack the expansion.
        // Deliberately absent: alloca (stack semantics can't forward), the
        // v*printf/v*scanf family (no header-free va_list spelling that is
        // ABI-correct on all targets), and names already handled elsewhere
        // (__builtin_expect, __builtin_constant_p,
        // __builtin_types_compatible_p, __builtin_inf/nan/huge_val,
        // __builtin_va_*, __builtin_offsetof, __builtin_*_overflow, and the
        // fp-comparison/classification builtins defined just below).
        const LIBC_FORWARD: &[(&str, &str, &[&str])] = &[
            ("abort", "void", &[]),
            ("abs", "int", &["int"]),
            (
                "bcmp",
                "int",
                &["const void *", "const void *", "unsigned long"],
            ),
            ("bzero", "void", &["void *", "unsigned long"]),
            ("calloc", "void *", &["unsigned long", "unsigned long"]),
            (
                "dcgettext",
                "char *",
                &["const char *", "const char *", "int"],
            ),
            ("dgettext", "char *", &["const char *", "const char *"]),
            ("exit", "void", &["int"]),
            ("_Exit", "void", &["int"]),
            ("_exit", "void", &["int"]),
            ("feclearexcept", "int", &["int"]),
            ("fegetround", "int", &[]),
            ("feraiseexcept", "int", &["int"]),
            ("ffs", "int", &["int"]),
            ("ffsl", "int", &["long"]),
            ("ffsll", "int", &["long long"]),
            ("fprintf", "int", &["void *", "const char *", "..."]),
            (
                "fprintf_unlocked",
                "int",
                &["void *", "const char *", "..."],
            ),
            ("fputs", "int", &["const char *", "void *"]),
            ("fputs_unlocked", "int", &["const char *", "void *"]),
            ("fscanf", "int", &["void *", "const char *", "..."]),
            ("free", "void", &["void *"]),
            ("frexp", "double", &["double", "int *"]),
            ("frexpf", "float", &["float", "int *"]),
            ("gamma_r", "double", &["double", "int *"]),
            ("gammaf_r", "float", &["float", "int *"]),
            ("gettext", "char *", &["const char *"]),
            ("jn", "double", &["int", "double"]),
            ("jnf", "float", &["int", "float"]),
            ("yn", "double", &["int", "double"]),
            ("ynf", "float", &["int", "float"]),
            ("imaxabs", "long", &["long"]),
            ("index", "char *", &["const char *", "int"]),
            ("isalnum", "int", &["int"]),
            ("isalpha", "int", &["int"]),
            ("isascii", "int", &["int"]),
            ("isblank", "int", &["int"]),
            ("iscntrl", "int", &["int"]),
            ("isdigit", "int", &["int"]),
            ("isgraph", "int", &["int"]),
            ("islower", "int", &["int"]),
            ("isprint", "int", &["int"]),
            ("ispunct", "int", &["int"]),
            ("isspace", "int", &["int"]),
            ("isupper", "int", &["int"]),
            ("iswalnum", "int", &["int"]),
            ("iswalpha", "int", &["int"]),
            ("iswblank", "int", &["int"]),
            ("iswcntrl", "int", &["int"]),
            ("iswdigit", "int", &["int"]),
            ("iswgraph", "int", &["int"]),
            ("iswlower", "int", &["int"]),
            ("iswprint", "int", &["int"]),
            ("iswpunct", "int", &["int"]),
            ("iswspace", "int", &["int"]),
            ("iswupper", "int", &["int"]),
            ("iswxdigit", "int", &["int"]),
            ("isxdigit", "int", &["int"]),
            ("labs", "long", &["long"]),
            ("ldexp", "double", &["double", "int"]),
            ("ldexpf", "float", &["float", "int"]),
            ("lgamma_r", "double", &["double", "int *"]),
            ("lgammaf_r", "float", &["float", "int *"]),
            ("llabs", "long long", &["long long"]),
            ("malloc", "void *", &["unsigned long"]),
            (
                "memchr",
                "void *",
                &["const void *", "int", "unsigned long"],
            ),
            (
                "memcmp",
                "int",
                &["const void *", "const void *", "unsigned long"],
            ),
            (
                "memcpy",
                "void *",
                &["void *", "const void *", "unsigned long"],
            ),
            (
                "mempcpy",
                "void *",
                &["void *", "const void *", "unsigned long"],
            ),
            ("memset", "void *", &["void *", "int", "unsigned long"]),
            ("modf", "double", &["double", "double *"]),
            ("modff", "float", &["float", "float *"]),
            ("printf", "int", &["const char *", "..."]),
            ("printf_unlocked", "int", &["const char *", "..."]),
            ("putchar", "int", &["int"]),
            ("puts", "int", &["const char *"]),
            ("realloc", "void *", &["void *", "unsigned long"]),
            ("remquo", "double", &["double", "double", "int *"]),
            ("remquof", "float", &["float", "float", "int *"]),
            ("rindex", "char *", &["const char *", "int"]),
            ("scanf", "int", &["const char *", "..."]),
            ("sincos", "void", &["double", "double", "double *"]),
            ("sincosf", "void", &["float", "float", "float *"]),
            (
                "snprintf",
                "int",
                &["char *", "unsigned long", "const char *", "..."],
            ),
            ("sprintf", "int", &["char *", "const char *", "..."]),
            ("sscanf", "int", &["const char *", "const char *", "..."]),
            ("stpcpy", "char *", &["char *", "const char *"]),
            (
                "stpncpy",
                "char *",
                &["char *", "const char *", "unsigned long"],
            ),
            ("strcasecmp", "int", &["const char *", "const char *"]),
            ("strcat", "char *", &["char *", "const char *"]),
            ("strchr", "char *", &["const char *", "int"]),
            ("strcmp", "int", &["const char *", "const char *"]),
            ("strcpy", "char *", &["char *", "const char *"]),
            (
                "strcspn",
                "unsigned long",
                &["const char *", "const char *"],
            ),
            ("strdup", "char *", &["const char *"]),
            (
                "strfmon",
                "long",
                &["char *", "unsigned long", "const char *", "..."],
            ),
            ("strlen", "unsigned long", &["const char *"]),
            (
                "strncasecmp",
                "int",
                &["const char *", "const char *", "unsigned long"],
            ),
            (
                "strncat",
                "char *",
                &["char *", "const char *", "unsigned long"],
            ),
            (
                "strncmp",
                "int",
                &["const char *", "const char *", "unsigned long"],
            ),
            (
                "strncpy",
                "char *",
                &["char *", "const char *", "unsigned long"],
            ),
            ("strndup", "char *", &["const char *", "unsigned long"]),
            (
                "strnlen",
                "unsigned long",
                &["const char *", "unsigned long"],
            ),
            ("strpbrk", "char *", &["const char *", "const char *"]),
            ("strrchr", "char *", &["const char *", "int"]),
            ("strspn", "unsigned long", &["const char *", "const char *"]),
            ("strstr", "char *", &["const char *", "const char *"]),
            ("toascii", "int", &["int"]),
            ("tolower", "int", &["int"]),
            ("toupper", "int", &["int"]),
            ("towlower", "int", &["int"]),
            ("towupper", "int", &["int"]),
            ("scalbln", "double", &["double", "long"]),
            ("scalblnf", "float", &["float", "long"]),
            ("scalbn", "double", &["double", "int"]),
            ("scalbnf", "float", &["float", "int"]),
        ];
        // Same-type math families: base name + ""/f/l suffix selects the
        // double/float/long-double spelling of both the builtin and the libc
        // function (e.g. __builtin_sin -> sin(double), __builtin_sinf ->
        // sinf(float)).
        const FP_SUFFIXES: &[(&str, &str)] = &[("", "double"), ("f", "float")];
        // NOTE: no "l" (long double) rows: passing/returning long double
        // across a real libc call miscompiles in the backend today (plain
        // ceill(1.5L) already returns garbage), so generating those would
        // bless silent wrong results. Pure-comparison builtins below keep
        // their long-double behavior, which is sound.
        // (T) -> T
        const MATH_U1: &[&str] = &[
            "acos",
            "acosh",
            "asin",
            "asinh",
            "atan",
            "atanh",
            "cbrt",
            "ceil",
            "cos",
            "cosh",
            "drem",
            "erf",
            "erfc",
            "exp",
            "exp10",
            "exp2",
            "expm1",
            "fabs",
            "floor",
            "gamma",
            "j0",
            "j1",
            "lgamma",
            "log",
            "log10",
            "log1p",
            "log2",
            "logb",
            "nearbyint",
            "pow10",
            "rint",
            "round",
            "roundeven",
            "scalb",
            "significand",
            "sin",
            "sinh",
            "sqrt",
            "tan",
            "tanh",
            "tgamma",
            "trunc",
            "y0",
            "y1",
        ];
        // (T, T) -> T
        const MATH_B2: &[&str] = &[
            "atan2",
            "atan2pi",
            "copysign",
            "fdim",
            "fmax",
            "fmin",
            "fmod",
            "hypot",
            "nextafter",
            "pow",
            "remainder",
        ];
        // (T, T, T) -> T
        const MATH_T3: &[&str] = &["fma"];
        // (T) -> int / long / long long
        const MATH_TO_INT: &[&str] = &["ilogb"];
        const MATH_TO_LONG: &[&str] = &["lrint", "lround"];
        const MATH_TO_LLONG: &[&str] = &["llrint", "llround"];
        // NOTE: no complex-math family (cacos/cpow/cabs/...): co2cc's complex
        // calling convention is broken independently (plain ccosh(0.0) already
        // returns a garbage imaginary part, and imaginary literals like 1.0fi
        // don't parse), so forwarding would bless miscompiles. Re-add the
        // "c*"+"" / f / l rows when complex calls are sound.

        // Collect every (libc name, return type, param types) row, then emit
        // one forwarding macro per row in the loop below.
        let mut rows: Vec<(String, String, Vec<String>)> = Vec::new();
        for &(name, ret, params) in LIBC_FORWARD {
            rows.push((
                name.to_string(),
                ret.to_string(),
                params.iter().map(|s| s.to_string()).collect(),
            ));
        }
        for &(suffix, ty) in FP_SUFFIXES {
            for base in MATH_U1 {
                rows.push((
                    format!("{base}{suffix}"),
                    ty.to_string(),
                    vec![ty.to_string()],
                ));
            }
            for base in MATH_B2 {
                rows.push((
                    format!("{base}{suffix}"),
                    ty.to_string(),
                    vec![ty.to_string(), ty.to_string()],
                ));
            }
            for base in MATH_T3 {
                rows.push((
                    format!("{base}{suffix}"),
                    ty.to_string(),
                    vec![ty.to_string(), ty.to_string(), ty.to_string()],
                ));
            }
            for base in MATH_TO_INT {
                rows.push((
                    format!("{base}{suffix}"),
                    "int".to_string(),
                    vec![ty.to_string()],
                ));
            }
            for base in MATH_TO_LONG {
                rows.push((
                    format!("{base}{suffix}"),
                    "long".to_string(),
                    vec![ty.to_string()],
                ));
            }
            for base in MATH_TO_LLONG {
                rows.push((
                    format!("{base}{suffix}"),
                    "long long".to_string(),
                    vec![ty.to_string()],
                ));
            }
        }
        for (name, ret, params) in rows {
            let variadic = params.last().is_some_and(|p| p == "...");
            let fixed: Vec<String> = params
                .iter()
                .filter(|p| *p != "...")
                .enumerate()
                .map(|(i, _)| format!("p{i}"))
                .collect();
            let mut call_args = fixed.clone();
            if variadic {
                call_args.push("__VA_ARGS__".to_string());
            }
            let decl_params = if params.is_empty() {
                "(void)".to_string()
            } else {
                format!("({})", params.join(", "))
            };
            let body = format!(
                "({{ extern {ret} ({name}){decl_params}; ({name})({}); }})",
                call_args.join(", ")
            );
            self.macros.define(macro_def_from_parts(
                format!("__builtin_{name}"),
                true,
                fixed,
                variadic,
                false,
                body,
            ));
        }

        // FP comparison/classification builtins from the same Library-Builtins
        // page. These have no libc function to forward to (several exist only
        // as header macros), so they expand to comparisons instead. Every
        // argument is evaluated exactly once; `__builtin_inf()` is a real
        // parser token, so these work with or without <math.h>. Value-correct
        // for NaN/inf/-0 in all float widths (promotion makes the double
        // spelling exact); only `isnormal`/`fpclassify` need a width-specific
        // threshold, selected with _Generic.
        const FP_CMP: &[(&str, &[&str], &str)] = &[
            ("__builtin_isgreater", &["x", "y"], "((x) > (y))"),
            ("__builtin_isgreaterequal", &["x", "y"], "((x) >= (y))"),
            ("__builtin_isless", &["x", "y"], "((x) < (y))"),
            ("__builtin_islessequal", &["x", "y"], "((x) <= (y))"),
            ("__builtin_islessgreater", &["x", "y"], "((x) != (y))"),
            ("__builtin_iseqsig", &["x", "y"], "((x) == (y))"),
            (
                "__builtin_isunordered",
                &["x", "y"],
                "({ __typeof__(x) __x = (x); __typeof__(y) __y = (y); ((__x != __x) || (__y != __y)); })",
            ),
        ];
        for &(name, params, body) in FP_CMP {
            self.macros.define(macro_def_from_parts(
                name.to_string(),
                true,
                params.iter().map(|s| s.to_string()).collect(),
                false,
                false,
                body.to_string(),
            ));
        }
        // (suffix, min-macro) per float width for the is*/signbit family.
        const FP_WIDTHS: &[(&str, &str)] = &[
            ("", "__DBL_MIN__"),
            ("f", "__FLT_MIN__"),
            ("l", "__LDBL_MIN__"),
        ];
        for &(suffix, min) in FP_WIDTHS {
            let defs = [
                (
                    format!("__builtin_isinf{suffix}"),
                    format!(
                        "({{ __typeof__(x) __v = (x); ((__v == __builtin_inf()) || (__v == -__builtin_inf())); }})"
                    ),
                ),
                (
                    format!("__builtin_isnan{suffix}"),
                    "({ __typeof__(x) __v = (x); ((__v != __v)); })".to_string(),
                ),
                (
                    format!("__builtin_isfinite{suffix}"),
                    format!(
                        "({{ __typeof__(x) __v = (x); ((__v != __builtin_inf()) && (__v != -__builtin_inf())); }})"
                    ),
                ),
                (
                    format!("__builtin_isnormal{suffix}"),
                    format!(
                        "({{ __typeof__(x) __v = (x); ((__v == __v) && ((__v != 0) && ((__v != __builtin_inf()) && ((__v != -__builtin_inf()) && (((__v >= {min}) || (__v <= -{min}))))))); }})"
                    ),
                ),
                (
                    format!("__builtin_signbit{suffix}"),
                    "({ __typeof__(x) __v = (x); (((__v < 0) || ((__v == 0) && (((1.0 / __v)) < 0)))); })"
                        .to_string(),
                ),
            ];
            for (name, body) in defs {
                self.macros.define(macro_def_from_parts(
                    name,
                    true,
                    vec!["x".to_string()],
                    false,
                    false,
                    body,
                ));
            }
        }
        self.macros.define(macro_def_from_parts(
            "__builtin_isinf_sign".to_string(),
            true,
            vec!["x".to_string()],
            false,
            false,
            "({ __typeof__(x) __v = (x); ((__v != __v) ? 0 : ((__v == __builtin_inf()) ? 1 : ((__v == -__builtin_inf()) ? -1 : 0))); })"
                .to_string(),
        ));
        // Width-correct __builtin_isnormal / __builtin_fpclassify via _Generic.
        // __builtin_fpclassify(nan, inf, normal, subnormal, zero, x) is GCC's
        // 6-argument form, so it never depends on the FP_* enum macros.
        self.macros.define(macro_def_from_parts(
            "__builtin_isnormal".to_string(),
            true,
            vec!["x".to_string()],
            false,
            false,
            "({ __typeof__(x) __v = (x); _Generic((__v), float: ((__v == __v) && (__v != 0) && (__v != __builtin_inf()) && (__v != -__builtin_inf()) && ((__v >= __FLT_MIN__) || (__v <= -__FLT_MIN__))), long double: ((__v == __v) && (__v != 0) && (__v != __builtin_inf()) && (__v != -__builtin_inf()) && ((__v >= __LDBL_MIN__) || (__v <= -__LDBL_MIN__))), default: ((__v == __v) && (__v != 0) && (__v != __builtin_inf()) && (__v != -__builtin_inf()) && ((__v >= __DBL_MIN__) || (__v <= -__DBL_MIN__)))); })"
                .to_string(),
        ));
        self.macros.define(macro_def_from_parts(
            "__builtin_fpclassify".to_string(),
            true,
            vec![
                "n".to_string(),
                "i".to_string(),
                "no".to_string(),
                "s".to_string(),
                "z".to_string(),
                "x".to_string(),
            ],
            false,
            false,
            "({ __typeof__(x) __v = (x); _Generic((__v), float: (((__v != __v) ? (n) : (((__v == __builtin_inf()) || (__v == -__builtin_inf())) ? (i) : ((__v == 0) ? (z) : ((((__v < __FLT_MIN__) && (__v > -__FLT_MIN__)) ? (s) : (no))))))), long double: (((__v != __v) ? (n) : (((__v == __builtin_inf()) || (__v == -__builtin_inf())) ? (i) : ((__v == 0) ? (z) : ((((__v < __LDBL_MIN__) && (__v > -__LDBL_MIN__)) ? (s) : (no))))))), default: (((__v != __v) ? (n) : (((__v == __builtin_inf()) || (__v == -__builtin_inf())) ? (i) : ((__v == 0) ? (z) : ((((__v < __DBL_MIN__) && (__v > -__DBL_MIN__)) ? (s) : (no)))))))); })"
                .to_string(),
        ));

        // Bit-operation builtins with no direct Rust method (clrsb, parity,
        // generic *g and C23 stdc_*). Expand to statement expressions built
        // from the resolver-backed __builtin_clz/ctz/popcount (or the
        // libc-forwarded ffs) so each argument evaluates exactly once.
        const BIT_1ARG: &[(&str, &[&str], &str)] = &[
            (
                "__builtin_clrsb",
                &["x"],
                "({ int __v = (x); (__v >= 0 ? __builtin_clz((unsigned)__v) - 1 : __builtin_clz((unsigned)(~__v)) - 1); })",
            ),
            (
                "__builtin_clrsbl",
                &["x"],
                "({ long __v = (x); (__v >= 0 ? __builtin_clzl((unsigned long)__v) - 1 : __builtin_clzl((unsigned long)(~__v)) - 1); })",
            ),
            (
                "__builtin_clrsbll",
                &["x"],
                "({ long long __v = (x); (__v >= 0 ? __builtin_clzll((unsigned long long)__v) - 1 : __builtin_clzll((unsigned long long)(~__v)) - 1); })",
            ),
            (
                "__builtin_parity",
                &["x"],
                "({ unsigned __v = (x); (__builtin_popcount(__v) & 1); })",
            ),
            (
                "__builtin_parityl",
                &["x"],
                "({ unsigned long __v = (x); (__builtin_popcountl(__v) & 1); })",
            ),
            (
                "__builtin_parityll",
                &["x"],
                "({ unsigned long long __v = (x); (__builtin_popcountll(__v) & 1); })",
            ),
            (
                "__builtin_ffsg",
                &["x"],
                "({ __typeof__(x) __v = (x); __builtin_ffsll((long long)__v); })",
            ),
            (
                "__builtin_clrsbg",
                &["x"],
                "({ __typeof__(x) __v = (x); ((__v >= 0 ? (sizeof(__v) <= 4 ? __builtin_clz((unsigned)__v) - (4 - (int)sizeof(__v))*8 : __builtin_clzll((unsigned long long)__v) - (8 - (int)sizeof(__v))*8) : (sizeof(__v) <= 4 ? __builtin_clz((unsigned)(~__v)) - (4 - (int)sizeof(__v))*8 : __builtin_clzll((unsigned long long)(~__v)) - (8 - (int)sizeof(__v))*8)) - 1); })",
            ),
            (
                "__builtin_popcountg",
                &["x"],
                "({ __typeof__(x) __v = (x); __builtin_popcountll((unsigned long long)__v); })",
            ),
            (
                "__builtin_parityg",
                &["x"],
                "({ __typeof__(x) __v = (x); (__builtin_popcountll((unsigned long long)__v) & 1); })",
            ),
            (
                "__builtin_stdc_count_ones",
                &["x"],
                "({ __typeof__(x) __v = (x); __builtin_popcountll((unsigned long long)__v); })",
            ),
            (
                "__builtin_stdc_count_zeros",
                &["x"],
                "({ __typeof__(x) __v = (x); ((int)sizeof(__v)*8 - __builtin_popcountll((unsigned long long)__v)); })",
            ),
            (
                "__builtin_stdc_leading_zeros",
                &["x"],
                "({ __typeof__(x) __v = (x); (sizeof(__v) <= 4 ? __builtin_clz((unsigned)__v) - (4 - (int)sizeof(__v))*8 : __builtin_clzll((unsigned long long)__v) - (8 - (int)sizeof(__v))*8); })",
            ),
            (
                "__builtin_stdc_leading_ones",
                &["x"],
                "({ __typeof__(x) __v = (x); (sizeof(__v) <= 4 ? __builtin_clz((unsigned)(~__v)) - (4 - (int)sizeof(__v))*8 : __builtin_clzll((unsigned long long)(~__v)) - (8 - (int)sizeof(__v))*8); })",
            ),
            (
                "__builtin_stdc_trailing_zeros",
                &["x"],
                "({ __typeof__(x) __v = (x); (__v == 0 ? (int)sizeof(__v)*8 : (sizeof(__v) <= 4 ? __builtin_ctz((unsigned)__v) : __builtin_ctzll((unsigned long long)__v))); })",
            ),
            (
                "__builtin_stdc_trailing_ones",
                &["x"],
                "({ __typeof__(x) __v = (x); (sizeof(__v) <= 4 ? __builtin_ctz((unsigned)(~__v)) : __builtin_ctzll((unsigned long long)(~__v))); })",
            ),
            (
                "__builtin_stdc_bit_width",
                &["x"],
                "({ __typeof__(x) __v = (x); (__v == 0 ? 0 : (int)sizeof(__v)*8 - (sizeof(__v) <= 4 ? __builtin_clz((unsigned)__v) - (4 - (int)sizeof(__v))*8 : __builtin_clzll((unsigned long long)__v) - (8 - (int)sizeof(__v))*8)); })",
            ),
            (
                "__builtin_stdc_bit_floor",
                &["x"],
                "({ __typeof__(x) __v = (x); (__v == 0 ? 0 : ((__typeof__(x))1 << (((int)sizeof(__v)*8 - 1) - (sizeof(__v) <= 4 ? __builtin_clz((unsigned)__v) - (4 - (int)sizeof(__v))*8 : __builtin_clzll((unsigned long long)__v) - (8 - (int)sizeof(__v))*8)))); })",
            ),
            (
                "__builtin_stdc_bit_ceil",
                &["x"],
                "({ __typeof__(x) __v = (x); (__v <= 1 ? 1 : ((__typeof__(x))1 << ((int)sizeof(__v)*8 - (sizeof(__v) <= 4 ? __builtin_clz((unsigned)(__v - 1)) - (4 - (int)sizeof(__v))*8 : __builtin_clzll((unsigned long long)(__v - 1)) - (8 - (int)sizeof(__v))*8)))); })",
            ),
            (
                "__builtin_stdc_first_leading_one",
                &["x"],
                "({ __typeof__(x) __v = (x); (__v == 0 ? 0 : (sizeof(__v) <= 4 ? __builtin_clz((unsigned)__v) - (4 - (int)sizeof(__v))*8 : __builtin_clzll((unsigned long long)__v) - (8 - (int)sizeof(__v))*8) + 1); })",
            ),
            (
                "__builtin_stdc_first_leading_zero",
                &["x"],
                "({ __typeof__(x) __v = (x); ((__typeof__(x))(~__v) == 0 ? 0 : (sizeof(__v) <= 4 ? __builtin_clz((unsigned)(~__v)) - (4 - (int)sizeof(__v))*8 : __builtin_clzll((unsigned long long)(~__v)) - (8 - (int)sizeof(__v))*8) + 1); })",
            ),
            (
                "__builtin_stdc_first_trailing_one",
                &["x"],
                "({ __typeof__(x) __v = (x); (__v == 0 ? 0 : (sizeof(__v) <= 4 ? __builtin_ctz((unsigned)__v) : __builtin_ctzll((unsigned long long)__v)) + 1); })",
            ),
            (
                "__builtin_stdc_first_trailing_zero",
                &["x"],
                "({ __typeof__(x) __v = (x); ((__typeof__(x))(~__v) == 0 ? 0 : (sizeof(__v) <= 4 ? __builtin_ctz((unsigned)(~__v)) : __builtin_ctzll((unsigned long long)(~__v))) + 1); })",
            ),
            (
                "__builtin_stdc_has_single_bit",
                &["x"],
                "({ __typeof__(x) __v = (x); (__v != 0 && (__v & (__v - 1)) == 0); })",
            ),
            (
                "__builtin_stdc_rotate_left",
                &["x", "y"],
                "({ __typeof__(x) __v = (x); int __s = (int)(y); int __w = (int)sizeof(__v)*8; int __r = __s % __w; (__r == 0 ? __v : (__typeof__(x))((__v << __r) | (__v >> (__w - __r)))); })",
            ),
            (
                "__builtin_stdc_rotate_right",
                &["x", "y"],
                "({ __typeof__(x) __v = (x); int __s = (int)(y); int __w = (int)sizeof(__v)*8; int __r = __s % __w; (__r == 0 ? __v : (__typeof__(x))((__v >> __r) | (__v << (__w - __r)))); })",
            ),
        ];
        for &(name, params, body) in BIT_1ARG {
            self.macros.define(macro_def_from_parts(
                name.to_string(),
                true,
                params.iter().map(|s| s.to_string()).collect(),
                false,
                false,
                body.to_string(),
            ));
        }
        // Generic clz/ctz accept 1 arg, or 2 args with an explicit zero
        // fallback (e.g. __builtin_clzg(0u, 32)). The fallback is ignored:
        // resolver-backed clz/ctz already return the width for zero, which
        // is what the tests pass as fallback.
        for name in ["__builtin_clzg", "__builtin_ctzg"] {
            let is_clz = name.ends_with("clzg");
            let body = if is_clz {
                "({ __typeof__(x) __v = (x); (sizeof(__v) <= 4 ? __builtin_clz((unsigned)__v) - (4 - (int)sizeof(__v))*8 : __builtin_clzll((unsigned long long)__v) - (8 - (int)sizeof(__v))*8); })"
            } else {
                "({ __typeof__(x) __v = (x); (__v == 0 ? (int)sizeof(__v)*8 : (sizeof(__v) <= 4 ? __builtin_ctz((unsigned)__v) : __builtin_ctzll((unsigned long long)__v))); })"
            };
            self.macros.define(macro_def_from_parts(
                name.to_string(),
                true,
                vec!["x".to_string()],
                true,
                false,
                body.to_string(),
            ));
        }

        for &(name, body) in PREDEFINED_OBJECT_MACROS {
            self.define_simple_macro(name, body);
        }

        for &(name, params, body) in PREDEFINED_FUNC_MACROS {
            self.macros.define(macro_def_from_parts(
                name.to_string(),
                true,
                params
                    .iter()
                    .map(std::string::ToString::to_string)
                    .collect(),
                false,
                false,
                body.to_string(),
            ));
        }
    }

    /// Helper to define a simple object-like macro.
    pub(super) fn define_simple_macro(&mut self, name: &str, body: &str) {
        self.macros.define(macro_def_from_parts(
            name.to_string(),
            false,
            Vec::new(),
            false,
            false,
            body.to_string(),
        ));
    }

    /// Define x86/x86_64 SIMD feature macros (__SSE__, __SSE2__, __MMX__, etc.).
    ///
    /// GCC/Clang always define these for x86_64 (SSE2 is baseline for the ISA).
    /// For i686, GCC only defines them with explicit -msse/-msse2, but since our
    /// i686 backend always uses SSE2 instructions, we define them unconditionally.
    ///
    /// When `no_sse` is true (from -mno-sse or similar flags), these macros are
    /// not defined (matching GCC behavior for kernel builds).
    ///
    /// Must be called after set_target() since it checks which arch is active.
    pub fn set_sse_macros(&mut self, no_sse: bool) {
        if no_sse {
            return;
        }
        // Only define SSE macros for x86 targets (x86_64 and i686).
        // Check that we're on an x86 target by looking for __x86_64__ or __i386__.
        let is_x86_64 = self.macros.is_defined("__x86_64__");
        let is_i386 = self.macros.is_defined("__i386__");
        if !is_x86_64 && !is_i386 {
            return;
        }

        // SSE and SSE2 are baseline for x86_64; our i686 backend also uses SSE2.
        self.define_simple_macro("__SSE__", "1");
        self.define_simple_macro("__SSE2__", "1");
        self.define_simple_macro("__MMX__", "1");

        if is_x86_64 {
            // x86_64 uses SSE for floating-point math by default
            self.define_simple_macro("__SSE_MATH__", "1");
            self.define_simple_macro("__SSE2_MATH__", "1");
            // GCC also defines this for x86_64
            self.define_simple_macro("__MMX_WITH_SSE__", "1");
        }
    }

    /// Set the target architecture, updating predefined macros and include paths.
    pub fn set_target(&mut self, target: &str) {
        match target {
            "aarch64" => {
                // Remove x86 macros
                self.macros.undefine("__x86_64__");
                self.macros.undefine("__x86_64");
                self.macros.undefine("__amd64__");
                self.macros.undefine("__amd64");
                self.macros.undefine("__CET__");
                self.macros.undefine("__SSE__");
                self.macros.undefine("__SSE2__");
                self.macros.undefine("__MMX__");
                self.macros.undefine("__SSE_MATH__");
                self.macros.undefine("__SSE2_MATH__");
                // Define aarch64 macros
                self.define_simple_macro("__aarch64__", "1");
                self.define_simple_macro("__ARM_64BIT_STATE", "1");
                self.define_simple_macro("__ARM_ARCH", "8");
                self.define_simple_macro("__ARM_ARCH_8A", "1");
                self.define_simple_macro("__ARM_ARCH_ISA_A64", "1");
                self.define_simple_macro("__ARM_ARCH_PROFILE", "65"); // 'A'
                // Floating-point and SIMD
                self.define_simple_macro("__ARM_FP", "14"); // 0b1110: half+single+double precision
                self.define_simple_macro("__ARM_NEON", "1");
                self.define_simple_macro("__ARM_FP16_ARGS", "1");
                self.define_simple_macro("__ARM_FP16_FORMAT_IEEE", "1");
                // ABI
                self.define_simple_macro("__ARM_PCS_AAPCS64", "1");
                self.define_simple_macro("__ARM_SIZEOF_MINIMAL_ENUM", "4");
                self.define_simple_macro("__ARM_SIZEOF_WCHAR_T", "4");
                // Features
                self.define_simple_macro("__ARM_FEATURE_CLZ", "1");
                self.define_simple_macro("__ARM_FEATURE_FMA", "1");
                self.define_simple_macro("__ARM_FEATURE_IDIV", "1");
                self.define_simple_macro("__ARM_FEATURE_NUMERIC_MAXMIN", "1");
                self.define_simple_macro("__ARM_FEATURE_UNALIGNED", "1");
                self.define_simple_macro("__ARM_ALIGN_MAX_PWR", "28");
                self.define_simple_macro("__ARM_ALIGN_MAX_STACK_PWR", "16");
                self.define_simple_macro("__AARCH64EL__", "1");
                self.define_simple_macro("__AARCH64_CMODEL_SMALL__", "1");
                // ARM: char is unsigned by default
                self.define_simple_macro("__CHAR_UNSIGNED__", "1");
                // AArch64 uses IEEE 754 binary128 for long double (not x87 80-bit)
                self.override_ldbl_binary128();
            }
            "riscv64" => {
                // Remove x86 macros
                self.macros.undefine("__x86_64__");
                self.macros.undefine("__x86_64");
                self.macros.undefine("__amd64__");
                self.macros.undefine("__amd64");
                self.macros.undefine("__CET__");
                self.macros.undefine("__SSE__");
                self.macros.undefine("__SSE2__");
                self.macros.undefine("__MMX__");
                self.macros.undefine("__SSE_MATH__");
                self.macros.undefine("__SSE2_MATH__");
                // Define riscv64 macros
                self.define_simple_macro("__riscv", "1");
                self.define_simple_macro("__riscv_xlen", "64");
                // Floating-point: double-precision (D extension)
                self.define_simple_macro("__riscv_flen", "64");
                self.define_simple_macro("__riscv_float_abi_double", "1");
                self.define_simple_macro("__riscv_fdiv", "1");
                self.define_simple_macro("__riscv_fsqrt", "1");
                // ISA extensions (RV64GC = IMAFDCZicsr_Zifencei)
                self.define_simple_macro("__riscv_atomic", "1");
                self.define_simple_macro("__riscv_mul", "1");
                self.define_simple_macro("__riscv_muldiv", "1");
                self.define_simple_macro("__riscv_div", "1");
                self.define_simple_macro("__riscv_compressed", "1");
                // Extension version macros (XYYYZZZZ format: e.g. 2001000 = v2.1.0)
                self.define_simple_macro("__riscv_i", "2001000");
                self.define_simple_macro("__riscv_m", "2000000");
                self.define_simple_macro("__riscv_a", "2001000");
                self.define_simple_macro("__riscv_f", "2002000");
                self.define_simple_macro("__riscv_d", "2002000");
                self.define_simple_macro("__riscv_c", "2000000");
                self.define_simple_macro("__riscv_zicsr", "2000000");
                self.define_simple_macro("__riscv_zifencei", "2000000");
                self.define_simple_macro("__riscv_arch_test", "1");
                self.define_simple_macro("__riscv_cmodel_medany", "1");
                // RISC-V uses IEEE 754 binary128 for long double (not x87 80-bit)
                self.override_ldbl_binary128();
            }
            "i686" | "i386" => {
                // Remove x86-64 macros (keep x86 general macros)
                self.macros.undefine("__x86_64__");
                self.macros.undefine("__x86_64");
                self.macros.undefine("__amd64__");
                self.macros.undefine("__amd64");
                self.macros.undefine("__LP64__");
                self.macros.undefine("_LP64");
                self.macros.undefine("__SIZEOF_INT128__");
                // i686 baseline does not include SSE (GCC only enables SSE with
                // -march=pentium4 or higher). Remove SSE macros to match GCC.
                self.macros.undefine("__SSE__");
                self.macros.undefine("__SSE2__");
                self.macros.undefine("__MMX__");
                self.macros.undefine("__SSE_MATH__");
                self.macros.undefine("__SSE2_MATH__");
                // i686-linux-gnu-gcc -m32 does NOT define __CET__ (CET is
                // disabled by -m32).  We must match this because .S assembly
                // files are assembled by GCC, and if the C code expects
                // ENDBR_PRESENT (44-byte trampolines) but the assembly
                // produces non-ENDBR trampolines (40 bytes), the mismatch
                // causes crashes (e.g. libffi closures).
                self.macros.undefine("__CET__");
                // Define i686/i386 macros
                self.define_simple_macro("__i386__", "1");
                self.define_simple_macro("__i386", "1");
                self.define_simple_macro("i386", "1");
                self.define_simple_macro("__i686__", "1");
                self.define_simple_macro("__i686", "1");
                self.define_simple_macro("__ILP32__", "1");
                self.define_simple_macro("_ILP32", "1");
                // ILP32 data model: pointer/long/size_t are 4 bytes
                self.define_simple_macro("__SIZEOF_POINTER__", "4");
                self.define_simple_macro("__SIZEOF_LONG__", "4");
                self.define_simple_macro("__SIZEOF_SIZE_T__", "4");
                self.define_simple_macro("__SIZEOF_PTRDIFF_T__", "4");
                // Long double is 12 bytes on i686 (80-bit x87 + 4 bytes padding)
                self.define_simple_macro("__SIZEOF_LONG_DOUBLE__", "12");
                // Type limits for ILP32
                self.define_simple_macro("__LONG_MAX__", "2147483647L");
                self.define_simple_macro("__SIZE_MAX__", "4294967295U");
                self.define_simple_macro("__PTRDIFF_MAX__", "2147483647");
                // Override <limits.h> macros for ILP32 (long is 32-bit)
                self.define_simple_macro("LONG_MIN", "(-2147483647L-1L)");
                self.define_simple_macro("LONG_MAX", "2147483647L");
                self.define_simple_macro("ULONG_MAX", "4294967295UL");
                // Override <stdint.h> macros for ILP32 (pointer/size_t are 32-bit)
                self.define_simple_macro("INTPTR_MIN", "(-2147483647-1)");
                self.define_simple_macro("INTPTR_MAX", "2147483647");
                self.define_simple_macro("UINTPTR_MAX", "4294967295U");
                self.define_simple_macro("SIZE_MAX", "4294967295U");
                self.define_simple_macro("PTRDIFF_MIN", "(-2147483647-1)");
                self.define_simple_macro("PTRDIFF_MAX", "2147483647");
                // Type names for ILP32 (long is 32-bit, so many types change)
                self.define_simple_macro("__SIZE_TYPE__", "unsigned int");
                self.define_simple_macro("__PTRDIFF_TYPE__", "int");
                self.define_simple_macro("__INTMAX_TYPE__", "long long int");
                self.define_simple_macro("__UINTMAX_TYPE__", "long long unsigned int");
                self.define_simple_macro("__INT64_TYPE__", "long long int");
                self.define_simple_macro("__UINT64_TYPE__", "long long unsigned int");
                self.define_simple_macro("__INTPTR_TYPE__", "int");
                self.define_simple_macro("__UINTPTR_TYPE__", "unsigned int");
                self.define_simple_macro("__INT_LEAST64_TYPE__", "long long int");
                self.define_simple_macro("__UINT_LEAST64_TYPE__", "long long unsigned int");
                self.define_simple_macro("__INT_FAST16_TYPE__", "int");
                self.define_simple_macro("__INT_FAST32_TYPE__", "int");
                self.define_simple_macro("__INT_FAST64_TYPE__", "long long int");
                self.define_simple_macro("__UINT_FAST16_TYPE__", "unsigned int");
                self.define_simple_macro("__UINT_FAST64_TYPE__", "long long unsigned int");
                // Override width macros for ILP32 (pointer/long/size_t/ptrdiff are 32-bit)
                self.define_simple_macro("__LONG_WIDTH__", "32");
                self.define_simple_macro("__PTRDIFF_WIDTH__", "32");
                self.define_simple_macro("__SIZE_WIDTH__", "32");
                self.define_simple_macro("__INTPTR_WIDTH__", "32");
                self.define_simple_macro("__INT_FAST16_WIDTH__", "32");
                self.define_simple_macro("__INT_FAST32_WIDTH__", "32");
                // i686 uses the same x87 80-bit long double format as x86-64
                // (LDBL macros are already set correctly), but sizeof differs (12 vs 16)
            }
            _ => {
                // x86_64 is already the default
            }
        }
    }

    /// Override long double macros from x87 80-bit extended to IEEE 754 binary128.
    /// Called by set_target() for aarch64 and riscv64 which use quad precision.
    fn override_ldbl_binary128(&mut self) {
        // GCC predefined macros (__LDBL_*__)
        self.define_simple_macro("__LDBL_MANT_DIG__", "113");
        self.define_simple_macro("__LDBL_DIG__", "33");
        self.define_simple_macro(
            "__LDBL_EPSILON__",
            "1.92592994438723585305597794258492732e-34L",
        );
        self.define_simple_macro(
            "__LDBL_MAX__",
            "1.18973149535723176508575932662800702e+4932L",
        );
        self.define_simple_macro(
            "__LDBL_MIN__",
            "3.36210314311209350626267781732175260e-4932L",
        );
        self.define_simple_macro(
            "__LDBL_DENORM_MIN__",
            "6.47517511943802511092443895822764655e-4966L",
        );
        self.define_simple_macro("__LDBL_DECIMAL_DIG__", "36");
        self.define_simple_macro("__DECIMAL_DIG__", "36");
        // MIN_EXP, MAX_EXP, MIN_10_EXP, MAX_10_EXP are the same for x87 and binary128
        // (both use 15-bit exponent fields), so no override needed.

        // <float.h> macros (LDBL_*)
        self.define_simple_macro("LDBL_MANT_DIG", "113");
        self.define_simple_macro("LDBL_DIG", "33");
        self.define_simple_macro("DECIMAL_DIG", "36");
    }
}
