// Esconde o console no Windows em builds de release.
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

fn main() {
    let args: Vec<String> = std::env::args().collect();
    // Iniciado pelo navegador (native messaging): vira a ponte, sem janela.
    if scout_vault_app::browser::is_native_host_launch(&args) {
        let (mut input, mut output) = (std::io::stdin().lock(), std::io::stdout().lock());
        scout_vault_app::browser::run_native_host(
            &mut input,
            &mut output,
            scout_vault_app::browser::ask_app,
        );
        return;
    }
    scout_vault_app::run();
}
