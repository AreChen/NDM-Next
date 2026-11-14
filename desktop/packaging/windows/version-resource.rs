use std::{env, fs, path::PathBuf};

pub fn embed(binary: &str, description: &str) {
    println!("cargo:rerun-if-changed=../../packaging/windows/version-resource.rs");

    let component = |name: &str| -> u16 {
        env::var(name)
            .expect("Cargo supplies the package version")
            .parse()
            .expect("Windows version components must fit in 16 bits")
    };
    let major = component("CARGO_PKG_VERSION_MAJOR");
    let minor = component("CARGO_PKG_VERSION_MINOR");
    let patch = component("CARGO_PKG_VERSION_PATCH");
    let version = format!("{major}.{minor}.{patch}");
    let resource = format!(
        r#"1 VERSIONINFO
FILEVERSION {major},{minor},{patch},0
PRODUCTVERSION {major},{minor},{patch},0
FILEFLAGSMASK 0x3fL
FILEFLAGS 0x0L
FILEOS 0x40004L
FILETYPE 0x1L
FILESUBTYPE 0x0L
BEGIN
  BLOCK "StringFileInfo"
  BEGIN
    BLOCK "040904b0"
    BEGIN
      VALUE "CompanyName", "NDM Next contributors\0"
      VALUE "FileDescription", "{description}\0"
      VALUE "FileVersion", "{version}\0"
      VALUE "ProductName", "NDM Next\0"
      VALUE "ProductVersion", "{version}\0"
    END
  END
  BLOCK "VarFileInfo"
  BEGIN
    VALUE "Translation", 0x0409, 1200
  END
END
"#
    );
    let file = PathBuf::from(env::var_os("OUT_DIR").expect("Cargo supplies OUT_DIR"))
        .join("ndm-version.rc");
    fs::write(&file, resource).expect("write Windows version resource");
    embed_resource::compile_for(&file, [binary], embed_resource::NONE)
        .manifest_required()
        .expect("embed Windows version information");
}
