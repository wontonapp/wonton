//! 任务号、主人和设备标识。

use uuid::Uuid;

/// 任务号，由服务器核发。
pub type TaskId = Uuid;
/// 任务或会话的主人（MXID）。
pub type Person = String;
/// 设备标识。
pub type DeviceId = String;

/// 发一个新任务号。
pub fn new_task_id() -> TaskId {
    TaskId::new_v4()
}
