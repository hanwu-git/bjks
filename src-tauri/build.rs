fn main() {
    println!("cargo:rerun-if-changed=../src/index.html");
    println!("cargo:rerun-if-changed=../src/main.js");
    println!("cargo:rerun-if-changed=../src/styles.css");
    println!("cargo:rerun-if-changed=tauri.conf.json");
    tauri_build::build()
}
