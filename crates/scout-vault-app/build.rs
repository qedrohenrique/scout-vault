//! Embeds the icon and the metadata in the Windows executable.
//! GPUI uses the icon with ID 1 for the window and the taskbar.

fn main() {
    println!("cargo:rerun-if-changed=assets/brand/scout-vault.ico");
    #[cfg(windows)]
    {
        let mut res = winresource::WindowsResource::new();
        res.set_icon_with_id("assets/brand/scout-vault.ico", "1")
            .set("ProductName", "Scout Vault")
            .set(
                "FileDescription",
                "Scout Vault — cliente desktop compatível com Bitwarden",
            );
        if let Err(e) = res.compile() {
            println!("cargo:warning=ícone não embutido: {e}");
        }
    }
}
