//! Gives `senzor.exe` its icon, which Explorer, the taskbar and shortcuts show. The icon is the
//! crate's copy of `assets/icon/senzor.ico` (dev/crate-files.sh), as crates.io takes only this folder.

fn main() {
    println!("cargo:rerun-if-changed=bundled/icon/senzor.ico");
    if std::env::var("CARGO_CFG_TARGET_OS").as_deref() == Ok("windows") {
        let mut resource = winresource::WindowsResource::new();
        resource.set_icon("bundled/icon/senzor.ico");
        resource
            .compile()
            .expect("embed bundled/icon/senzor.ico into senzor.exe");
    }
}
