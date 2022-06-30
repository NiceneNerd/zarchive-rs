fn main() {
    println!("cargo:rerun-if-changed=src/lib.rs");
    println!("cargo:rerun-if-changed=Cargo.toml");
    println!("cargo:rerun-if-changed=src/zarchivereader.cpp");
    println!("cargo:rerun-if-changed=src/zarchivewriter.cpp");
    println!("cargo:rerun-if-changed=include/zarchive/zarchivereader.h");
    println!("cargo:rerun-if-changed=include/zarchive/zarchivewriter.h");
    println!("cargo:rustc-link-lib=static=zstd");
    cxx_build::bridges(["src/reader.rs", "src/writer.rs"].into_iter())
        .file("include/sha_256.c")
        .include("include")
        .flag("-w")
        .flag_if_supported("-std=c++20")
        .flag_if_supported("/std:c++20")
        .file("src/zarchivereader.cpp")
        .file("src/zarchivewriter.cpp")
        .compile("zarchive");
}
