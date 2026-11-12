import Observation
import SwiftUI
import WontonClient

/// App 锁和核心之间的桥：持有核心推来的 ViewModel，转发开关和验证结果。
@MainActor
@Observable
final class AppLockModel {
    private(set) var viewModel: AppLockViewModel
    /// 核心拒绝保存的原因，设置页就地显示。
    private(set) var rejection: Problem?

    private let core: ClientCore

    init(core: ClientCore) {
        self.core = core
        viewModel = core.appLock()
    }

    /// 在根视图的 `.task` 里调用，App 在前台时一直订阅。
    func observe() async {
        for await viewModel in core.subscribeAppLock().updates() {
            self.viewModel = viewModel
        }
    }

    func set(enabled: Bool, timeout: AppLockTimeout) {
        do {
            try core.setAppLock(enabled: enabled, timeout: timeout)
            rejection = nil
        } catch {
            rejection = error.problem
        }
    }

    func unlock() {
        core.unlockApp()
    }

    func clearRejection() {
        rejection = nil
    }
}

/// 锁定时盖在整棵界面上的遮罩（ADR-0102）。退到后台立刻盖上，切换器的快照里看不到内容；
/// 回到前台时锁定就弹一次系统验证，取消后停在锁页可再点。
struct AppLockGate<Content: View>: View {
    let lock: AppLockModel
    @ViewBuilder let content: Content
    @Environment(\.scenePhase) private var scenePhase

    private var shows: Bool { scenePhase != .active || lock.viewModel.locked }

    var body: some View {
        ZStack {
            content
            if shows {
                AppLockScreen(lock: lock, locked: lock.viewModel.locked)
            }
        }
        .task(id: Attempt(scenePhase: scenePhase, locked: lock.viewModel.locked)) {
            guard scenePhase == .active, lock.viewModel.locked else { return }
            if await SystemAuth.authenticate(reason: String(localized: .appLockVerifyReason)) {
                lock.unlock()
            }
        }
    }

    /// 一起变才重新弹：进后台、解锁都不该再来一次。
    private struct Attempt: Equatable {
        let scenePhase: ScenePhase
        let locked: Bool
    }
}

private struct AppLockScreen: View {
    let lock: AppLockModel
    /// 锁着才画解锁的内容；只是切出去遮一下（`.inactive`）时留白。
    let locked: Bool
    @State private var needsDeviceLock = false
    @Environment(\.wontonColors) private var colors

    var body: some View {
        ZStack {
            Color(colors.canvas).ignoresSafeArea()
            if locked {
                VStack(spacing: WontonTheme.spacing.s16) {
                    Image(systemName: "lock.fill")
                        .font(.system(size: 44))
                        .foregroundStyle(Color(colors.accentInk))
                    Text(.appLockLockedTitle)
                        .wontonText(WontonTheme.typography.screenTitle)
                        .foregroundStyle(Color(colors.textPrimary))
                    if needsDeviceLock {
                        Text(.appLockNoDeviceLock)
                            .wontonText(WontonTheme.typography.supporting)
                            .foregroundStyle(Color(colors.textSecondary))
                            .multilineTextAlignment(.center)
                        Button(action: openPasscodeSettings) {
                            Text(.appLockOpenSettings).frame(maxWidth: .infinity)
                        }
                        .buttonStyle(WontonButtonStyle(kind: .primary))
                        .frame(maxWidth: 240)
                    } else {
                        Button(action: authenticate) {
                            Text(.appLockUnlock).frame(maxWidth: .infinity)
                        }
                        .buttonStyle(WontonButtonStyle(kind: .primary))
                        .frame(maxWidth: 240)
                    }
                }
                .padding(WontonTheme.spacing.pageInset)
            }
        }
        .task { needsDeviceLock = !SystemAuth.canAuthenticate() }
    }

    private func authenticate() {
        Task {
            if await SystemAuth.authenticate(reason: String(localized: .appLockVerifyReason)) {
                lock.unlock()
            }
        }
    }
}
