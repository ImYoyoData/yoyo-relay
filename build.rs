fn main() {
    println!("cargo:rerun-if-changed=assets/logo.ico");
    println!("cargo:rerun-if-changed=assets/icon_256.rgba");

    #[cfg(windows)]
    {
        let mut resource = winresource::WindowsResource::new();
        resource.set_icon("assets/logo.ico");
        resource.set("ProductName", "YoyoRelay");
        resource.set("FileDescription", "YoyoRelay local LLM relay");
        resource.set("LegalCopyright", "MIT");
        if let Err(err) = resource.compile() {
            println!("cargo:warning=Windows icon resource skipped: {err}");
        }
    }
}
