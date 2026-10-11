fn main() {
    println!("cargo:rerun-if-changed=assets/world-of-osso.ico");
    #[cfg(windows)]
    winresource::WindowsResource::new()
        .set_icon("assets/world-of-osso.ico")
        .set("ProductName", "World of Osso")
        .set("FileDescription", "World of Osso Launcher")
        .compile()
        .expect("Compile the World of Osso Windows icon resource");
}
