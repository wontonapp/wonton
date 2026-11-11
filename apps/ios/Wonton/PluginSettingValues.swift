// 草稿到提交值的换算：按控件类型把页面上的草稿变成请求里的值，顺手就地记下输入错误。

import Foundation
import WontonClient

/// 一个字段换算的结果：给值、清空回默认（请求里要显式的 null）、跳过、或者输入不合法（错误已记到 `localErrors`）。
enum Submitted {
    case set(SettingValue)
    case clear
    case skip
    case invalid
}

/// 按控件类型换算一个字段的提交值；只在动过的字段上调用。
func submitted(
    field: PluginSettingField,
    draft: Draft,
    errors: inout [String: String]
) -> Submitted {
    switch (field.kind, draft) {
    case let (.toggle, .toggle(isOn)):
        return .set(.flag(value: isOn))
    case let (.integer, .text(input)):
        if input.isEmpty {
            return .clear
        }
        if let integer = Int64(input) {
            return .set(.integer(value: integer))
        }
        errors[field.name] = String(localized: .pluginSettingsInvalidInteger)
        return .invalid
    case let (.decimal, .text(input)):
        if input.isEmpty {
            return .clear
        }
        if Decimal(string: input) != nil {
            return .set(.decimal(value: input))
        }
        errors[field.name] = String(localized: .pluginSettingsInvalidDecimal)
        return .invalid
    case let (.text, .text(input)):
        if input.isEmpty { return .clear }
        return .set(.text(value: input))
    case let (.choice(options, false), .choice(index)):
        return .set(options[index].value)
    case let (.choice(options, true), .multi(indices)):
        let texts = indices.compactMap { settingText(options[$0].value) }
        if indices.isEmpty { return .clear }
        return .set(.texts(value: texts))
    default:
        return .skip
    }
}

private func settingText(_ value: SettingValue) -> String? {
    if case let .text(value: text) = value { return text }
    return nil
}
