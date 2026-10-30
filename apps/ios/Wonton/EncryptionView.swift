import SwiftUI
import UniformTypeIdentifiers
import WontonClient

struct EncryptionView: View {
    let model: ChatModel
    @Environment(\.dismiss) private var dismiss
    @State private var key = ""

    var body: some View {
        NavigationStack {
            Form {
                if let recoveryKey = encryption.recoveryKey {
                    RecoveryKeySection(recoveryKey: recoveryKey) { model.send(.confirmRecoveryKeySaved) }
                }
                Section {
                    LabeledContent("本设备", value: verificationText)
                    LabeledContent("恢复", value: recoveryText)
                    switch encryption.recovery {
                    case .disabled:
                        Button("设置恢复") { model.send(.setUpRecovery) }
                    case .incomplete:
                        SecureField("输入恢复密钥", text: $key)
                            .font(.body.monospaced())
                            .textInputAutocapitalization(.never)
                            .autocorrectionDisabled()
                        Button("恢复") {
                            model.send(.recoverEncryption(recoveryKey: key))
                            key = ""
                        }
                        .disabled(key.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty)
                    case .inProgress:
                        ProgressView()
                    case .enabled, .unknown:
                        EmptyView()
                    }
                }
            }
            .navigationTitle("加密")
            .navigationBarTitleDisplayMode(.inline)
            .toolbar { Button("完成") { dismiss() } }
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

    var body: some View {
        Section {
            // 不能长按选中：系统的拷贝不带下面的保护，复制只走“复制”按钮。
            Text(recoveryKey).font(.body.monospaced())
            Button("复制") {
                // 只留在本机的剪贴板（不经“通用剪贴板”同步到别的设备），两分钟后清掉。
                UIPasteboard.general.setItems(
                    [[UTType.plainText.identifier: recoveryKey]],
                    options: [.localOnly: true, .expirationDate: Date.now.addingTimeInterval(120)]
                )
            }
            Button("已保存") { confirm() }
        } header: {
            Text("恢复密钥")
        } footer: {
            Text("只显示这一次。点“已保存”后本机不再留着它。")
        }
    }
}
