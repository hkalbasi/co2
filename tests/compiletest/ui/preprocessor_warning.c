//@ mode: c
//@ compile-fail

  #warning "heads up"
// ^^^^^^^ warning: #warning "heads up"

int main(void) {
    return missing;
    //     ^^^^^^^ error: Unresolved name
}
