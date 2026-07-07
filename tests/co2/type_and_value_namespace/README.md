# Type and value namespaces

This code works in Rust:

```Rust
pub mod square {
    pub fn y() {
        println!("salam");
    }
}

pub mod gav {
    use super::square;

    fn x() {
        square(2);
        square::y();
    }    
}

pub fn square(num: i32) -> i32 {
    num * num
}
```

So ideally it should work in CO2 too, but it is not supported yet. When you implemented the support,
add a test here.
