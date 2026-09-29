//@ mode: c
//@ compile-fail

// TODO: spans and error messages are broken

void shl_with_space() {
    int x = 1 < < 5;
              //^ error: expected expression, found <
}

void shr_with_space() {
    int x = 1 > > 5;
      //TODO emit error: found end of input expected Type specifier, Type qualifier, Storage specifier, or Function specifier
}

int main() {
    return 0;
}
