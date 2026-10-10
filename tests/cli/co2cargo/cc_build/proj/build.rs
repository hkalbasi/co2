fn main() {
    cc::Build::new()
        .compiler("co2cc")
        .file("src/native/add.c")
        .compile("add");
}
