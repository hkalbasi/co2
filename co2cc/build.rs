fn main() {
    let version = std::process::Command::new("rustc")
        .arg("--version")
        .output()
        .expect("failed to run `rustc --version`");
    assert!(version.status.success(), "`rustc --version` failed");
    let version = String::from_utf8(version.stdout).expect("rustc version is not valid UTF-8");
    println!("cargo:rustc-env=RUSTC_VERSION={}", version.trim());

    let verbose = std::process::Command::new("rustc")
        .args(["--version", "--verbose"])
        .output()
        .expect("failed to run `rustc --version --verbose`");
    assert!(
        verbose.status.success(),
        "`rustc --version --verbose` failed"
    );
    let verbose =
        String::from_utf8(verbose.stdout).expect("rustc verbose output is not valid UTF-8");
    let llvm = verbose
        .lines()
        .find(|l| l.starts_with("LLVM version: "))
        .map(|l| l.trim_start_matches("LLVM version: "))
        .unwrap_or("unknown");
    println!("cargo:rustc-env=LLVM_VERSION={}", llvm);
}
