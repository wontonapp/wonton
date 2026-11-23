import SwiftUI
import WontonClient

/// “/”命令列表（ADR-0054、ADR-0109 第 8 节）：浮在输入框上方的一张玻璃卡片，一块一条命令（Bot 头像、名字和参数提示、
/// 说明），点一条就发出去。敲“/”弹出和点“菜单”按钮打开的是同一张；列哪些由输入栏定，这里只画。
struct CommandMenu: View {
    let commands: [RoomCommand]
    /// 各 Bot 的表里超过上限被截掉的条数：列表末尾说一声。按前缀过滤时不给。
    let omitted: Int
    /// 房间里有几个 Bot 时，每条命令旁写是谁的。
    let showsBot: Bool
    /// 这条命令是哪个 Bot 的，画它的头像；头像怎么取由聊天页给。
    let avatar: (RoomCommand) -> AnyView
    let choose: (RoomCommand) -> Void
    @Environment(\.wontonColors) private var colors

    var body: some View {
        // 条数少时按内容高，多了封顶在 `size.commandMenu`、在里面滚。
        ViewThatFits(in: .vertical) {
            rows
            ScrollView { rows }
        }
        .frame(maxHeight: WontonTheme.size.commandMenu)
        // 不加这一句，上面的高度上限会被当成固定高度，三条命令也占满一整块。
        .fixedSize(horizontal: false, vertical: true)
        .padding(.vertical, WontonTheme.spacing.s4)
        .clipShape(shape)
        .chatGlass(in: shape)
        .accessibilityElement(children: .contain)
        .accessibilityIdentifier("commandMenu")
    }

    private var shape: RoundedRectangle {
        RoundedRectangle(cornerRadius: WontonTheme.size.chatControl / 2, style: .continuous)
    }

    private var rows: some View {
        VStack(spacing: 0) {
            ForEach(commands, id: \.identity) { command in
                Button {
                    choose(command)
                } label: {
                    row(command)
                }
                .buttonStyle(.plain)
                .accessibilityIdentifier("command.\(command.name)")
            }
            if omitted > 0 {
                Text(.Composer.composerCommandsOmitted(omitted))
                    .wontonText(WontonTheme.typography.caption)
                    .foregroundStyle(Color(colors.textSecondary))
                    .frame(maxWidth: .infinity, alignment: .leading)
                    .padding(.horizontal, WontonTheme.spacing.s12)
                    .padding(.vertical, WontonTheme.spacing.s8)
            }
        }
    }

    private func row(_ command: RoomCommand) -> some View {
        HStack(spacing: WontonTheme.spacing.s12) {
            avatar(command)
                .frame(width: WontonTheme.size.commandAvatar, height: WontonTheme.size.commandAvatar)
                .accessibilityHidden(true)
            // 名字和参数提示一行，说明在下一行，和 Android 的排法一致（ADR-0109 第 8 节）。
            VStack(alignment: .leading, spacing: WontonTheme.spacing.s4) {
                HStack(spacing: WontonTheme.spacing.s8) {
                    Text(verbatim: "/\(command.name)")
                        .wontonText(WontonTheme.typography.message)
                        .fontWeight(.medium)
                        .foregroundStyle(Color(colors.textPrimary))
                        .lineLimit(1)
                        .layoutPriority(1)
                    if let hint = command.hint {
                        Text(verbatim: hint)
                            .wontonText(WontonTheme.typography.caption)
                            .foregroundStyle(Color(colors.textSecondary))
                            .lineLimit(1)
                    }
                }
                Text(command.description)
                    .wontonText(WontonTheme.typography.supporting)
                    .foregroundStyle(Color(colors.textSecondary))
                    .lineLimit(2)
            }
            .frame(maxWidth: .infinity, alignment: .leading)
            if showsBot {
                Text(command.botName)
                    .wontonText(WontonTheme.typography.caption)
                    .foregroundStyle(Color(colors.textSecondary))
                    .lineLimit(1)
            }
        }
        .padding(.horizontal, WontonTheme.spacing.s12)
        .frame(minHeight: WontonTheme.size.chatControl)
        .contentShape(Rectangle())
    }
}

private extension RoomCommand {
    /// 几个 Bot 可以有同名的命令：按 Bot 和名字一起认。
    var identity: String { "\(botUserId) \(name)" }
}
