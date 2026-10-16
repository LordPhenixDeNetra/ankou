use std::path::Path;

fn main() {
    println!("cargo:rerun-if-changed=../../locales");
    let manifest = std::env::var("CARGO_MANIFEST_DIR").unwrap_or_default();
    let locales = Path::new(&manifest).join("../../locales");
    // Lu par la macro `t!` pour vérifier les identifiants à la compilation.
    println!(
        "cargo:rustc-env=LPDNSOFT_I18N_LOCALES_DIR={}",
        locales.display()
    );
}
