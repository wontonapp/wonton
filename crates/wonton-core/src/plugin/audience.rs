use wonton_protocol::{PluginAudienceNotice, PluginAudienceView};

use super::manifest::{Accounts, Manifest};

pub(super) fn view(manifest: &Manifest) -> PluginAudienceView {
    PluginAudienceView {
        choices: Vec::new(),
        notice: (manifest.webui.as_ref().and_then(|webui| webui.accounts)
            != Some(Accounts::PerUser))
        .then_some(PluginAudienceNotice::SharedData),
    }
}
