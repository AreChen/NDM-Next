#[path = "../../packaging/windows/version-resource.rs"]
mod version_resource;

fn main() {
    println!("cargo:rerun-if-changed=../../packaging/windows/ndm-next.ico");
    if std::env::var("CARGO_CFG_TARGET_OS").as_deref() == Ok("windows") {
        // GPUI's debug-mode element builders can exceed Windows' 1 MiB default
        // stack when rendering task rows. This reserves address space; Windows
        // commits stack pages on demand.
        if std::env::var("CARGO_CFG_TARGET_ENV").as_deref() == Ok("msvc") {
            println!("cargo:rustc-link-arg-bin=ndm-desktop=/STACK:8388608");
        }
        embed_resource::compile("icon.rc", embed_resource::NONE)
            .manifest_optional()
            .expect("embed the application icon");
        version_resource::embed("ndm-desktop", "OnesDM desktop application");
    }
}
