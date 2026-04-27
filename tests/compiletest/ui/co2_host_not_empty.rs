//@ mode: rust
//@ compile-fail

  #![language(co2)]
    #![allow(dead_code)]
  //^^^^^^^^^^^^^^^^^^^^ error: CO2 host file must not contain any other attributes

    fn main() {}
  //^^^^^^^^^^^^ error: CO2 host file must not contain any items
