import Foundation
import WontonClient

/// 聊天列表顶部“恢复密钥”横幅关掉后的记忆：同类提醒关了就不再挂，设置里的“聊天记录备份”还在。
/// 只关这条横幅，不动核心的备份状态；核心换一类提醒（保存↔输入）时照常出现。
struct BackupReminderDismissals {
    private let defaults: UserDefaults

    init(defaults: UserDefaults = .standard) {
        self.defaults = defaults
    }

    func dismissed(_ reminder: BackupReminder) -> Bool {
        let names = defaults.stringArray(forKey: Self.storageKey) ?? []
        return names.contains(name(of: reminder))
    }

    func dismiss(_ reminder: BackupReminder) {
        var names = Set(defaults.stringArray(forKey: Self.storageKey) ?? [])
        names.insert(name(of: reminder))
        defaults.set(names.sorted(), forKey: Self.storageKey)
    }

    /// 存的是核心给的提醒类别，用协议的 snake_case 名，跟平台枚举拼写解耦。
    private func name(of reminder: BackupReminder) -> String {
        switch reminder {
        case .saveRecoveryKey: return "save_recovery_key"
        case .enterRecoveryKey: return "enter_recovery_key"
        case .continueOnOtherDevice: return "continue_on_other_device"
        case .other: return "other"
        }
    }

    private static let storageKey = "backupReminderDismissed"
}
