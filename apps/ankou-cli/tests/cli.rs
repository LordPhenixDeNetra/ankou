#![allow(clippy::unwrap_used)]

use assert_cmd::Command;
use std::path::Path;

fn version(lang: &str) -> String {
    let out = Command::cargo_bin("ankou-cli")
        .unwrap()
        .args(["version", "--lang", lang])
        .assert()
        .success();
    String::from_utf8(out.get_output().stdout.clone()).unwrap()
}

#[test]
fn version_in_french() {
    assert_eq!(version("fr"), "aNkou 0.1.0 (protocole 1)\n");
}

#[test]
fn version_in_english() {
    assert_eq!(version("en"), "aNkou 0.1.0 (protocol 1)\n");
}

#[test]
fn unknown_lang_falls_back_to_english() {
    assert!(version("de").contains("(protocol 1)"));
}

#[test]
fn locales_are_complete() {
    let dir = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../locales");
    lpdnsoft_i18n::check_complete(&lpdnsoft_i18n::load_catalog(&dir).unwrap()).unwrap();
}
