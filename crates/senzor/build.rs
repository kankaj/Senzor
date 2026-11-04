//! Gives `senzor.exe` its icon, which Explorer, the taskbar and shortcuts show.

fn main() {
    println!("cargo:rerun-if-changed=../../assets/icon/senzor.ico");
    if std::env::var("CARGO_CFG_TARGET_OS").as_deref() == Ok("windows") {
        let mut resource = winresource::WindowsResource::new();
        resource.set_icon("../../assets/icon/senzor.ico");
        resource
            .compile()
            .expect("embed assets/icon/senzor.ico into senzor.exe");
    }
}
