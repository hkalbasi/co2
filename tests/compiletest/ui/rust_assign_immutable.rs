//@ mode: rust
//@ compile-fail

fn main() {
    let x = 2;
    x = 3;
  //^^^^^ error: cannot assign twice to immutable variable `x`
}
