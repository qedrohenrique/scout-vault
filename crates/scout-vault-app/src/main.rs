// Hides the console on Windows in release builds.
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

fn main() {
    let args: Vec<String> = std::env::args().collect();
    // Started by the browser (native messaging): becomes the bridge, no window.
    if scout_vault_app::browser::is_native_host_launch(&args) {
        // Error messages for the extension come out in the app's language.
        let language = scout_vault_app::settings::SettingsStore::default_location()
            .load()
            .language;
        scout_vault_app::i18n::set(scout_vault_app::i18n::Language::from_setting(&language));
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
