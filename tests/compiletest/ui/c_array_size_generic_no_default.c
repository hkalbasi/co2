//@ mode: c
//@ compile-fail

int arr[_Generic(1, double: 1)] = {};
//      ^^^^^^^^^^^^^^^^^^^^^^ error: no matching association in _Generic and no default provided
