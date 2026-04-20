fn main() {
    let arg = std::env::args().nth(1);
    match arg.as_deref() {
        Some("-V") | Some("--version") | Some("version") => {
            println!("gwtd {}", env!("CARGO_PKG_VERSION"));
        }
        Some("-h") | Some("--help") => {
            println!("gwtd {}", env!("CARGO_PKG_VERSION"));
            println!("Internal runtime daemon binary. Launch `gwt` instead.");
        }
        _ => {
            println!("gwtd is reserved for internal runtime daemon use. Launch `gwt` instead.");
        }
    }
}
