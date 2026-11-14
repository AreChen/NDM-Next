#[path = "../../packaging/windows/version-resource.rs"]
mod version_resource;

fn main() {
    if std::env::var("CARGO_CFG_TARGET_OS").as_deref() == Ok("windows") {
        version_resource::embed("ndm-host", "NDM Next browser integration host");
    }
}
