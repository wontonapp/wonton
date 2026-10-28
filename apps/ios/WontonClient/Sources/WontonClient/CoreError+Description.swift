/// UniFFI 0.32 给错误类型生成了取自 Rust `Display` 的 `description`，但没有声明
/// `CustomStringConvertible`。补上声明后，`String(describing:)` 显示核心给出的原因。
extension CoreError: CustomStringConvertible {}
