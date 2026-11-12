import LocalAuthentication

/// 系统验证（ADR-0102）：面容、触控 ID 或设备密码都算过。看恢复密钥和 App 锁共用这一处，两端同一套机制。
/// 验证只对这一次有效；取消和失败都不算过，系统界面自己说明了原因，这里不再另弹提示。
enum SystemAuth {
    /// 设备有没有能用的验证手段。没设设备密码时没有（`passcodeNotSet`）；生物识别坏了这类不算没有，
    /// 密码还在，系统提示里会说原因。
    static func canAuthenticate() -> Bool {
        !passcodeNotSet()
    }

    /// 设备没设设备密码。恢复密钥页此时不拦，App 锁靠它标出“去系统设置”。
    static func passcodeNotSet() -> Bool {
        let context = LAContext()
        var error: NSError?
        _ = context.canEvaluatePolicy(.deviceOwnerAuthentication, error: &error)
        return (error as? LAError)?.code == .passcodeNotSet
    }

    /// 弹系统验证，通过返回 true。页面看得见时才调，否则系统提示会被收走。
    static func authenticate(reason: String) async -> Bool {
        let context = LAContext()
        var error: NSError?
        guard context.canEvaluatePolicy(.deviceOwnerAuthentication, error: &error) else { return false }
        return (try? await context.evaluatePolicy(.deviceOwnerAuthentication, localizedReason: reason)) == true
    }
}
