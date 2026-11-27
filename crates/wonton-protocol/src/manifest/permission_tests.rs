use super::*;

#[test]
fn every_row_parses_displays_and_round_trips() {
    for (index, row) in PERMISSIONS.iter().enumerate() {
        assert_eq!(row.kind as usize, index, "{}", row.syntax);
        let permission: Permission = row.example.parse().unwrap_or_else(|error| {
            panic!("{} does not parse: {error:?}", row.example);
        });
        assert_eq!(permission.kind(), row.kind, "{}", row.example);
        assert_eq!(permission.spec(), row);
        assert_eq!(permission.to_string(), row.example);
        let names: Vec<&str> = permission.params().iter().map(|(name, _)| *name).collect();
        assert_eq!(names, row.params, "{}", row.example);
        for name in row.params {
            assert!(
                row.syntax.contains(&format!("{{{name}}}")),
                "{}",
                row.syntax
            );
        }
        let json = serde_json::to_value(&permission).unwrap();
        assert_eq!(json, serde_json::json!(row.example));
        assert_eq!(
            serde_json::from_value::<Permission>(json).unwrap(),
            permission
        );
    }
}

#[test]
fn rows_have_distinct_syntax_and_text() {
    for (index, row) in PERMISSIONS.iter().enumerate() {
        for other in &PERMISSIONS[index + 1..] {
            assert_ne!(row.syntax, other.syntax);
            assert_ne!(row.text, other.text);
        }
    }
}

#[test]
fn only_optional_rows_inject_a_variable() {
    for row in PERMISSIONS {
        if let Some(var) = row.env {
            assert!(row.optional, "{}", row.syntax);
            assert!(var.starts_with("WONTON_PERMISSION_"), "{var}");
        }
    }
}

#[test]
fn unknown_and_malformed_names_are_unknown_permissions() {
    for value in [
        "x-example.thing",
        "storage.share.{name}:{access}",
        "storage.share.photos",
        "storage.share.photos:admin",
        "storage.home:admin",
        "cap.",
        "cap.NET-ADMIN",
        "devices.tun",
        "Models",
    ] {
        assert_eq!(
            value.parse::<Permission>(),
            Err(ManifestError::UnknownPermission(value.to_owned())),
            "{value}"
        );
    }
}

#[test]
fn parameters_are_normalized_like_the_manifest_spec() {
    assert_eq!(
        "cap.net_admin".parse::<Permission>().unwrap(),
        Permission::Capability {
            name: "NET_ADMIN".into()
        }
    );
    assert_eq!(
        "devices.gpu".parse::<Permission>().unwrap(),
        Permission::DevicesGpu
    );
}

#[test]
fn risk_and_notice_follow_the_table() {
    let cap = |name: &str| Permission::Capability { name: name.into() };
    assert_eq!(Permission::Models.risk(), None);
    assert!(!Permission::NetworkClients.is_high());
    assert_eq!(cap("NET_ADMIN").risk(), Some(RiskLevel::VeryHigh));
    assert_eq!(cap("NET_ADMIN").notice(), None);
    assert_eq!(cap("SYS_ADMIN").risk(), Some(RiskLevel::Highest));
    assert_eq!(cap("SYS_ADMIN").notice(), Some(HIGHEST_RISK_NOTICE));
    assert_eq!(Permission::DockerSocket.notice(), Some(HIGHEST_RISK_NOTICE));
    assert_eq!(Permission::DevicesGpu.notice(), None);
}
