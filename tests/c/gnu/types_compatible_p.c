//@ mode: c
//@ run-status: 0

typedef int global_ty;

int main(void) {
    // Same primitive types are compatible
    if (!__builtin_types_compatible_p(int, int)) return 1;
    if (!__builtin_types_compatible_p(double, double)) return 2;
    if (!__builtin_types_compatible_p(char, char)) return 3;

    // Different primitive types are not compatible
    if (__builtin_types_compatible_p(int, double)) return 4;
    if (__builtin_types_compatible_p(int, char)) return 5;
    if (__builtin_types_compatible_p(float, double)) return 6;

    // Pointer types
    if (!__builtin_types_compatible_p(int *, int *)) return 7;
    if (__builtin_types_compatible_p(int *, double *)) return 8;

    // Signed vs unsigned are not compatible
    if (__builtin_types_compatible_p(int, unsigned int)) return 9;

    // const-qualified and unqualified types are compatible (GCC ignores top-level qualifiers)
    if (!__builtin_types_compatible_p(int, const int)) return 10;

    typedef double local_ty;

    if (!__builtin_types_compatible_p(int, global_ty)) return 11;
    if (__builtin_types_compatible_p(int, local_ty)) return 12;
    if (__builtin_types_compatible_p(double, global_ty)) return 13;
    if (!__builtin_types_compatible_p(double, local_ty)) return 14;
    if (__builtin_types_compatible_p(global_ty, local_ty)) return 15;

    if (__builtin_types_compatible_p(int[], int*)) return 16;
    if (__builtin_types_compatible_p(int[5], int*)) return 17;
    if (__builtin_types_compatible_p(int[5], int[6])) return 18;
    if (!__builtin_types_compatible_p(int[5], int[5])) return 19;
    if (!__builtin_types_compatible_p(int[5], int[])) return 20;
    if (!__builtin_types_compatible_p(int[], int[])) return 21;

    return 0;
}
