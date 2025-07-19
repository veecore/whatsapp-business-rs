fn main() {
    let rustc = std::env::var("RUSTC").unwrap_or_else(|_| "rustc".into());

    let output = std::process::Command::new(rustc)
        .arg("--version")
        .output()
        .expect("failed to run rustc --version");

    let stdout = String::from_utf8_lossy(&output.stdout);

    if stdout.contains("nightly") {
        println!("cargo::rustc-check-cfg=cfg(nightly_rust)");
    }
}
