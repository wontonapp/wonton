//! `acme-dns` 服务商：注册两个子域，凭据分别当 apex 和 LAN 的。

use super::*;

const ACME_DNS: &str =
    "joohoi/acme-dns:v1.0@sha256:08435bf0c6cfe7c29816ff48beabcc333743b5bb060cae3867777d9b0d956517";

pub(super) async fn up(case: &mut Case) {
    let acme_dns_image = ensure_image(&case.engine, &wonton_test_images::image(ACME_DNS)).await;
    let container = case.names.oneshot_container("acme-dns");
    let config = case.dir.path().join("acme-dns/config.cfg");
    let pebble_ip = case.pebble_ip.clone();
    write(
        &config,
        &format!(
            "[general]\nlisten = \"0.0.0.0:53\"\nprotocol = \"both4\"\ndomain = \"auth.{ZONE}\"\nnsname = \"auth.{ZONE}\"\nnsadmin = \"admin.{ZONE}\"\nrecords = [\"auth.{ZONE}. A {pebble_ip}\", \"auth.{ZONE}. NS auth.{ZONE}.\"]\ndebug = false\n[database]\nengine = \"sqlite3\"\nconnection = \"/var/lib/acme-dns/acme-dns.db\"\n[api]\nip = \"0.0.0.0\"\nport = \"8080\"\ntls = \"none\"\ndisable_registration = false\ncorsorigins = []\nuse_header = false\n[logconfig]\nloglevel = \"info\"\nlogtype = \"stdout\"\nlogformat = \"text\"\n"
        ),
    );
    let address = format!("{}.8", case.base);
    start(
        &case.names,
        &case.network,
        &container,
        None,
        &address,
        &[],
        &[(config, "/etc/acme-dns/config.cfg".into())],
        &acme_dns_image,
        "/root/acme-dns",
        &[],
    );
    wait_http(&format!("http://{address}:8080/health"), None)
        .await
        .unwrap();
    let client = reqwest::Client::new();
    let first: Value = client
        .post(format!("http://{address}:8080/register"))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    let second: Value = client
        .post(format!("http://{address}:8080/register"))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    let credentials = |value: &Value| crate::entry_certificate::AcmeDnsCredentials {
        username: value["username"].as_str().unwrap().to_owned(),
        password: value["password"].as_str().unwrap().to_owned(),
        subdomain: value["subdomain"].as_str().unwrap().to_owned(),
        fulldomain: value["fulldomain"].as_str().unwrap().to_owned(),
    };
    case.acme_dns_ip = Some(address.clone());
    case.acme_dns_container = Some(container);
    case.acme_dns = Some((
        format!("http://{address}:8080"),
        credentials(&first),
        credentials(&second),
    ));
}
