//! Embute o ícone e os metadados no executável do Windows.
//! O GPUI usa o ícone de ID 1 na janela e na barra de tarefas.

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
