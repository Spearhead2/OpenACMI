//! Gives the Windows executable its icon and version details.

fn main() {
    println!("cargo:rerun-if-changed=../../assets/icon.ico");
    if std::env::var_os("CARGO_CFG_WINDOWS").is_some() {
        let mut resource = winresource::WindowsResource::new();
        resource.set_icon("../../assets/icon.ico");
        resource.set("ProductName", "OpenACMI");
        resource.set("FileDescription", "OpenACMI flight recording viewer");
        if let Err(err) = resource.compile() {
            // A missing resource compiler should not stop a build from source.
            println!("cargo:warning=the executable will have no icon: {err}");
        }
    }
}
