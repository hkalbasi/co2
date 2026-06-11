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

    pub fn and_then<U: Debug, F: FnOnce(T) -> GenericWrap<U>>(&self, f: F) -> GenericWrap<U> {
        let u = f(self.val);
        println!("GenericWrap and_then: {:?} {:?}", self.val, u.val);
        u
    }
}

pub struct GenericWrapWithDefault<A, B = i32> {
    val_a: A,
    val_b: B,
}

impl<A, B> GenericWrapWithDefault<A, B> {
    pub fn new(val_a: A, val_b: B) -> Self {
        Self { val_a, val_b }
    }
}

impl<A: Debug, B: Debug> GenericWrapWithDefault<A, B> {
    pub fn ok(&self) {
        println!(
            "GenericWrapWithDefault ok: {:?} {:?}",
            self.val_a, self.val_b,
        );
    }
}

impl GenericWrapWithDefault<i32> {
    pub fn sum(&self) {
        println!(
            "GenericWrapWithDefault<i32> sum: {}",
            self.val_a + self.val_b,
        );
    }
}

impl GenericWrapWithDefault<i64, i64> {
    pub fn sum(&self) {
        println!(
            "GenericWrapWithDefault<i64> sum: {}",
            self.val_a + self.val_b,
        );
    }
}

// -----------------------------------------------------------------------------
// Trait methods
// -----------------------------------------------------------------------------

pub trait TraitSimple {
    fn trait_simple(&self);
}

impl TraitSimple for Struct1 {
    fn trait_simple(&self) {
        println!("TraitSimple {}", self.f1);
    }
}

// -----------------------------------------------------------------------------
// Generic trait parameter
// -----------------------------------------------------------------------------

pub trait TraitArg<T> {
    fn trait_arg(&self, value: T);
}

impl TraitArg<i32> for Struct1 {
    fn trait_arg(&self, value: i32) {
        println!("TraitArg<i32> {} {}", self.f1, value);
    }
}

impl TraitArg<&str> for Struct1 {
    fn trait_arg(&self, value: &str) {
        println!("TraitArg<&str> {} {}", self.f1, value);
    }
}

// -----------------------------------------------------------------------------
// Generic method inside trait
// -----------------------------------------------------------------------------

pub trait TraitGenericMethod {
    fn generic_method<T: Debug>(&self, value: T);
}

impl TraitGenericMethod for Struct1 {
    fn generic_method<T: Debug>(&self, value: T) {
        println!("TraitGenericMethod {:?} {}", value, self.f1);
    }
}

// -----------------------------------------------------------------------------
// Trait implemented for generic type
// -----------------------------------------------------------------------------

pub trait WrapTrait {
    fn wrap_trait(&self);
}

impl<T: Debug + Copy> WrapTrait for GenericWrap<T> {
    fn wrap_trait(&self) {
        println!("WrapTrait {:?}", self.val);
    }
}

// -----------------------------------------------------------------------------
// Generic trait implemented for generic type
// -----------------------------------------------------------------------------

pub trait Combine<U> {
    fn combine(&self, u: U);
}

impl<T: Debug + Copy, U: Debug> Combine<U> for GenericWrap<T> {
    fn combine(&self, u: U) {
        println!("Combine {:?} {:?}", self.val, u);
    }
}

// -----------------------------------------------------------------------------
// Associated type
// -----------------------------------------------------------------------------

pub trait ReplaceTrait {
    type Output;

    fn replace_trait(self) -> Self::Output;
}

impl<T: Debug + Copy> ReplaceTrait for GenericWrap<T> {
    type Output = T;

    fn replace_trait(self) -> T {
        self.val
    }
}

// -----------------------------------------------------------------------------
// Associated type on a trait used in method bound
// -----------------------------------------------------------------------------

pub trait MyTrait {
    type Assoc;
    fn get_assoc(&self) -> Self::Assoc;
}

pub struct MyAssocProvider(i32);

impl MyAssocProvider {
    pub fn new(v: i32) -> Self {
        Self(v)
    }
}

impl MyTrait for MyAssocProvider {
    type Assoc = i32;
    fn get_assoc(&self) -> i32 {
        self.0
    }
}

impl<T: Debug + Copy> GenericWrap<T> {
    pub fn use_assoc<U, F: MyTrait<Assoc = U>>(&self, _f: F) -> U {
        _f.get_assoc()
    }
}

// -----------------------------------------------------------------------------
// Blanket impl
// -----------------------------------------------------------------------------

pub trait PrintDebug {
    fn print_debug(&self);
}

impl<T: Debug> PrintDebug for T {
    fn print_debug(&self) {
        println!("PrintDebug {:?}", self);
    }
}

// -----------------------------------------------------------------------------
// Same method name as inherent method
// -----------------------------------------------------------------------------

pub trait TraitOk {
    fn ok(&self);
}

impl<T: Debug + Copy> TraitOk for GenericWrap<T> {
    fn ok(&self) {
        println!("TraitOk {:?}", self.val);
    }
}

// -----------------------------------------------------------------------------
// Receiver by value
// -----------------------------------------------------------------------------

pub trait ReceiverTrait {
    fn recv(self);
}

impl ReceiverTrait for Box<Struct1> {
    fn recv(self) {
        println!("ReceiverTrait {}", self.f1);
    }
}

// -----------------------------------------------------------------------------
// Trait objects
// -----------------------------------------------------------------------------

pub trait DynTrait {
    fn dyn_call(&self);
}

impl DynTrait for Struct1 {
    fn dyn_call(&self) {
        println!("DynTrait {}", self.f1);
    }
}

// -----------------------------------------------------------------------------
// Debug impls so blanket impl works
// -----------------------------------------------------------------------------

impl Debug for Struct1 {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Struct1").field("f1", &self.f1).finish()
    }
}

impl<T: Debug> Debug for GenericWrap<T> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("GenericWrap")
            .field("val", &self.val)
            .finish()
    }
}

pub fn print_name(name: &str) {
    println!("--- {name} ---");
}
