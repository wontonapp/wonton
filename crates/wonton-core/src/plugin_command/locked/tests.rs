use super::*;

const COMMIT: &str = "0123456789abcdef0123456789abcdef01234567";

fn line(source: &str, commit: &str) -> String {
    format!("system models {source} {commit} sha256:{}", "a".repeat(64))
}

#[test]
fn lock_refuses_floating_or_conflicting_revisions() {
    let pinned = format!("github:acme/models@{COMMIT}");
    assert!(parse(&line(&pinned, COMMIT)).is_ok());
    assert!(parse(&line("github:acme/models@v1.0.0", COMMIT)).is_ok());
    assert!(parse(&line("github:acme/models", COMMIT)).is_err());
    assert!(parse(&line(&pinned, &"b".repeat(40))).is_err());
    assert!(parse(&line(&pinned, "main")).is_err());
}

#[test]
fn lock_refuses_ambiguous_or_escaping_destinations() {
    let source = format!("github:acme/models@{COMMIT}");
    let valid = line(&source, COMMIT);
    assert!(parse(&format!("{valid}\n{valid}")).is_err());
    for name in ["../models", "/models", "models/other", "", "--", "Models"] {
        assert!(parse(&valid.replacen(" models ", &format!(" {name} "), 1)).is_err());
    }
    assert!(parse(&valid.replacen("system", "community", 1)).is_err());
    assert!(parse("# no pinned packages\n").is_err());
}

#[test]
fn package_must_match_the_reviewed_digest_and_preinstall_category() {
    let compose = "services:\n  web:\n    image: nginx\nx-wonton:\n  spec: '0.4'\n  id: app.models\n  version: 1.0.0\n  kind: system\n  title: {en: Models}\n  description: {en: Model gateway}\n  permissions: []\n";
    let package = PluginPackage::parse(
        compose,
        &wonton_core::plugin::languages::Languages::default(),
    )
    .unwrap();
    let source = format!("github:acme/models@{COMMIT}");
    let mut entries = parse(&line(&source, COMMIT)).unwrap();
    let entry = &mut entries[0];
    entry.digest = package.digest();
    assert!(check_package(entry, &package).is_ok());
    let changed = PluginPackage::parse(
        &compose.replace("image: nginx", "image: caddy"),
        &wonton_core::plugin::languages::Languages::default(),
    )
    .unwrap();
    assert!(check_package(entry, &changed).is_err());
    entry.category = "default".to_owned();
    assert!(check_package(entry, &package).is_err());
}
