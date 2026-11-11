//! 模型服务商这条线按请求的语言写（ADR-0043）：同一场景用 `ja` 和 `fr-FR` 请求，分别拿到 ja 和
//! en 的那条；期望值由目录本身格式化，再断言两者不同，防止协商被绕过。

use super::super::languages::Languages;
use super::manager::manager_with;
use crate::l10n::CATALOG;

fn langs(header: &str) -> Languages {
    Languages::from_header(Some(header))
}

fn expected(tag: &str, message: &str) -> String {
    let lang = CATALOG.negotiate(&[tag]);
    CATALOG.format(lang, message, &[])
}

/// 没装模型网关时列出服务商：失败原因按请求的语言写。
#[tokio::test]
async fn model_providers_absent_reason_follows_the_requested_language() {
    let (_docker, engine) = super::super::fake_docker::FakeDocker::start().await;
    let dir = tempfile::tempdir().unwrap();
    let m = manager_with(engine, &dir);
    for (header, tag) in [("ja", "ja"), ("fr-FR", "en")] {
        let error = m.model_providers(&langs(header)).await.unwrap_err();
        assert_eq!(
            error.to_string(),
            expected(tag, "plugin-settings-models-gateway-absent"),
            "{header}: {error}"
        );
    }
}
