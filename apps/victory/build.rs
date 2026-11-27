//! Exports the DirectX 12 Agility SDK's two symbols from the program
//! (`D3D12SDKVersion`, `D3D12SDKPath` in `main.rs`): the D3D12 loader looks
//! them up in the process's executable.

fn main() {
    if std::env::var("CARGO_CFG_TARGET_OS").as_deref() == Ok("windows") {
        println!("cargo:rustc-link-arg-bins=/EXPORT:D3D12SDKVersion,DATA");
        println!("cargo:rustc-link-arg-bins=/EXPORT:D3D12SDKPath,DATA");
    }
}
