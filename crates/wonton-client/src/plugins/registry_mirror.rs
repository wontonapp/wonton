//! 镜像加速设置和测速；请求绑定发起时的服务器，保存使之前的读取失效。

use std::sync::Arc;
use std::sync::atomic::Ordering;

use wonton_l10n::l10n;
use wonton_protocol::{
    CoreError, PluginFailure, PluginProblem, RegistryMirrorSettings, RegistryMirrorTestRequest,
    RegistryMirrorTests, RegistryMirrorUpdate,
};

use super::api::{CoreApi, READ_TIMEOUT};
use super::{PluginsLink, lock};
use crate::l10n::{CATALOG, language};

const PATH: [&str; 1] = ["registry-mirror"];
const TEST_PATH: [&str; 2] = ["registry-mirror", "test"];

#[derive(Default)]
pub(super) struct RegistryLocal {
    settings_epoch: u64,
    test_epoch: u64,
}

struct Request {
    generation: u64,
    server: Option<String>,
    epoch: u64,
    api: Result<CoreApi, PluginProblem>,
}

/// 索引的地址列表读不懂时，设置照常显示，问题挂在 `load_problem` 上让人看见（表单里带重试）。
fn choices_problem(settings: &RegistryMirrorSettings) -> Option<PluginProblem> {
    settings
        .choices_problem
        .as_ref()
        .map(|message| PluginProblem {
            reason: PluginFailure::Upstream,
            message: message.clone(),
            min_version: None,
            settings: None,
        })
}

impl PluginsLink {
    fn registry_request(&self, epoch: u64) -> Request {
        Request {
            generation: self.generation.load(Ordering::SeqCst),
            server: self.home.view().server_id,
            epoch,
            api: self.api(),
        }
    }

    pub(crate) fn refresh_registry_mirror(self: &Arc<Self>) {
        let request = {
            let mut local = lock(&self.registry);
            if self.view.borrow().registry_mirror.saving {
                return;
            }
            local.settings_epoch += 1;
            let request = self.registry_request(local.settings_epoch);
            self.view.send_modify(|view| {
                view.registry_mirror.loading = true;
                view.registry_mirror.load_problem = None;
            });
            request
        };
        let link = Arc::clone(self);
        self.runtime.spawn(async move {
            let result: Result<RegistryMirrorSettings, _> = match &request.api {
                Ok(api) => api.get(&PATH, &[], READ_TIMEOUT).await,
                Err(problem) => Err(problem.clone()),
            };
            let local = lock(&link.registry);
            if local.settings_epoch != request.epoch
                || !link.is_current_home(request.generation, &request.server)
            {
                return;
            }
            link.view.send_modify(|view| {
                let vm = &mut view.registry_mirror;
                vm.loading = false;
                match result {
                    Ok(settings) => {
                        vm.load_problem = choices_problem(&settings);
                        vm.settings = Some(settings);
                    }
                    Err(problem) => vm.load_problem = Some(problem),
                }
            });
        });
    }

    pub(crate) fn save_registry_mirror(self: &Arc<Self>, mirror: String) -> Result<(), CoreError> {
        let request = {
            let mut local = lock(&self.registry);
            if self.view.borrow().registry_mirror.saving {
                return Err(CoreError::NotReady {
                    reason: l10n!(CATALOG, language(), "plugins-registry-save-in-progress"),
                });
            }
            local.settings_epoch += 1;
            let request = self.registry_request(local.settings_epoch);
            self.view.send_modify(|view| {
                let vm = &mut view.registry_mirror;
                vm.saving = true;
                vm.loading = false;
                vm.save_problem = None;
            });
            request
        };
        let update = RegistryMirrorUpdate {
            mirror: (!mirror.is_empty()).then_some(mirror),
        };
        let link = Arc::clone(self);
        self.runtime.spawn(async move {
            let result: Result<RegistryMirrorSettings, _> = match &request.api {
                Ok(api) => api.put(&PATH, &update, READ_TIMEOUT).await,
                Err(problem) => Err(problem.clone()),
            };
            let local = lock(&link.registry);
            if local.settings_epoch != request.epoch
                || !link.is_current_home(request.generation, &request.server)
            {
                return;
            }
            link.view.send_modify(|view| {
                let vm = &mut view.registry_mirror;
                vm.saving = false;
                match result {
                    Ok(settings) => {
                        vm.load_problem = choices_problem(&settings);
                        vm.settings = Some(settings);
                    }
                    Err(mut problem) => {
                        if problem.reason == PluginFailure::TransportUnavailable {
                            problem.message = l10n!(
                                CATALOG,
                                language(),
                                "plugins-registry-save-uncertain",
                                reason = problem.message
                            );
                        }
                        vm.save_problem = Some(problem);
                    }
                }
            });
        });
        Ok(())
    }

    pub(crate) fn test_registry_mirrors(self: &Arc<Self>) -> Result<(), CoreError> {
        let (request, body) = {
            let mut local = lock(&self.registry);
            let body = {
                let view = self.view.borrow();
                let Some(settings) = &view.registry_mirror.settings else {
                    return Err(CoreError::NotReady {
                        reason: l10n!(CATALOG, language(), "plugins-registry-not-loaded"),
                    });
                };
                let mut mirrors = Vec::new();
                for mirror in settings.choices.iter().chain(settings.mirror.iter()) {
                    if !mirrors.contains(mirror) {
                        mirrors.push(mirror.clone());
                    }
                }
                RegistryMirrorTestRequest { mirrors }
            };
            local.test_epoch += 1;
            let request = self.registry_request(local.test_epoch);
            self.view.send_modify(|view| {
                view.registry_mirror.testing = true;
                view.registry_mirror.test_problem = None;
            });
            (request, body)
        };
        let link = Arc::clone(self);
        self.runtime.spawn(async move {
            let result: Result<RegistryMirrorTests, _> = match &request.api {
                Ok(api) => api.post(&TEST_PATH, &body, READ_TIMEOUT).await,
                Err(problem) => Err(problem.clone()),
            };
            let local = lock(&link.registry);
            if local.test_epoch != request.epoch
                || !link.is_current_home(request.generation, &request.server)
            {
                return;
            }
            // 测速响应不是设置，不用它重写地址，也不再发可能读到保存前状态的 GET。
            link.view.send_modify(|view| {
                let vm = &mut view.registry_mirror;
                vm.testing = false;
                match result {
                    Ok(tests) => {
                        if let Some(settings) = &mut vm.settings {
                            settings.docker_hub.clone_from(&tests.docker_hub);
                            settings.recommended = tests.docker_hub.as_ref().is_some_and(|probe| {
                                probe.problem.is_some()
                                    || probe.latency_ms.is_some_and(|ms| ms > 5_000)
                            });
                        }
                        vm.tests = Some(tests);
                    }
                    Err(problem) => vm.test_problem = Some(problem),
                }
            });
        });
        Ok(())
    }
}
