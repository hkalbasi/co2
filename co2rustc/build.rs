fn main() {
    let version = std::process::Command::new("rustc")
        .arg("--version")
        .output()
        .expect("failed to run `rustc --version`");
    assert!(version.status.success(), "`rustc --version` failed");
    let version = String::from_utf8(version.stdout).expect("rustc version is not valid UTF-8");
    println!("cargo:rustc-env=RUSTC_VERSION={}", version.trim());
}
