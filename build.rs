extern crate skeptic;
extern crate version_check;

use std::{env, fs};


#[cfg(unix)]
fn main() {
    if env::var("USE_SKEPTIC").is_ok() {
        // generates doc tests for `README.md`.
        skeptic::generate_doc_tests(&["README.md", "guide/src/qs_2.md"]);
    } else {
        let f = env::var("OUT_DIR").unwrap() + "/skeptic-tests.rs";
        let _ = fs::File::create(f);
    }

    match version_check::is_nightly() {
        Some(true) => println!("cargo:rustc-cfg=actix_nightly"),
        Some(false) => (),
        None => (),
    };
}

#[cfg(not(unix))]
fn main() {
    match version_check::is_nightly() {
        Some(true) => println!("cargo:rustc-cfg=actix_nightly"),
        Some(false) => (),
        None => (),
    };
}
