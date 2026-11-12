//! 登录流程里在流程自己的运行时上跑的步骤，以及成功后的收尾。

use std::sync::{Arc, Weak};
use std::time::Instant;

use url::Url;
use wonton_matrix::{LoginError, OidcClient, OwnedUserId, PendingLogin, SavedSession, StoreConfig};
use wonton_protocol::{
    CoreError, MatrixAccount, MatrixLoginFailure, MatrixLoginState, SecretString,
};

use super::{Flow, MatrixLogins, OIDC_DEADLINE, Oidc, lock};
use crate::chat::{ChatStorage, StoredSession};
use crate::servers::ServerList;

/// OAuth 2.0 动态注册时报的客户端主页（MSC2966 的 `client_uri`）。要是公开能打开的页面，域名倒过来就是回调的 scheme。
pub(super) const CLIENT_URI: &str = "https://wontonapp.github.io";
const CLIENT_NAME: &str = "Wonton";

/// 登录用的存储：占的槽位里聊天以后要用的那个目录，登录得到的设备密钥直接落在那里。
pub(super) fn store(servers: &ServerList, slot: &str) -> Result<StoreConfig, CoreError> {
    let storage = ChatStorage::open(&servers.slot_dir(slot))?;
    Ok(StoreConfig::Sqlite {
        path: storage.store_dir(),
        passphrase: None,
    })
}

pub(super) async fn discover(
    logins: Arc<MatrixLogins>,
    flow: Arc<Flow>,
    input: String,
    store: StoreConfig,
) {
    // 测试里的 homeserver 只有 HTTP；产品构建只认 HTTPS。
    let allow_http = cfg!(feature = "test-hooks");
    match PendingLogin::discover(&input, &store, allow_http).await {
        Ok(pending) => {
            let methods = pending.methods().to_vec();
            let pending = Arc::new(pending);
            logins.update(&flow, |state| {
                state.view.methods = methods;
                state.view.state = MatrixLoginState::Ready;
                state.pending = Some(pending);
            });
        }
        Err(error) => failed(&logins, &flow, &error),
    }
}

pub(super) async fn password(
    logins: Arc<MatrixLogins>,
    flow: Arc<Flow>,
    pending: Arc<PendingLogin>,
    user_id: OwnedUserId,
    password: SecretString,
    device_name: String,
) {
    let signed_in = pending
        .login_password(&user_id, password.expose(), &device_name)
        .await;
    drop(password);
    match signed_in {
        Ok(saved) => save(&logins, &flow, pending, saved),
        Err(error) => failed(&logins, &flow, &error),
    }
}

pub(super) async fn oidc_url(
    logins: Arc<MatrixLogins>,
    flow: Arc<Flow>,
    pending: Arc<PendingLogin>,
    redirect: Url,
    device_name: String,
) {
    let client = OidcClient {
        client_uri: Url::parse(CLIENT_URI).expect("the client URI is valid"),
        client_name: CLIENT_NAME.to_owned(),
    };
    match pending.start_oidc(&redirect, &client).await {
        Ok(start) => {
            logins.update(&flow, |state| {
                state.oidc = Some(Oidc {
                    redirect,
                    state: start.state,
                    deadline: Instant::now() + OIDC_DEADLINE,
                    device_name,
                });
                state.view.state = MatrixLoginState::AwaitingOidc {
                    authorization_url: SecretString::new(start.authorization_url),
                };
            });
        }
        Err(error) => failed(&logins, &flow, &error),
    }
}

pub(super) async fn finish_oidc(
    logins: Arc<MatrixLogins>,
    flow: Arc<Flow>,
    pending: Arc<PendingLogin>,
    callback: Url,
    device_name: String,
) {
    match pending.finish_oidc(callback, &device_name).await {
        Ok(saved) => save(&logins, &flow, pending, saved),
        Err(error) => failed(&logins, &flow, &error),
    }
}

fn failed(logins: &MatrixLogins, flow: &Arc<Flow>, error: &LoginError) {
    tracing::info!(reason = ?error.reason, "Matrix sign-in failed: {}", error.detail);
    logins.fail(flow, error.reason);
}

/// 认证完成：先把登录状态写进占的槽位，再停掉登录用的运行时（存储关掉），最后在普通线程上加槽位、换过去。
fn save(
    logins: &Arc<MatrixLogins>,
    flow: &Arc<Flow>,
    pending: Arc<PendingLogin>,
    saved: SavedSession,
) {
    let account = MatrixAccount {
        user_id: saved.user_id().to_string(),
        homeserver: pending.homeserver(),
    };
    let saving = logins.update(flow, |state| {
        state.view.state = MatrixLoginState::Saving;
        state.pending = None;
    });
    drop(pending);
    if !saving {
        return;
    }
    let written = ChatStorage::open(&logins.servers.slot_dir(&flow.slot)).and_then(|storage| {
        storage.save_session(&StoredSession {
            server_id: String::new(),
            homeserver_url: account.homeserver.clone(),
            session: saved,
        })
    });
    if let Err(error) = written {
        tracing::warn!(%error, "could not save the Matrix sign-in");
        logins.fail(flow, MatrixLoginFailure::Storage);
        return;
    }
    let logins = logins.clone();
    let flow = flow.clone();
    // 这里就在登录的运行时上，不能等它停下；换作用域也会阻塞。
    std::thread::spawn(move || logins.finish(&flow, account));
}

impl MatrixLogins {
    fn finish(&self, flow: &Arc<Flow>, account: MatrixAccount) {
        if let Some(stopping) = flow.runtime.stopping() {
            let _ = stopping.join();
        }
        // 拿着流程锁换槽位：取消要么在这之前（不加槽位），要么在这之后（已经成功，取消不再有作用）。
        let current = self.lock();
        let still_current = current.as_ref().is_some_and(|c| Arc::ptr_eq(c, flow))
            && matches!(lock(&flow.state).view.state, MatrixLoginState::Saving);
        if !still_current {
            return;
        }
        let switched = self
            .switch
            .get()
            .and_then(Weak::upgrade)
            .ok_or_else(|| CoreError::NotReady {
                reason: "the client core is gone".to_owned(),
            })
            .and_then(|switch| switch.add_matrix_slot(&flow.slot, account.clone()));
        let mut state = lock(&flow.state);
        match switched {
            Ok(()) => {
                state.view.state = MatrixLoginState::Succeeded {
                    slot_id: flow.slot.clone(),
                    account,
                };
                self.servers.set_login(Some(state.view.clone()));
            }
            Err(error) => {
                tracing::warn!(%error, "could not add the Matrix account to the server list");
                state.view.state = MatrixLoginState::Failed {
                    reason: MatrixLoginFailure::Storage,
                };
                self.servers.set_login(Some(state.view.clone()));
                drop(state);
                if let Err(error) = self.servers.release_login_slot(&flow.slot) {
                    tracing::warn!(%error, "could not delete an unfinished Matrix sign-in");
                }
            }
        }
    }
}
