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
                if encryption.canReset {
                    Button {
                        isConfirmingReset = true
                    } label: {
                        Text(.chatBackupReset).frame(maxWidth: .infinity)
                    }
                    .buttonStyle(WontonButtonStyle(kind: .secondary))
                }
            }
            .frame(maxWidth: .infinity, alignment: .leading)
            .padding(WontonTheme.spacing.pageInset)
        }
        .scrollDismissesKeyboard(.interactively)
        .wontonPushedScreen(String(localized: .chatBackupTitle))
        .wontonDialog(
            String(localized: .chatBackupResetTitle),
            message: String(localized: .chatBackupResetMessage),
            isPresented: $isConfirmingReset,
            actions: [
                WontonDialogAction(title: String(localized: .commonCancel), kind: .secondary) {},
                WontonDialogAction(title: String(localized: .chatBackupResetConfirm), kind: .danger) {
                    model.send(.resetRecovery)
                }
            ]
        )
    }

    @ViewBuilder private var nextStep: some View {
        switch encryption.recovery {
        case .incomplete:
            if showKeyEntry {
                recoveryKeyEntry
            } else if encryption.verificationFlow != nil {
                EmptyView()
            } else if encryption.canVerifyWithOtherDevice {
                // 账号里有别的已验证设备：确认和恢复密钥两条路并列（ADR-0046）。
                HStack(spacing: WontonTheme.spacing.s8) {
                    Button { model.send(.startDeviceVerification) } label: {
                        Text(.verificationVerifyWithOtherDevice).frame(maxWidth: .infinity)
                    }
                    .buttonStyle(WontonButtonStyle(kind: .primary))
                    Button { showKeyEntry = true } label: {
                        Text(.verificationUseRecoveryKey).frame(maxWidth: .infinity)
                    }
                    .buttonStyle(WontonButtonStyle(kind: .secondary))
                }
            } else {
                recoveryKeyEntry
            }
        case .inProgress:
            ProgressView().frame(maxWidth: .infinity)
        case .disabled, .enabled, .unknown:
            EmptyView()
        }
    }

    private var recoveryKeyEntry: some View {
        VStack(spacing: WontonTheme.spacing.s12) {
            SecureField(String(localized: .chatBackupRecoveryKey), text: $key)
                .textFieldStyle(.plain)
                .textInputAutocapitalization(.never)
                .autocorrectionDisabled()
                .wontonField()
            Button {
                model.send(.recoverEncryption(recoveryKey: key))
                key = ""
            } label: {
                Text(.chatBackupRecover).frame(maxWidth: .infinity)
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
        case .enabled: return String(localized: .chatBackupStatusEnabled)
        case .disabled: return String(localized: .chatBackupStatusDisabled)
        case .incomplete: return String(localized: .chatBackupStatusIncomplete)
        case .inProgress: return String(localized: .chatBackupStatusInProgress)
        case .unknown: return String(localized: .chatBackupStatusUnknown)
        }
    }

    var backupValue: String {
        switch self {
        case .enabled: return String(localized: .chatBackupValueEnabled)
        case .disabled: return String(localized: .chatBackupValueDisabled)
        case .incomplete: return String(localized: .chatBackupValueIncomplete)
        case .inProgress: return String(localized: .chatBackupValueInProgress)
        case .unknown: return String(localized: .chatBackupValueUnknown)
        }
    }
}

extension EncryptionStatus {
    /// 恢复密钥丢了、备份残缺或者开启不了时的出路。正在显示新密钥时不出现，免得主人还没抄下来就把它作废。
    var canReset: Bool {
        guard recoveryKey == nil else { return false }
        switch recovery {
        case .enabled, .incomplete: return true
        case .disabled: return verification == .unverified
        case .inProgress, .unknown: return false
        }
    }

    var backupDetail: String? {
        switch recovery {
        case .enabled: return String(localized: .chatBackupDetailEnabled)
        case .disabled:
            // 未验证的设备开启不了，核心会拒绝。
            return verification == .unverified
                ? String(localized: .chatBackupDetailUnverified)
                : String(localized: .chatBackupDetailVerified)
        case .incomplete: return String(localized: .chatBackupDetailIncomplete)
        case .inProgress: return nil
        case .unknown: return String(localized: .chatBackupDetailUnknown)
        }
    }
}

extension BackupReminder {
    var text: String? {
        switch self {
        case .saveRecoveryKey: return String(localized: .chatBackupReminderSaveKey)
        case .enterRecoveryKey: return String(localized: .chatBackupReminderEnterKey)
        case .other: return nil
        }
    }
}

private struct RecoveryKeySection: View {
    let recoveryKey: String
    let confirm: () -> Void
    @Environment(\.wontonColors) private var colors

    var body: some View {
        VStack(alignment: .leading, spacing: WontonTheme.spacing.s8) {
            Text(.chatBackupSaveKeyTitle)
                .wontonText(WontonTheme.typography.itemTitle)
                .foregroundStyle(Color(colors.textPrimary))
            // 不能长按选中：系统的拷贝不带下面的保护，复制只走“复制”按钮。
            Text(recoveryKey)
                .font(.body.monospaced())
                .foregroundStyle(Color(colors.textPrimary))
                .padding(WontonTheme.spacing.s12)
                .frame(maxWidth: .infinity, alignment: .leading)
                .background(RoundedRectangle(cornerRadius: WontonTheme.radii.control).fill(Color(colors.surfaceSubtle)))
            Text(.chatBackupSaveKeyNote)
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
                    Text(.chatBackupCopy).frame(maxWidth: .infinity)
                }
                .buttonStyle(WontonButtonStyle(kind: .secondary))
                Button(action: confirm) {
                    Text(.chatBackupSaved).frame(maxWidth: .infinity)
                }
                .buttonStyle(WontonButtonStyle(kind: .primary))
            }
        }
    }
}
