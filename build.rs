fn main() {
    println!("cargo::rustc-check-cfg=cfg(no_diagnostic_namespace)");
    let rustc = std::env::var("RUSTC").unwrap_or_else(|_| "rustc".into());

    let output = std::process::Command::new(rustc)
        .arg("--version")
        .output()
        .expect("failed to run rustc --version");

    let stdout = String::from_utf8_lossy(&output.stdout);

    println!("cargo:rustc-check-cfg=cfg(nightly_rust)");
    if stdout.contains("nightly") {
        println!("cargo:rustc-cfg=nightly_rust");
    }
}
