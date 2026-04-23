#[cfg(target_os = "windows")]
fn main() {
    println!("cargo:rerun-if-changed=../../assets/icons/icon.ico");

    let mut resource = winresource::WindowsResource::new();
    resource.set_icon("../../assets/icons/icon.ico");
    resource.set("FileDescription", "GWT");
    resource.set("ProductName", "GWT");
    resource.set("CompanyName", "GWT Contributors");
    resource.set("LegalCopyright", "Copyright (c) GWT Contributors");

    if let Err(error) = resource.compile() {
        panic!("failed to compile Windows resources: {error}");
    }
}

#[cfg(not(target_os = "windows"))]
fn main() {}
