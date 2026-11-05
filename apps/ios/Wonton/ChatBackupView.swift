import SwiftUI
import UniformTypeIdentifiers
import WontonClient

/// 聊天记录备份。协议里叫恢复（密钥备份加秘密存储），这里按它对主人的用处说：换设备后还能看到以前的消息。
struct ChatBackupView: View {
    let model: ChatModel
    @State private var key = ""
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
                nextStep
                if encryption.canReset {
                    Button {
                        isConfirmingReset = true
                    } label: {
                        Text("重置备份").frame(maxWidth: .infinity)
                    }
                    .buttonStyle(WontonButtonStyle(kind: .secondary))
                }
            }
            .frame(maxWidth: .infinity, alignment: .leading)
            .padding(WontonTheme.spacing.pageInset)
        }
        .scrollDismissesKeyboard(.interactively)
        .wontonPushedScreen("聊天记录备份")
        .wontonDialog(
            "重置聊天记录备份？",
            message: "会换一把新的恢复密钥，旧的作废。重置后旧消息可能读不了，其他设备要输入新密钥。",
            isPresented: $isConfirmingReset,
            actions: [
                WontonDialogAction(title: "取消", kind: .secondary) {},
                WontonDialogAction(title: "重置", kind: .danger) { model.send(.resetRecovery) }
            ]
        )
    }

    @ViewBuilder private var nextStep: some View {
        switch encryption.recovery {
        case .incomplete:
            VStack(spacing: WontonTheme.spacing.s12) {
                SecureField("恢复密钥", text: $key)
                    .textFieldStyle(.plain)
                    .textInputAutocapitalization(.never)
                    .autocorrectionDisabled()
                    .wontonField()
                Button {
                    model.send(.recoverEncryption(recoveryKey: key))
                    key = ""
                } label: {
                    Text("取回以前的消息").frame(maxWidth: .infinity)
                }
                .buttonStyle(WontonButtonStyle(kind: .primary))
                .disabled(key.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty)
            }
        case .inProgress:
            ProgressView().frame(maxWidth: .infinity)
        case .disabled, .enabled, .unknown:
            EmptyView()
        }
    }

    private var encryption: EncryptionStatus { model.viewModel.encryption }
}

extension RecoveryStatus {
    var backupTitle: String {
        switch self {
        case .enabled: return "聊天记录已备份"
        case .disabled: return "还没有备份聊天记录"
        case .incomplete: return "这台设备还看不到以前的消息"
        case .inProgress: return "正在处理…"
        case .unknown: return "还不知道备份的情况"
        }
    }

    var backupValue: String {
        switch self {
        case .enabled: return "已开启"
        case .disabled: return "未开启"
        case .incomplete: return "需要恢复密钥"
        case .inProgress: return "处理中"
        case .unknown: return "不确定"
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
        case .enabled: return "换手机或新设备时，输入恢复密钥就能看到以前的消息。"
        case .disabled:
            // 未验证的设备开启不了，核心会拒绝。
            return verification == .unverified
                ? "请打开最早配对的那台设备，备份会自动开启。那台设备不在手边，就重置备份。"
                : "设备验证后会自动开启备份。"
        case .incomplete: return "输入开启备份时保存的恢复密钥。密钥丢了，就重置备份。"
        case .inProgress: return nil
        case .unknown: return "连上家里的服务器后才能确认。"
        }
    }
}

extension BackupReminder {
    var text: String? {
        switch self {
        case .saveRecoveryKey: return "保存恢复密钥，换手机时要用"
        case .enterRecoveryKey: return "输入恢复密钥，取回以前的消息"
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
            Text("保存恢复密钥")
                .wontonText(WontonTheme.typography.itemTitle)
                .foregroundStyle(Color(colors.textPrimary))
            // 不能长按选中：系统的拷贝不带下面的保护，复制只走“复制”按钮。
            Text(recoveryKey)
                .font(.body.monospaced())
                .foregroundStyle(Color(colors.textPrimary))
                .padding(WontonTheme.spacing.s12)
                .frame(maxWidth: .infinity, alignment: .leading)
                .background(RoundedRectangle(cornerRadius: WontonTheme.radii.control).fill(Color(colors.surfaceSubtle)))
            Text("换手机时要用它。只显示这一次，点“已保存”后本机不再留着它。")
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
                    Text("复制").frame(maxWidth: .infinity)
                }
                .buttonStyle(WontonButtonStyle(kind: .secondary))
                Button(action: confirm) {
                    Text("已保存").frame(maxWidth: .infinity)
                }
                .buttonStyle(WontonButtonStyle(kind: .primary))
            }
        }
    }
}
