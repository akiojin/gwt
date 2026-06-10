#[cfg(target_os = "windows")]
fn main() {
    println!("cargo:rerun-if-changed=../../assets/icons/icon.ico");

    let mut resource = winresource::WindowsResource::new();
    resource.set_icon("../../assets/icons/icon.ico");
    resource.set("FileDescription", "GWT");
    resource.set("ProductName", "GWT");
    resource.set("CompanyName", "GWT Contributors");
    resource.set("LegalCopyright", "Copyright (c) GWT Contributors");
    // #3018: without an embedded manifest declaring requestedExecutionLevel,
    // UAC installer detection treats keyword-named copies of these binaries
    // (e.g. the self-update helper) as legacy installers and CreateProcess
    // fails with ERROR_ELEVATION_REQUIRED (os error 740).
    resource.set_manifest(
        r#"<assembly xmlns="urn:schemas-microsoft-com:asm.v1" manifestVersion="1.0">
<trustInfo xmlns="urn:schemas-microsoft-com:asm.v3">
    <security>
        <requestedPrivileges>
            <requestedExecutionLevel level="asInvoker" uiAccess="false" />
        </requestedPrivileges>
    </security>
</trustInfo>
</assembly>"#,
    );

    if let Err(error) = resource.compile() {
        panic!("failed to compile Windows resources: {error}");
    }
}

#[cfg(not(target_os = "windows"))]
fn main() {}
