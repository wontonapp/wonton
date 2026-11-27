//! 建相册、共享相册时能选谁：本机家人里已加入的、不含自己。名单读核心的成员接口，和成员页同一份；
//! 那个接口要“管理成员”权限，没有就如实给 `permission_denied`，不拿别的名单凑。

use std::sync::Arc;

use serde::Deserialize;
use wonton_protocol::media::MediaFailure;
use wonton_protocol::media::library_view::PhotoShareTarget;
use wonton_protocol::{ME_PATH, MEMBERS_PATH, Member, MemberList, MemberStatus};

use super::{PhotosLink, Section};
use crate::media_link::MediaApi;

/// 只要用户名：`Me` 的其余字段与这里无关，少一个字段也不该让名单取不成。
#[derive(Deserialize)]
struct Me {
    username: String,
}

impl PhotosLink {
    pub(super) fn share_targets(self: &Arc<Self>) {
        let Some(ticket) = self.start(Section::ShareTargets, |_, view| {
            view.share_targets.loading = true;
            true
        }) else {
            return;
        };
        let api = MediaApi::connect(&self.home);
        let link = Arc::clone(self);
        self.runtime.spawn(async move {
            let result = match api {
                Ok(api) => fetch(&api).await,
                Err(failure) => Err(failure),
            };
            link.apply(&ticket, |_, view| {
                let targets = &mut view.share_targets;
                targets.loading = false;
                match result {
                    Ok(people) => {
                        targets.people = people;
                        targets.problem = None;
                    }
                    // 名单不确定就不给选：留着旧名单会让人共享给已经不在的家人。
                    Err(failure) => {
                        targets.people.clear();
                        targets.problem = Some(failure);
                    }
                }
            });
        });
    }
}

async fn fetch(api: &MediaApi) -> Result<Vec<PhotoShareTarget>, MediaFailure> {
    let me: Me = api.get(ME_PATH).await?;
    let list: MemberList = api.get(MEMBERS_PATH).await?;
    Ok(shareable(&me.username, list.members))
}

/// 已加入的、不是自己的，按名单原来的顺序。
fn shareable(me: &str, members: Vec<Member>) -> Vec<PhotoShareTarget> {
    members
        .into_iter()
        .filter(|member| member.status == MemberStatus::Joined && member.username != me)
        .map(|member| PhotoShareTarget {
            username: member.username,
            display_name: member.display_name,
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use wonton_protocol::{Member, MemberStatus};

    use super::shareable;

    fn member(username: &str, status: MemberStatus) -> Member {
        Member {
            username: username.to_owned(),
            user_id: format!("@{username}:example.com"),
            display_name: username.to_uppercase(),
            email: None,
            status,
            role_id: "child".to_owned(),
            private_bot: None,
            pairing_code_expires_at_ms: None,
        }
    }

    #[test]
    fn only_joined_people_other_than_me() {
        let members = vec![
            member("me", MemberStatus::Joined),
            member("mom", MemberStatus::Joined),
            member("aunt", MemberStatus::Invited),
            member("gone", MemberStatus::Removed),
            member("kid", MemberStatus::Joined),
        ];
        let names: Vec<_> = shareable("me", members)
            .into_iter()
            .map(|person| (person.username, person.display_name))
            .collect();
        assert_eq!(
            names,
            [
                ("mom".to_owned(), "MOM".to_owned()),
                ("kid".to_owned(), "KID".to_owned())
            ]
        );
    }
}
