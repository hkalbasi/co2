//@ mode: c
//@ compile-fail

/*
 * This banner comment used to collapse during preprocessing
 * and shift later diagnostics to the wrong line.
 * Keep it multiline so the UI span check catches regressions.
 */

int main(void) {
    unknown_type value;
  //^^^^^^^^^^^^ error: unresolved name unknown_type
    return 0;
}

int main2(void) {
    /*Span shifting comment*/   return missing;
                                //     ^^^^^^^ error: unresolved name missing
}

int main3(void) {
    /* Multiline
        Span shifting comment*/   return missing;
                                  //     ^^^^^^^ error: unresolved name missing
}
