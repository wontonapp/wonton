//! 凭据命令的纯函数：参数替换、标准输出取值和空输出。

use wonton_protocol::manifest::CredentialsAuth;

use super::credentials::{Op, Ran, argv, first_line, generated_or_empty};

fn credentials() -> CredentialsAuth {
    CredentialsAuth {
        issue: vec!["sh".into(), "issue".into(), "{name}".into()],
        verify: Some(vec!["sh".into(), "verify".into(), "{name}".into()]),
        revoke: Some(vec!["sh".into(), "revoke".into(), "{name}".into()]),
        rotate: Some(vec!["sh".into(), "rotate".into(), "{name}".into()]),
        ends_sessions: false,
        service: None,
        username: None,
        per_client: true,
        generates: false,
    }
}

#[test]
fn every_operation_substitutes_the_password_id() {
    let credentials = credentials();
    for op in [Op::Issue, Op::Verify, Op::Revoke, Op::Rotate] {
        assert_eq!(argv(&credentials, op, "pw-7").unwrap()[2], "pw-7");
    }
}

#[test]
fn absent_operation_names_itself() {
    let mut credentials = credentials();
    credentials.rotate = None;
    assert!(matches!(
        argv(&credentials, Op::Rotate, "shared"),
        Err(super::credentials::CredentialError::Missing(Op::Rotate))
    ));
}

#[test]
fn generated_value_is_the_trimmed_first_line() {
    assert_eq!(first_line("  abc  \nsecond\n"), "abc");
    assert_eq!(first_line(""), "");
}

#[test]
fn generated_value_is_cut_at_4096_on_a_char_boundary() {
    let ascii = "a".repeat(5000);
    assert_eq!(first_line(&ascii).len(), 4096);

    // 4095 个单字节字符后是 3 字节汉字，第 4096 字节落在汉字中间。
    let mixed = format!("{}中", "a".repeat(4095));
    let cut = first_line(&mixed);
    assert_eq!(cut.len(), 4095);
    assert!(cut.ends_with('a'));
}

#[test]
fn empty_generated_output_fails_as_result_unknown() {
    let error = generated_or_empty(Ran { generated: None }).unwrap_err();
    assert!(matches!(
        error,
        super::credentials::CredentialError::Failed { code: 0, .. }
    ));
    assert_eq!(
        generated_or_empty(Ran {
            generated: Some("v".into())
        })
        .unwrap(),
        Some("v".into())
    );
}
