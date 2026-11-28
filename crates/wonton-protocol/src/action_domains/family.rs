use serde::{Deserialize, Serialize};

/// 家庭成员角色的操作。自己的头像不走动作：资料页本来就 await 地读写 `me()`、`saveMe`，
/// 头像也一样由 `set_my_avatar` 这类直接返回 `Me` 的调用做，失败原因由 [`MyAvatarProblem`](crate::MyAvatarProblem) 分类。
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "uniffi", derive(uniffi::Enum))]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum FamilyAction {
    /// 打开成员页：取自己、成员、角色和加入请求，并在页面开着时跟上新的请求。
    RefreshFamily,
    /// 离开成员页，停掉刷新。
    LeaveFamily,
    /// 邀请一个人：记成“已邀请”并出一张码，码放进 [`FamilyViewModel::invite`](crate::FamilyViewModel::invite)。
    /// 只有主人能填 `role_id`；管理员留空，新人拿默认角色。
    InviteMember {
        display_name: String,
        #[cfg_attr(feature = "uniffi", uniffi(default = None))]
        username: Option<String>,
        private_bot: bool,
        #[cfg_attr(feature = "uniffi", uniffi(default = None))]
        role_id: Option<String>,
    },
    /// 重发邀请码，之前的不再有效；新码放进 `invite`。
    ResendMemberCode { username: String },
    /// 作废这个人的邀请码，不出新的。
    RevokeMemberCode { username: String },
    /// 移除一个人：吊销设备、停用账号；`delete_home` 删私人空间，`keep_bot_data` 留 Bot 的数据。
    RemoveMember {
        username: String,
        delete_home: bool,
        keep_bot_data: bool,
    },
    /// 关掉刚出的邀请码那一屏。
    DismissMemberInvite,
    /// 重新取一遍加入请求。
    RefreshJoinRequests,
    /// 同意一条加入请求：设备登记在这个人名下。
    ApproveJoinRequest { id: String },
    /// 拒绝一条加入请求：连同邀请码一起作废。
    RejectJoinRequest { id: String },
    /// 取全部角色和默认邀请角色；只有主人能用。
    RefreshRoles,
    /// 新建角色，名字和权限项见 [`RoleDraft`](crate::RoleDraft)；成功后进角色列表。
    CreateRole { draft: crate::RoleDraft },
    /// 改一个角色的名字和权限项；ID 和模板标记不变。
    UpdateRole {
        role_id: String,
        draft: crate::RoleDraft,
    },
    /// 删一个没人用的角色；默认邀请角色和在用的角色删不掉。
    DeleteRole { role_id: String },
    /// 换别的管理员发邀请时新人拿的角色。
    SetDefaultRole { role_id: String },
    /// 给一个家人换角色；主人以外的人发了邀请也一样。
    AssignMemberRole { username: String, role_id: String },
    /// 给好友名单里的一个人换角色，user_id 是完整的 Matrix 用户 ID。
    AssignFriendRole { user_id: String, role_id: String },
}
