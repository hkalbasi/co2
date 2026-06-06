pub struct Struct1 {
    f1: i32,
}

impl Struct1 {
    pub fn new(f1: i32) -> Self {
        Self { f1 }
    }
}

pub use Struct1 as Struct1Alias;
