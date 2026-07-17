fn main() {
    println!("cargo:rerun-if-env-changed=FAILURE_VERSION");
}
