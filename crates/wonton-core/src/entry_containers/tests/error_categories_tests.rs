//! 错误按原因归类。

use super::super::EntryError;

/// 类别不能都报成 docker_unavailable。
#[test]
fn categories_follow_the_cause() {
    use crate::plugin::docker::EngineError;
    let category = |error: EntryError| error.category();
    assert_eq!(
        category(EntryError::Docker(EngineError::PortTaken("x".into()))),
        "conflict"
    );
    assert_eq!(
        category(EntryError::Docker(EngineError::Rejected {
            context: "x".into(),
            message: "y".into()
        })),
        "invalid_input"
    );
    assert_eq!(
        category(EntryError::Docker(EngineError::NotFound("x".into()))),
        "not_found"
    );
    assert_eq!(
        category(EntryError::Docker(EngineError::Pull {
            image: "i".into(),
            message: "m".into()
        })),
        "source_unavailable"
    );
    assert_eq!(
        category(EntryError::Docker(EngineError::Unavailable("x".into()))),
        "docker_unavailable"
    );
}
