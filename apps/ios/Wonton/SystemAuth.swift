import LocalAuthentication

/// 系统验证（ADR-0102）：面容、触控 ID 或设备密码都算过。看恢复密钥和 App 锁共用这一处，两端同一套机制。
/// 验证只对这一次有效；取消不算过也不算失败，验证用不了按失败即停给系统原因。
enum SystemAuth {
    /// 设备没设设备密码。恢复密钥页此时不拦，App 锁靠它标出“去系统设置”。
    static func passcodeNotSet() -> Bool {
        let context = LAContext()
        var error: NSError?
        _ = context.canEvaluatePolicy(.deviceOwnerAuthentication, error: &error)
        return (error as? LAError)?.code == .passcodeNotSet
    }

    /// 设备有没有能用的验证手段。没设设备密码时没有；生物识别坏了这类不算没有，密码还在。
    static func canAuthenticate() -> Bool {
        !passcodeNotSet()
    }

    /// `canEvaluatePolicy` 失败时系统给的原因，验证能弹就没有。
    static func failureDetail() -> String? {
        let context = LAContext()
        var error: NSError?
        let canEvaluate = context.canEvaluatePolicy(.deviceOwnerAuthentication, error: &error)
        return canEvaluate ? nil : error?.localizedDescription
    }

    /// 弹系统验证。页面看得见时才调，否则系统提示会被收走。
    static func authenticate(reason: String) async -> Outcome {
        let context = LAContext()
        var error: NSError?
        guard context.canEvaluatePolicy(.deviceOwnerAuthentication, error: &error) else {
            // 没设密码不该走到这（调用方初始就直显了）；真发生了当作不用验证。
            if (error as? LAError)?.code == .passcodeNotSet { return .passed }
            return .unavailable(error?.localizedDescription)
        }
        do {
            if try await context.evaluatePolicy(.deviceOwnerAuthentication, localizedReason: reason) {
                return .passed
            }
            return .cancelled
        } catch {
            // 用户取消（含系统和 App 代取消）不算过也不算失败，不提示；其余失败给系统原因。
            switch (error as? LAError)?.code {
            case .userCancel, .systemCancel, .appCancel:
                return .cancelled
            default:
                return .unavailable(error.localizedDescription)
            }
        }
    }

    /// 验证的最终结果：只有 [Outcome.passed] 才算过；[Outcome.cancelled] 不提示；
    /// [Outcome.unavailable] 是验证用不了或出错，按失败即停给系统原因。
    enum Outcome {
        case passed
        case cancelled
        case unavailable(String?)
    }
}
