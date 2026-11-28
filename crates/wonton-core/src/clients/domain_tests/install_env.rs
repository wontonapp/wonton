//! 装机环境的初值与凭据文件：设置种子、变量名解析、引号转义、原样交给 lego。

use super::*;

/// 装机时的环境变量是初值：设置文件还不存在时，服务商、凭据和订户协议的勾选从这里来。
#[tokio::test]
async fn env_install_seeds_settings() {
    let dir = tempfile::tempdir().unwrap();
    let env_file = dir.path().join("provider.env");
    fs::write(&env_file, "DESEC_TOKEN='seeded'\n").unwrap();
    let path = dir.path().join("entry/settings.json");
    fs::create_dir_all(path.parent().unwrap()).unwrap();
    let settings = serde_json::json!({
        "domain": "abcdefgh.example.test",
        "service_url": null,
        "service_ca": null,
        "relay_ca": null,
        "registration": null,
        "acme_directory": "https://pebble.test:14000/dir",
        "acme_ca": null,
        "acme_resolvers": [],
        "acme_email": null,
        "acme_accept_terms": true,
        "acme_network": "bridge",
        "dns_provider": "desec",
        "dns_env_file": env_file,
        "external_certificate": false
    });
    fs::write(&path, serde_json::to_vec(&settings).unwrap()).unwrap();
    let view = domain(dir.path()).view().await.unwrap();
    assert_eq!(view.provider.as_deref(), Some("desec"));
    assert!(view.credentials_set);
    assert!(view.terms_accepted);
    assert!(!dir.path().join("entry/domain.json").exists());
}
#[test]
fn install_env_file_names_are_parsed_but_values_pass_through_untouched() {
    // 装机给的 dotenv 原样交给 lego：godotenv 的双引号、转义、展开语义归它，我们只认变量名
    // 做存在性检查。`export ` 前缀、双引号值、注释都不能把名字认错或把值改掉。
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("dns.env");
    let original = b"\
CF_DNS_API_TOKEN=\"abc123\"\n\
export DESEC_TOKEN=abc\n\
export \t HTTPREQ_ENDPOINT='http://x'\n\
# comment\n\
\n\
EAAPI_KEY=\n\
WEIRD=value;not-params\n";
    fs::write(&path, original).unwrap();
    let names = dotenv_names(&path).unwrap();
    assert_eq!(
        names,
        [
            "CF_DNS_API_TOKEN",
            "DESEC_TOKEN",
            "HTTPREQ_ENDPOINT",
            "EAAPI_KEY",
            "WEIRD"
        ]
        .into_iter()
        .map(str::to_owned)
        .collect::<std::collections::BTreeSet<_>>()
    );
    assert_eq!(fs::read(&path).unwrap(), original, "文件必须原样保留");
    let empty = tempfile::tempdir().unwrap();
    let path = empty.path().join("empty.env");
    fs::write(&path, "# 只有注释\n").unwrap();
    assert!(dotenv_names(&path).is_err(), "一个变量名都没有要当场报错");
}
#[test]
fn dotenv_values_quote_metacharacters_for_godotenv() {
    // App 提交的凭据由我们写成 godotenv：单引号是字面量，`$`、反斜杠和空格都进单引号，
    // lego 实收的值和主人填的一致。
    assert_eq!(dotenv_value("plain-token_1"), "plain-token_1");
    assert_eq!(dotenv_value("x_./:+-y"), "x_./:+-y");
    assert_eq!(dotenv_value("user$TOKEN"), "'user$TOKEN'");
    assert_eq!(dotenv_value("a\\b$c"), "'a\\b$c'");
    assert_eq!(dotenv_value("with space"), "'with space'");
    assert_eq!(dotenv_value(""), "''");
}
/// 装机给的凭据文件原样传给 lego，不解析值：manager 挂的是原文件，App 没提交新凭据时一直用它。
#[tokio::test]
async fn install_env_file_is_passed_to_lego_verbatim() {
    let dir = tempfile::tempdir().unwrap();
    let env_file = dir.path().join("provider.env");
    let original = b"export CF_DNS_API_TOKEN=\"tok$en\"\nCF_ZONE=\"a\\\\b\"\n";
    fs::write(&env_file, original).unwrap();
    settings_with_provider(dir.path(), &env_file);
    let clients = domain(dir.path());
    let store = clients.effective().unwrap();
    assert!(store.credentials.is_empty(), "值不进设置，原样留给 lego");
    assert_eq!(store.env_file.as_deref(), Some(env_file.as_path()));
    let view = clients.view().await.unwrap();
    assert!(view.credentials_set, "装机文件算凭据已备");
    let manager = clients.manager(&store).unwrap();
    assert_eq!(
        manager.config.dns_env_file.as_deref(),
        Some(env_file.as_path()),
        "lego 拿到的必须是原文件，不是重写出来的副本"
    );
    assert_eq!(manager.config.dns_provider.as_deref(), Some("httpreq"));
    assert_eq!(fs::read(&env_file).unwrap(), original, "原文件不能被动过");

    // 同服务商保存（凭据留空）不该要求重新提交凭据，装机文件继续有效。
    // 自己域名的安装 provider 必须是真实代码（家名安装才允许空），这里保持 httpreq。
    let mut put = request(Some("abcdefgh.example.test"));
    put.credentials.clear();
    assert!(clients.save(put).await.is_ok());
    let store = clients.effective().unwrap();
    assert_eq!(store.env_file.as_deref(), Some(env_file.as_path()));

    // 提交了新凭据：改用提交值，装机文件退场，lego 改挂我们写出的 dns.env。
    clients
        .save(request(Some("abcdefgh.example.test")))
        .await
        .unwrap();
    let store = clients.effective().unwrap();
    assert!(store.env_file.is_none());
    let manager = clients.manager(&store).unwrap();
    assert_eq!(
        manager.config.dns_env_file.as_deref(),
        Some(dir.path().join(DNS_ENV_FILE).as_path())
    );
}
