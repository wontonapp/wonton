import SwiftUI
import WontonClient

/// 要填的密钥（ADR-0064）：每项一个密码输入框，标题、说明和“去哪里拿”都由核心给好。
struct SecretInputsSection: View {
    let inputs: [SecretInput]
    @Binding var values: [String: String]
    var enabled = true
    var onShownChange: (Bool) -> Void = { _ in }
    @State private var shown: Set<String> = []

    var body: some View {
        VStack(alignment: .leading, spacing: WontonTheme.spacing.s12) {
            ForEach(inputs, id: \.name) { input in
                SecretInputField(
                    input: input,
                    value: Binding(
                        get: { values[input.name] ?? "" },
                        set: { values[input.name] = $0 }
                    ),
                    enabled: enabled
                ) { isShown in
                    shown = isShown ? shown.union([input.name]) : shown.subtracting([input.name])
                    onShownChange(!shown.isEmpty)
                }
            }
        }
    }
}

/// 一项密钥的输入框：值默认打码，右边是显示按钮；标题下面写说明和链接。
struct SecretInputField: View {
    let input: SecretInput
    @Binding var value: String
    var enabled = true
    var onShownChange: (Bool) -> Void = { _ in }
    @State private var shown = false
    @Environment(\.wontonColors) private var colors

    var body: some View {
        VStack(alignment: .leading, spacing: WontonTheme.spacing.s8) {
            HStack(spacing: WontonTheme.spacing.s8) {
                Text(input.title)
                    .wontonText(WontonTheme.typography.body)
                    .foregroundStyle(Color(colors.textPrimary))
                    .frame(maxWidth: .infinity, alignment: .leading)
                if !input.required {
                    Text(.pluginPermissionOptional)
                        .wontonText(WontonTheme.typography.caption)
                        .foregroundStyle(Color(colors.textSecondary))
                }
            }
            if let description = input.description {
                Text(description)
                    .wontonText(WontonTheme.typography.caption)
                    .foregroundStyle(Color(colors.textSecondary))
                    .lineLimit(2)
            }
            HStack(spacing: WontonTheme.spacing.s8) {
                field
                WontonIconButton(
                    icon: shown ? .lockKey : .eye,
                    label: String(localized: shown ? .pluginSecretHide : .pluginSecretShow),
                    tint: Color(colors.textSecondary)
                ) {
                    shown.toggle()
                    onShownChange(shown)
                }
                .disabled(!enabled)
            }
            if let url = input.url.flatMap(URL.init(string:)) {
                Link(destination: url) {
                    Text(.pluginSecretGet)
                        .wontonText(WontonTheme.typography.caption)
                        .foregroundStyle(Color(colors.accentInk))
                }
                .accessibilityLabel(url.absoluteString)
            }
        }
        .accessibilityElement(children: .contain)
    }

    @ViewBuilder private var field: some View {
        if shown {
            TextField(input.title, text: $value)
                .textFieldStyle(.plain)
                .wontonField()
                .modifiers(input: input, enabled: enabled)
        } else {
            SecureField(input.title, text: $value)
                .textFieldStyle(.plain)
                .wontonField()
                .modifiers(input: input, enabled: enabled)
        }
    }
}

private extension View {
    /// 关掉自动更正、拼写和自动填充建议：值不是自然语言，系统学去了也没用。
    func modifiers(input: SecretInput, enabled: Bool) -> some View {
        textContentType(.none)
            .textInputAutocapitalization(.never)
            .autocorrectionDisabled()
            .keyboardType(.asciiCapable)
            .submitLabel(.done)
            .accessibilityLabel(input.title)
            .disabled(!enabled)
    }
}

/// 请求里带的密钥：只带填了的，可选项没填就不发。
func secretValues(_ inputs: [SecretInput], _ values: [String: String]) -> [String: String] {
    var result: [String: String] = [:]
    for input in inputs {
        guard let value = values[input.name]?.trimmingCharacters(in: .whitespacesAndNewlines),
              !value.isEmpty else { continue }
        result[input.name] = value
    }
    return result
}

/// 还有必填项没填：确认按钮不可点。
func missingRequiredSecret(_ inputs: [SecretInput], _ values: [String: String]) -> Bool {
    inputs.contains { input in
        guard input.required else { return false }
        return values[input.name]?.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty ?? true
    }
}

/// App 不在前台时盖住这一屏，切换器里的快照看不到填过的值。
private struct SecretSnapshotMask: ViewModifier {
    @Environment(\.scenePhase) private var scenePhase
    @Environment(\.wontonColors) private var colors

    func body(content: Content) -> some View {
        content.overlay {
            if scenePhase != .active {
                Color(colors.canvas).ignoresSafeArea()
            }
        }
    }
}

extension View {
    func secretSnapshotMask() -> some View {
        modifier(SecretSnapshotMask())
    }
}
