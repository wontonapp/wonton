use super::*;
use crate::{Action, ServersViewModel};

fn round_trip<T>(value: &T) -> T
where
    T: Serialize + for<'de> Deserialize<'de>,
{
    serde_json::from_value(serde_json::to_value(value).unwrap()).unwrap()
}

fn account() -> MatrixAccount {
    MatrixAccount {
        user_id: "@alice:matrix.org".into(),
        homeserver: "https://matrix-client.matrix.org/".into(),
    }
}

fn password_login() -> MatrixPasswordLogin {
    MatrixPasswordLogin {
        login_id: "login-1".into(),
        user_id: "@alice:matrix.org".into(),
        password: SecretString::new("private-password-1"),
        device_name: "phone".into(),
    }
}

fn oidc_login() -> MatrixOidcLogin {
    MatrixOidcLogin {
        login_id: "login-1".into(),
        redirect_uri: "wonton://matrix-login".into(),
        device_name: "phone".into(),
    }
}

fn callback() -> MatrixOidcCallback {
    MatrixOidcCallback {
        login_id: "login-1".into(),
        callback_url: SecretString::new(
            "wonton://matrix-login?code=private-code-1&state=private-state-1",
        ),
    }
}

#[test]
fn login_records_round_trip_through_json() {
    assert_eq!(round_trip(&account()), account());
    assert_eq!(round_trip(&password_login()), password_login());
    assert_eq!(round_trip(&oidc_login()), oidc_login());
    assert_eq!(round_trip(&callback()), callback());
    let login = MatrixLoginViewModel {
        login_id: "login-1".into(),
        homeserver: "matrix.org".into(),
        methods: vec![MatrixLoginMethod::Password, MatrixLoginMethod::Oidc],
        state: MatrixLoginState::Ready,
    };
    assert_eq!(round_trip(&login), login);
    let servers = ServersViewModel {
        servers: vec![],
        adding: false,
        matrix_login: Some(login),
    };
    assert_eq!(round_trip(&servers), servers);
}

#[test]
fn login_states_round_trip_and_unknown_state_is_other() {
    for state in [
        MatrixLoginState::Discovering,
        MatrixLoginState::Ready,
        MatrixLoginState::Authenticating,
        MatrixLoginState::AwaitingOidc {
            authorization_url: SecretString::new(
                "https://matrix.org/authorize?state=private-state-1",
            ),
        },
        MatrixLoginState::Saving,
        MatrixLoginState::Succeeded {
            slot_id: "slot-1".into(),
            account: account(),
        },
        MatrixLoginState::Cancelled,
        MatrixLoginState::Failed {
            reason: MatrixLoginFailure::InvalidCredentials,
        },
        MatrixLoginState::Other,
    ] {
        assert_eq!(round_trip(&state), state);
    }
    assert_eq!(
        serde_json::from_str::<MatrixLoginState>(r#"{"state":"future","extra":true}"#).unwrap(),
        MatrixLoginState::Other
    );
}

#[test]
fn login_methods_round_trip_and_unknown_method_is_other() {
    for method in [
        MatrixLoginMethod::Password,
        MatrixLoginMethod::Oidc,
        MatrixLoginMethod::Other,
    ] {
        assert_eq!(round_trip(&method), method);
    }
    assert_eq!(
        serde_json::from_str::<MatrixLoginMethod>(r#""future_method""#).unwrap(),
        MatrixLoginMethod::Other
    );
}

#[test]
fn login_failures_round_trip_and_unknown_failure_is_other() {
    for failure in [
        MatrixLoginFailure::InvalidHomeserver,
        MatrixLoginFailure::Discovery,
        MatrixLoginFailure::UnsupportedMethod,
        MatrixLoginFailure::Transport,
        MatrixLoginFailure::InvalidCredentials,
        MatrixLoginFailure::OidcRejected,
        MatrixLoginFailure::InvalidCallback,
        MatrixLoginFailure::Expired,
        MatrixLoginFailure::Storage,
        MatrixLoginFailure::Other,
    ] {
        assert_eq!(round_trip(&failure), failure);
    }
    assert_eq!(
        serde_json::from_str::<MatrixLoginFailure>(r#""future_failure""#).unwrap(),
        MatrixLoginFailure::Other
    );
}

#[test]
fn login_actions_round_trip_through_json() {
    for action in [
        Action::StartMatrixLogin { homeserver: None },
        Action::StartMatrixLogin {
            homeserver: Some("matrix.org".into()),
        },
        Action::SubmitMatrixPassword {
            login: password_login(),
        },
        Action::StartMatrixOidc {
            login: oidc_login(),
        },
        Action::CompleteMatrixOidc {
            callback: callback(),
        },
        Action::CancelMatrixLogin {
            login_id: "login-1".into(),
        },
    ] {
        assert_eq!(round_trip(&action), action);
    }
}

#[test]
fn password_and_oidc_secrets_are_redacted_in_debug_not_serialization() {
    let password = password_login();
    let callback = callback();
    let state = MatrixLoginState::AwaitingOidc {
        authorization_url: SecretString::new("https://matrix.org/authorize?state=private-state-1"),
    };
    let login = MatrixLoginViewModel {
        login_id: "login-1".into(),
        homeserver: "matrix.org".into(),
        methods: vec![MatrixLoginMethod::Oidc],
        state,
    };
    let password_action = Action::SubmitMatrixPassword {
        login: password.clone(),
    };
    let callback_action = Action::CompleteMatrixOidc {
        callback: callback.clone(),
    };
    for debug in [
        format!("{password:?}"),
        format!("{callback:?}"),
        format!("{login:?}"),
        format!("{password_action:?}"),
        format!("{callback_action:?}"),
    ] {
        for secret in ["private-password-1", "private-code-1", "private-state-1"] {
            assert!(!debug.contains(secret), "{debug}");
        }
    }
    assert_eq!(
        round_trip(&password).password.expose(),
        "private-password-1"
    );
    assert_eq!(round_trip(&callback).callback_url, callback.callback_url);
    assert_eq!(round_trip(&login), login);
}
