//@ mode: c
//@ compile-fail

int f1(void) {
    int arr[_Generic(0, default: 1, default: 2)];
    //                              ^^^^^^^^^^ error: duplicate default association in _Generic
    return 0;
}

int f2(void) {
    _Generic(0, default: 1, default: 2);
    //                      ^^^^^^^^^^ error: duplicate default association in _Generic
    return 0;
}

int f3(void) {
    // A duplicate `default` is a constraint violation even when a type
    // association matches.
    int arr[_Generic(0, int: 3, default: 1, default: 2)];
    //                                      ^^^^^^^^^^ error: duplicate default association in _Generic
    return 0;
}

int f4(void) {
    // Same as f3 but in runtime position.
    return _Generic(0, int: 3, default: 1, default: 2);
    //                                     ^^^^^^^^^^ error: duplicate default association in _Generic
}

int f5(void) {
    // No matching association and no default, in constant context.
    int arr[_Generic(1, double: 1)];
    //      ^^^^^^^^^^^^^^^^^^^^^^ error: no matching association in _Generic and no default provided
    return 0;
}

int f6(void) {
    // Defaults split around the matching association are still an error.
    return _Generic(0, default: 1, int: 3, default: 2);
    //                                     ^^^^^^^^^^ error: duplicate default association in _Generic
}

int main(void) {
    return 0;
}
