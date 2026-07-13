//@ mode: c
//@ compile-fail

static int f1(void *opcode) {
    goto *opcode;
  //^^^^^^^^^^^^^ error: indirect goto in function with no address-of-label expressions
}

static int f2(void *opcode) {
// TODO: We should emit error here as there is no &&some_lab in this function.
some_lab:
    goto *opcode;
}

static int f3(void *opcode) {
    goto some_lab;
  //^^^^^^^^^^^^^^ error: unresolved label
}

int main(void) {}
