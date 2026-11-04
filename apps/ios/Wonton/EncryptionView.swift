import SwiftUI
import UniformTypeIdentifiers
import WontonClient

struct EncryptionView: View {
    let model: ChatModel
    @State private var key = ""

    var body: some View {
        ScrollView {
            VStack(spacing: WontonTheme.spacing.sectionGap) {
                if let recoveryKey = encryption.recoveryKey {
                    RecoveryKeySection(recoveryKey: recoveryKey) { model.send(.confirmRecoveryKeySaved) }
                }
                WontonGroup {
                    WontonRow(title: "本设备", value: verificationText)
                    WontonSeparator()
                    WontonRow(title: "恢复", value: recoveryText)
                }
                recovery
            }
            .padding(WontonTheme.spacing.pageInset)
        }
        .wontonPushedScreen("加密")
    }

    @ViewBuilder private var recovery: some View {
        switch encryption.recovery {
        case .disabled:
            Button {
                model.send(.setUpRecovery)
            } label: {
                Text("设置恢复").frame(maxWidth: .infinity)
            }
            .buttonStyle(WontonButtonStyle(kind: .primary))
        case .incomplete:
            VStack(spacing: WontonTheme.spacing.s12) {
                SecureField("输入恢复密钥", text: $key)
                    .textFieldStyle(.plain)
                    .textInputAutocapitalization(.never)
                    .autocorrectionDisabled()
                    .wontonField()
                Button {
                    model.send(.recoverEncryption(recoveryKey: key))
                    key = ""
                } label: {
                    Text("恢复").frame(maxWidth: .infinity)
                }
                .buttonStyle(WontonButtonStyle(kind: .primary))
                .disabled(key.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty)
            }
        case .inProgress:
            ProgressView()
        case .enabled, .unknown:
            EmptyView()
        }
    }

    private var encryption: EncryptionStatus { model.viewModel.encryption }

    private var verificationText: String {
        switch encryption.verification {
        case .verified: return "已验证"
        case .unverified: return "未验证"
        case .unknown: return "未知"
        }
    }

    private var recoveryText: String {
        switch encryption.recovery {
        case .enabled: return "已开启"
        case .disabled: return "未设置"
        case .incomplete: return "缺少密钥"
        case .inProgress: return "进行中"
        case .unknown: return "未知"
        }
    }
}

private struct RecoveryKeySection: View {
    let recoveryKey: String
    let confirm: () -> Void
    @Environment(\.wontonColors) private var colors

    var body: some View {
        VStack(alignment: .leading, spacing: WontonTheme.spacing.s8) {
            Text("恢复密钥")
                .wontonText(WontonTheme.typography.itemTitle)
                .foregroundStyle(Color(colors.textPrimary))
            // 不能长按选中：系统的拷贝不带下面的保护，复制只走“复制”按钮。
            Text(recoveryKey)
                .font(.body.monospaced())
                .foregroundStyle(Color(colors.textPrimary))
                .padding(WontonTheme.spacing.s12)
                .frame(maxWidth: .infinity, alignment: .leading)
                .background(RoundedRectangle(cornerRadius: WontonTheme.radii.control).fill(Color(colors.surfaceSubtle)))
            Text("只显示这一次。点“已保存”后本机不再留着它。")
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
