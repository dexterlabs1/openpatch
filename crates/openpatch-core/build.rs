fn main() {
    let root = "vendor/mutable";
    let mut build = cc::Build::new();
    build
        .cpp(true)
        .std("c++17")
        .define("TEST", None)
        .include(root)
        .flag_if_supported("-fwrapv")
        .flag_if_supported("-fno-strict-aliasing")
        .warnings(false);
    if std::env::var("CARGO_CFG_TARGET_OS").as_deref() == Ok("macos") {
        let output = std::process::Command::new("xcrun")
            .arg("--show-sdk-path")
            .output()
            .expect("macOS SDK");
        assert!(output.status.success(), "xcrun must locate the macOS SDK");
        let sdk = String::from_utf8(output.stdout).unwrap();
        build.flag("-isysroot").flag(sdk.trim());
        build
            .flag("-isystem")
            .flag(format!("{}/usr/include/c++/v1", sdk.trim()));
    }
    for path in [
        "braids/analog_oscillator.cc",
        "braids/digital_oscillator.cc",
        "braids/macro_oscillator.cc",
        "braids/resources.cc",
        "braids_wrapper.cc",
    ] {
        build.file(format!("{root}/{path}"));
    }
    build.compile("openpatch_braids");
    println!("cargo:rerun-if-changed={root}");
}
