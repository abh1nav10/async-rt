use std::env;
use std::fs;
use std::path::Path;

fn main() {
    let parallelism: usize = if let Ok(n) = std::thread::available_parallelism() {
        n.into()
    } else {
        2
    };

    let environment =
        env::var("OUT_DIR").expect("Could not interpret the OUT_DIR environment variable!");
    let destination = Path::new(&environment).join("available.rs");

    let data = format!("pub const AVAILABLE_PARALLELISM: usize = {};", parallelism);
    fs::write(destination, data).expect("Full directory path does not exist!");

    // Rerun the build script only if it changes because on any particular machine the amount of parallelism
    // is going to be same!
    println!("cargo:rerun-if-changed=build.rs");
}
