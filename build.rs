fn main() {
    println!("cargo:rerun-if-changed=src/lib.rs");
    println!("cargo:rerun-if-changed=Cargo.toml");
    println!("cargo:rerun-if-changed=src/zarchivereader.cpp");
    println!("cargo:rerun-if-changed=include/zarchive/zarchivereader.h");
    println!("cargo:rustc-link-lib=static=zstd");
    cxx_build::bridge("src/lib.rs")
        .include("include")
        .flag("-w")
        .flag_if_supported("-std=c++17")
        .file("src/zarchivereader.cpp")
        .compile("zarchive");
}
