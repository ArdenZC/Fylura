fn main() {
    let include = std::env::var("DEP_SQLITE3_INCLUDE").expect("bundled sqlite header required");
    cc::Build::new()
        .include(include)
        .file("src/rank.c")
        .compile("effective_rank");
    println!("cargo:rustc-link-lib=m");
    println!("cargo:rerun-if-changed=src/rank.c");
}
