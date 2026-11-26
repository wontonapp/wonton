//! 测试钩子：登记下一次“悄悄”清空的时间线，`apply` 遇到清空批时消费掉。

use wonton_matrix::{ChatRoom, OwnedRoomId};

/// 测试钩子：登记下一条要“悄悄”清空的时间线，见 [`super::diffs::apply`] 里清空批的分支。
#[cfg(feature = "test-hooks")]
pub(crate) fn mark_silent_clear(
    room_id: OwnedRoomId,
    thread_root: Option<wonton_matrix::OwnedEventId>,
) {
    silent_clears()
        .lock()
        .expect("安静清空的记录锁")
        .push((room_id, thread_root));
}

#[cfg(feature = "test-hooks")]
type SilentClears = std::sync::Mutex<Vec<(OwnedRoomId, Option<wonton_matrix::OwnedEventId>)>>;

#[cfg(feature = "test-hooks")]
fn silent_clears() -> &'static SilentClears {
    static CLEARS: std::sync::LazyLock<SilentClears> = std::sync::LazyLock::new(Default::default);
    &CLEARS
}

/// 这条时间线登记过安静清空就消费掉；`apply` 据此不发清空。
#[cfg(feature = "test-hooks")]
pub(super) fn take_silent_clear(room: &ChatRoom) -> bool {
    let key = (
        room.room_id().to_owned(),
        room.thread_root().map(ToOwned::to_owned),
    );
    let mut clears = silent_clears().lock().expect("安静清空的记录锁");
    let Some(index) = clears.iter().position(|registered| *registered == key) else {
        return false;
    };
    clears.swap_remove(index);
    true
}
