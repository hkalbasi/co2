//@ mode: c
//@ compile-fail

// sizeof and alignof on an extern (incomplete) struct type are invalid:
// the type has no statically known size.

struct Ext;

int f1() {
    int a = sizeof(struct Ext);
  //        ^^^^^^^^^^^^^^^^^^ error: a trait bound is not satisfied: `std::marker::Sized` for types [__co2_c_adt_Ext_0__foreign]
    return a;
}

int f2() {
    int b = alignof(struct Ext);
  //        ^^^^^^^^^^^^^^^^^^^ error: a trait bound is not satisfied: `std::marker::Sized` for types [__co2_c_adt_Ext_0__foreign]
    return b;
}

int main() {
    return 0;
}
