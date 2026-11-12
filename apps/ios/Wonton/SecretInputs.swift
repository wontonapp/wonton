import SwiftUI
import WontonClient

/// 密钥一节：每项一个输入框，标题、说明和“去哪里拿”都由核心给好。值只上行，不进日志。
struct SecretInputsSection: View {
    let inputs: [SecretInput]
    let values: [String: String]
    var enabled = true
    var onShownChange: (Bool) -> Void = { _ in }
    let onChange: (String, String) -> Void

    var body: some View {
        if !inputs.isEmpty {
            VStack(alignment: .leading, spacing: WontonTheme.spacing.s12) {
                ForEach(inputs, id: \.name) { input in
                    SecretInputField(
                        input, values[input.name] ?? "", enabled: enabled,
                        onShownChange: onShownChange
                    ) { onChange(input.name, $0) }
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
    @Environment(\.openURL) private var openURL
    @Environment(\.wontonColors) private var colors

    init(
        _ input: SecretInput, _ value: String, enabled: Bool = true,
        onShownChange: @escaping (Bool) -> Void = { _ in },
        onChange: @escaping (String) -> Void
    ) {
        self.input = input
        _value = Binding(get: { value }, set: onChange)
        self.enabled = enabled
        self.onShownChange = onShownChange
    }

    var body: some View {
        VStack(alignment: .leading, spacing: WontonTheme.spacing.s4) {
            HStack(spacing: WontonTheme.spacing.s8) {
                Text(verbatim: input.title)
                    .wontonText(WontonTheme.typography.body)
                    .foregroundStyle(Color(colors.textPrimary))
                if !input.required {
                    Tag(text: String(localized: .secretOptional), tint: colors.textSecondary)
                }
            }
            if let description = input.description {
                Text(verbatim: description)
                    .wontonText(WontonTheme.typography.supporting)
                    .foregroundStyle(Color(colors.textSecondary))
                    .fixedSize(horizontal: false, vertical: true)
            }
            HStack(spacing: WontonTheme.spacing.s8) {
                Group {
                    if shown {
                        TextField(input.title, text: $value)
                            .textInputAutocapitalization(.never)
                            .autocorrectionDisabled()
                    } else {
                        SecureField(input.title, text: $value)
                    }
                }
                .disabled(!enabled)
                .wontonText(WontonTheme.typography.body)
                .foregroundStyle(Color(colors.textPrimary))
                .padding(.horizontal, WontonTheme.spacing.s12)
                .frame(minHeight: WontonTheme.size.target)
                .background(Color(colors.surfaceSubtle), in: RoundedRectangle(cornerRadius: WontonTheme.radii.control))
                .accessibilityLabel(Text(verbatim: input.title))
                Button {
                    shown.toggle()
                    onShownChange(shown)
                } label: {
                    (shown ? WontonIcon.lock : WontonIcon.eye)
                        .view(size: WontonTheme.size.iconSmall)
                        .foregroundStyle(Color(colors.textSecondary))
                        .frame(width: WontonTheme.size.target, height: WontonTheme.size.target)
                        .contentShape(Rectangle())
                }
                .disabled(!enabled)
                .accessibilityLabel(Text(shown ? .secretHide : .secretShow))
            }
            if let link = input.url.flatMap(URL.init(string:)) {
                Button(String(localized: .secretGetOne)) { openURL(link) }
                    .wontonText(WontonTheme.typography.supporting)
                    .foregroundStyle(Color(colors.accentInk))
            }
        }
    }
}

/// 请求里带的密钥：只带填了的，可选项没填就不发。
func secretValues(_ inputs: [SecretInput], _ values: [String: String]) -> [String: String] {
    inputs.reduce(into: [:]) { sent, input in
        let value = values[input.name] ?? ""
        if !value.isEmpty { sent[input.name] = value }
    }
}

/// 还有必填项没填：确认按钮不可点。
func missingRequiredSecret(_ inputs: [SecretInput], _ values: [String: String]) -> Bool {
    inputs.contains { input in input.required && (values[input.name] ?? "").isEmpty }
}
