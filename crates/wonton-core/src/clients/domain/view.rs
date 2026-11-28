//! 设置页看到的状态：证书、CAA 结果和建议记录。

use std::io::Cursor;
use std::sync::atomic::Ordering;

use anyhow::Result;
use wonton_protocol::{CaaState, CertificateState, DomainSettings, LanRecordState};

use super::{Domain, Store};

impl Domain {
    /// 设置页看到的整份状态。
    pub async fn view(&self) -> Result<DomainSettings> {
        let store = self.effective()?;
        let fixed = self.fixed_suffix()?;
        let caa = self.caa_state(&store).await;
        // 已经限定到本机账号了就不再给建议，页面不用多显示一条。
        let caa_record = match caa {
            CaaState::Bound => None,
            _ => self.caa_record(&store).await,
        };
        Ok(DomainSettings {
            suffix: fixed.clone().or(store.suffix.clone()),
            suffix_editable: fixed.is_none(),
            home: None,
            tail: None,
            provider: store.provider.clone(),
            credentials_set: store.has_credentials(),
            terms_accepted: store.terms_accepted,
            certificate: self.certificate_state(&store),
            caa,
            caa_record,
            // `*.lan` 的记录和目标由网关从局域网检测的同一份状态覆盖，这里先给空。
            lan_record: LanRecordState::Unknown,
            lan_target: None,
        })
    }

    fn certificate_state(&self, store: &Store) -> CertificateState {
        if self.pending.load(Ordering::SeqCst) > 0 {
            return CertificateState::Issuing;
        }
        let now = now_ms();
        match self.not_after_ms() {
            Some(not_after) if not_after > now => match &store.last_error {
                Some(error) => CertificateState::Renewing {
                    not_after_ms: not_after,
                    error: error.clone(),
                },
                None => CertificateState::Valid {
                    not_after_ms: not_after,
                },
            },
            Some(not_after) => CertificateState::Expired {
                not_after_ms: not_after,
                error: store
                    .last_error
                    .clone()
                    .unwrap_or_else(|| "证书已过期".into()),
            },
            None => match &store.last_error {
                Some(error) => CertificateState::Expired {
                    not_after_ms: 0,
                    error: error.clone(),
                },
                None => CertificateState::None,
            },
        }
    }

    /// 已发布证书的到期时间（Unix 毫秒）。取第一张不是 CA 的叶证书：自己拼的链条可能把 CA 放前面。
    pub(super) fn not_after_ms(&self) -> Option<u64> {
        let pem = std::fs::read(self.install_dir.join("entry/certs/site.pem")).ok()?;
        let mut fallback = None;
        for der in rustls_pemfile::certs(&mut Cursor::new(pem)).flatten() {
            let Ok((_, certificate)) = x509_parser::parse_x509_certificate(&der) else {
                continue;
            };
            let seconds = u64::try_from(certificate.validity().not_after.timestamp()).ok()?;
            let millis = seconds.checked_mul(1000);
            let ca = certificate
                .basic_constraints()
                .ok()
                .flatten()
                .is_some_and(|constraints| constraints.value.ca);
            if ca {
                fallback.get_or_insert(millis);
            } else {
                return millis;
            }
        }
        fallback.flatten()
    }

    /// 当前 CA 的身份和本机账号：CAA 记录和判定都要这两样。
    async fn caa_binding(&self, store: &Store) -> Option<(String, String)> {
        let acme = self.acme().ok()?;
        // 认不出当前 CA 的身份就不猜：显示 Unknown，签发路径会当场拒绝。
        let identity = crate::entry_dns::caa_identity(&acme.directory_url)?;
        let manager = self.manager(store).ok()?;
        let account = manager.account_uri().ok()?;
        Some((account, identity))
    }

    async fn caa_state(&self, store: &Store) -> CaaState {
        let Some(suffix) = store.suffix.as_deref() else {
            return CaaState::Unknown;
        };
        let Some((account, identity)) = self.caa_binding(store).await else {
            return CaaState::Unknown;
        };
        let Ok(acme) = self.acme() else {
            return CaaState::Unknown;
        };
        let key = format!("{suffix}|{account}|{identity}");
        if let Some((at, cached_key, state)) = &*self.caa_cache.lock().await
            && cached_key == &key
            && at.elapsed() < std::time::Duration::from_secs(60)
        {
            return *state;
        }
        let Ok(records) = crate::entry_dns::caa_issues(suffix, &acme.resolvers).await else {
            return CaaState::Unknown;
        };
        let state = crate::entry_dns::caa_state(&records, &account, &identity);
        *self.caa_cache.lock().await = Some((std::time::Instant::now(), key, state));
        state
    }

    /// 照抄就能变成 `Bound` 的那条 `issue` 记录（RFC 8657 限定到本机账号）。
    async fn caa_record(&self, store: &Store) -> Option<String> {
        let suffix = store.suffix.as_deref()?;
        let (account, identity) = self.caa_binding(store).await?;
        Some(format!(
            "{suffix}. CAA 0 issue \"{identity}; accounturi={account}; validationmethods=dns-01\""
        ))
    }
}

pub(in crate::clients) fn now_ms() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|elapsed| elapsed.as_millis() as u64)
        .unwrap_or(0)
}
