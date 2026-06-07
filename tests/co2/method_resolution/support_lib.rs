use std::fmt::Debug;

pub struct Struct1 {
    f1: i32,
}

impl Struct1 {
    pub fn new(f1: i32) -> Self {
        Self { f1 }
    }

    pub fn simple(&self, v: i32) {
        println!("Simple {v} {}", self.f1);
    }

    pub fn generic_debug<T: Debug>(&self, v: T) {
        println!("Generic Debug {v:?} {}", self.f1);
    }
}

pub use Struct1 as Struct1Alias;

pub struct GenericWrap<T> {
    val: T,
}

impl<T: Debug + Copy> GenericWrap<T> {
    pub fn new(val: T) -> Self {
        Self { val }
    }

    pub fn ok(&self) {
        println!("GenericWrap simple: {:?}", self.val);
    }

    pub fn extra_param<U: Debug>(&self, u: U) {
        println!("GenericWrap extra_param: {:?} {:?}", self.val, u);
    }

    pub fn replace<U: Debug>(&self, u: U) -> GenericWrap<U> {
        println!("GenericWrap replace: {:?} {:?}", self.val, u);
        GenericWrap { val: u }
    }

    pub fn map<U: Debug, F: FnOnce(T) -> U>(&self, f: F) -> GenericWrap<U> {
        let u = f(self.val);
        println!("GenericWrap map: {:?} {:?}", self.val, u);
        GenericWrap { val: u }
    }
}
