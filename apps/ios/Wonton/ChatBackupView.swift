import LocalAuthentication
import SwiftUI
import UniformTypeIdentifiers
import WontonClient

/// 聊天记录备份。协议里叫恢复（密钥备份加秘密存储），这里按它对主人的用处说：换设备后还能看到以前的消息。
struct ChatBackupView: View {
    let model: ChatModel
    @State private var key = ""
    @State private var showKeyEntry = false
    @State private var isConfirmingReset = false
    @Environment(\.wontonColors) private var colors

    var body: some View {
        ScrollView {
            VStack(alignment: .leading, spacing: WontonTheme.spacing.sectionGap) {
                if let recoveryKey = encryption.recoveryKey {
                    RecoveryKeySection(recoveryKey: recoveryKey) { model.send(.confirmRecoveryKeySaved) }
                }
                VStack(alignment: .leading, spacing: WontonTheme.spacing.s8) {
                    Text(encryption.recovery.backupTitle)
                        .wontonText(WontonTheme.typography.itemTitle)
                        .foregroundStyle(Color(colors.textPrimary))
                    if let detail = encryption.backupDetail {
                        Text(detail)
                            .wontonText(WontonTheme.typography.supporting)
                            .foregroundStyle(Color(colors.textSecondary))
                    }
                }
                // 确认流程在这一屏显示，不管恢复是什么状态：这一页开着时弹窗压着不弹（DeviceVerificationDialogs），
                // 页面就是唯一看得到请求的地方。流程结束时留着结果，收起弹窗后还在这里看得到。
                if let flow = encryption.verificationFlow {
                    DeviceVerificationFlowView(flow: flow, model: model, useRecoveryKey: { showKeyEntry = true })
                }
                nextStep
                // 新设备进来第一件事是输恢复密钥；重置只是丢了钥匙时的退路，做成不抢眼的文字按钮放在最下面。
                if encryption.canReset {
                    Button {
                        isConfirmingReset = true
                    } label: {
                        Text(.ChatBackup1.chatBackupReset)
                            .wontonText(WontonTheme.typography.supporting)
                            .foregroundStyle(Color(colors.textSecondary))
                            .frame(maxWidth: .infinity, minHeight: WontonTheme.size.target)
                            .contentShape(Rectangle())
                    }
                    .buttonStyle(.plain)
                }
            }
            .frame(maxWidth: .infinity, alignment: .leading)
            .padding(WontonTheme.spacing.pageInset)
        }
        .scrollDismissesKeyboard(.interactively)
        .wontonPushedScreen(String(localized: .ChatBackup1.chatBackupTitle))
        .wontonDialog(
            String(localized: .ChatBackup1.chatBackupResetTitle),
            message: String(localized: .ChatBackup0.chatBackupResetMessage),
            isPresented: $isConfirmingReset,
            actions: [
                WontonDialogAction(title: String(localized: .Common.commonCancel), kind: .secondary) {},
                WontonDialogAction(title: String(localized: .ChatBackup1.chatBackupResetConfirm), kind: .danger) {
                    model.send(.resetRecovery)
                }
            ]
        )
    }

    @ViewBuilder private var nextStep: some View {
        // 类别由核心按恢复状态算好（ADR-0076），这里只按类别画。
        switch encryption.recoveryNextStep {
        case .enterKey:
            recoveryKeyEntry
        case .chooseDeviceOrKey:
            // 账号里有别的已验证设备：确认和恢复密钥两条路并列（ADR-0046）。
            if showKeyEntry {
                recoveryKeyEntry
            } else {
                HStack(spacing: WontonTheme.spacing.s8) {
                    Button { model.send(.startDeviceVerification) } label: {
                        Text(.Verification1.verificationVerifyWithOtherDevice).frame(maxWidth: .infinity)
                    }
                    .buttonStyle(WontonButtonStyle(kind: .primary))
                    Button { showKeyEntry = true } label: {
                        Text(.Verification1.verificationUseRecoveryKey).frame(maxWidth: .infinity)
                    }
                    .buttonStyle(WontonButtonStyle(kind: .secondary))
                }
            }
        case .showFlow:
            // 设备确认是这一步：流程在上面那一屏；还没开始时给入口按钮（账号没有备份，输恢复密钥没用）。
            if encryption.verificationFlow == nil {
                Button { model.send(.startDeviceVerification) } label: {
                    Text(.Verification1.verificationVerifyWithOtherDevice).frame(maxWidth: .infinity)
                }
                .buttonStyle(WontonButtonStyle(kind: .primary))
            } else if showKeyEntry {
                recoveryKeyEntry
            }
        case .progress:
            ProgressView().frame(maxWidth: .infinity)
        case .waitForOtherDevice:
            // 新设备在等另一台设备交出密钥（ADR-0087）：这里不给恢复密钥，回到交接页。
            Button { model.keyHandoffDeferred = false } label: {
                Text(.Handoff.handoffTitle).frame(maxWidth: .infinity)
            }
            .buttonStyle(WontonButtonStyle(kind: .primary))
        case .none, .other:
            EmptyView()
        }
    }

    private var recoveryKeyEntry: some View {
        VStack(spacing: WontonTheme.spacing.s12) {
            SecureField(String(localized: .ChatBackup0.chatBackupRecoveryKey), text: $key)
                .textFieldStyle(.plain)
                .textInputAutocapitalization(.never)
                .autocorrectionDisabled()
                .wontonField()
            Button {
                model.send(.recoverEncryption(recoveryKey: key))
                key = ""
            } label: {
                Text(.ChatBackup1.chatBackupRecover).frame(maxWidth: .infinity)
            }
            .buttonStyle(WontonButtonStyle(kind: .primary))
            .disabled(key.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty)
        }
    }

    private var encryption: EncryptionStatus { model.viewModel.encryption }
}

extension RecoveryStatus {
    var backupTitle: String {
        switch self {
        case .enabled: return String(localized: .ChatBackup0.chatBackupStatusEnabled)
        case .disabled: return String(localized: .ChatBackup1.chatBackupStatusDisabled)
        case .incomplete: return String(localized: .ChatBackup0.chatBackupStatusIncomplete)
        case .inProgress: return String(localized: .ChatBackup1.chatBackupStatusInProgress)
        case .unknown: return String(localized: .ChatBackup1.chatBackupStatusUnknown)
        }
    }

    var backupValue: String {
        switch self {
        case .enabled: return String(localized: .ChatBackup1.chatBackupValueEnabled)
        case .disabled: return String(localized: .ChatBackup1.chatBackupValueDisabled)
        case .incomplete: return String(localized: .ChatBackup1.chatBackupValueIncomplete)
        case .inProgress: return String(localized: .ChatBackup0.chatBackupValueInProgress)
        case .unknown: return String(localized: .ChatBackup0.chatBackupValueUnknown)
        }
    }
}

extension EncryptionStatus {
    var backupDetail: String? {
        switch recovery {
        case .enabled: return String(localized: .ChatBackup0.chatBackupDetailEnabled)
        case .disabled:
            // 未验证的设备开启不了，核心会拒绝。
            return verification == .unverified
                ? String(localized: .ChatBackup0.chatBackupDetailUnverified)
                : String(localized: .ChatBackup0.chatBackupDetailVerified)
        case .incomplete: return String(localized: .ChatBackup0.chatBackupDetailIncomplete)
        case .inProgress: return nil
        case .unknown: return String(localized: .ChatBackup1.chatBackupDetailUnknown)
        }
    }
}

extension KeyNotice {
    /// 按钮上的字；`other` 是更新的核心给的新类别，只显示说明。
    var action: String? {
        switch self {
        case .enterRecoveryKey: return String(localized: .Verification1.verificationUseRecoveryKey)
        case .verifyWithOtherDevice: return String(localized: .Verification1.verificationVerifyWithOtherDevice)
        case .other: return nil
        }
    }
}

extension BackupReminder {
    var text: String? {
        switch self {
        case .saveRecoveryKey: return String(localized: .ChatBackup0.chatBackupReminderSaveKey)
        case .enterRecoveryKey: return String(localized: .ChatBackup1.chatBackupReminderEnterKey)
        case .continueOnOtherDevice: return String(localized: .Handoff.handoffTitle)
        case .other: return nil
        }
    }
}

private struct RecoveryKeySection: View {
    let recoveryKey: String
    let confirm: () -> Void
    // 验证只对这一次显示有效：收起页面再进会重建这个视图，要重新验证。
    @State private var revealed: Bool
    @Environment(\.wontonColors) private var colors

    /// 没设锁屏密码的设备不拦（canEvaluatePolicy 报 passcodeNotSet），密钥直接显示。
    init(recoveryKey: String, confirm: @escaping () -> Void) {
        self.recoveryKey = recoveryKey
        self.confirm = confirm
        let context = LAContext()
        var error: NSError?
        _revealed = State(
            initialValue: !context.canEvaluatePolicy(.deviceOwnerAuthentication, error: &error)
                && (error as? LAError)?.code == .passcodeNotSet
        )
    }

    var body: some View {
        VStack(alignment: .leading, spacing: WontonTheme.spacing.s8) {
            Text(.ChatBackup0.chatBackupSaveKeyTitle)
                .wontonText(WontonTheme.typography.itemTitle)
                .foregroundStyle(Color(colors.textPrimary))
            if revealed {
                revealedKey
            } else {
                Button {
                    authenticate()
                } label: {
                    Text(.ChatBackup1.chatBackupRevealKey).frame(maxWidth: .infinity)
                }
                .buttonStyle(WontonButtonStyle(kind: .primary))
            }
        }
    }

    private var revealedKey: some View {
        VStack(alignment: .leading, spacing: WontonTheme.spacing.s8) {
            // 不能长按选中：系统的拷贝不带下面的保护，复制只走“复制”按钮。
            Text(recoveryKey)
                .font(.body.monospaced())
                .foregroundStyle(Color(colors.textPrimary))
                .padding(WontonTheme.spacing.s12)
                .frame(maxWidth: .infinity, alignment: .leading)
                .background(RoundedRectangle(cornerRadius: WontonTheme.radii.control).fill(Color(colors.surfaceSubtle)))
            Text(.ChatBackup0.chatBackupSaveKeyNote)
                .wontonText(WontonTheme.typography.supporting)
                .foregroundStyle(Color(colors.textSecondary))
            HStack(spacing: WontonTheme.spacing.s8) {
                Button {
                    // 只留在本机的剪贴板（不经“通用剪贴板”同步到别的设备），两分钟后清掉。
                    UIPasteboard.general.setItems(
                        [[UTType.plainText.identifier: recoveryKey]],
                        options: [.localOnly: true, .expirationDate: Date.now.addingTimeInterval(120)]
                    )
                } label: {
                    Text(.ChatBackup1.chatBackupCopy).frame(maxWidth: .infinity)
                }
                .buttonStyle(WontonButtonStyle(kind: .secondary))
                Button(action: confirm) {
                    Text(.ChatBackup1.chatBackupSaved).frame(maxWidth: .infinity)
                }
                .buttonStyle(WontonButtonStyle(kind: .primary))
            }
        }
    }

    @MainActor
    private func authenticate() {
        let context = LAContext()
        var error: NSError?
        guard context.canEvaluatePolicy(.deviceOwnerAuthentication, error: &error) else {
            // 生物识别坏了这类失败：留在本页，可再点一次；系统提示里会说明原因，这里不另弹提示。
            return
        }
        Task {
            // 成功才显示；取消和失败都不显示，也不另弹提示。
            if (try? await context.evaluatePolicy(
                .deviceOwnerAuthentication,
                localizedReason: String(localized: .ChatBackup1.chatBackupVerifyReason)
            )) == true {
                revealed = true
            }
        }
    }
}
