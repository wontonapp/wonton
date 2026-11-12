import SwiftUI
import PhotosUI
import WontonClient

/// 已装 Agent 插件详情页最上面的「资料」一节（ADR-0044）：名字、头像和状态头像，家人线的「谁能用」以后加在这一节下面。
/// 资料存在 Matrix，改动经客户端核心提交；进展和失败都在 `BotView.saving`、`BotView.problem` 里，这里只渲染和转发。
struct BotProfileSection: View {
    let chat: ChatModel
    let plugins: PluginsModel
    let pluginId: String

    @State private var isEditingName = false
    @State private var draftName = ""
    /// 主人刚发的一次改动。`saving` 是整个 Bot 一份，圈画在这次改的控件上，其余控件禁用（核心一次只做一件）。
    @State private var pending: PendingEdit?
    @State private var pickerSlot: AvatarSlot?
    @State private var showsPicker = false
    @State private var pickedItem: PhotosPickerItem?
    @State private var sheetSlot: AvatarSlot?
    @State private var processing: AvatarSlot?
    @State private var processingProgress = 0.0
    /// 选取和转码在壳里，失败不经过核心，就地显示。
    @State private var localProblem: Problem?
    @Environment(\.wontonColors) private var colors

    private enum PendingEdit: Equatable {
        case name
        case staticSet
        case staticRemove
        case stateSet(BotActivity)
        case stateRemove(BotActivity)
    }

    private var bot: BotView? {
        chat.viewModel.bots.first { $0.bot.pluginId == pluginId && $0.bot.bot == "default" }
    }

    private var plugin: InstalledPlugin? {
        plugins.viewModel.installed?.plugins.first { $0.id == pluginId }
    }

    var body: some View {
        if let bot {
            DetailSection(title: String(localized: .BotProfile0.botProfileTitle)) {
                VStack(alignment: .leading, spacing: WontonTheme.spacing.s12) {
                    HStack(alignment: .top, spacing: WontonTheme.spacing.s16) {
                        BotAvatarView(
                            bot: bot, pluginIcon: plugin?.icon, core: plugins.core,
                            model: plugins, size: WontonTheme.size.detailIcon
                        )
                        VStack(alignment: .leading, spacing: WontonTheme.spacing.s8) {
                            nameRow(bot, isBusy: isBusy(bot))
                            staticRow(bot, isBusy: isBusy(bot))
                        }
                    }
                    stateGrid(bot, isBusy: isBusy(bot))
                    if let failure = bot.problem {
                        ProblemNotice(problem: failure.reason.problem(detail: failure.detail)) {
                            dispatch(.dismissBotProfileProblem(pluginId: pluginId))
                        }
                    }
                    if let localProblem {
                        ProblemNotice(problem: localProblem) { self.localProblem = nil }
                    }
                }
            }
            .photosPicker(isPresented: $showsPicker, selection: $pickedItem, matching: pickerSlot?.filter ?? .images)
            .wontonActionSheet(
                isPresented: Binding(
                    get: { sheetSlot != nil },
                    set: { if !$0 { sheetSlot = nil } }
                ),
                actions: sheetActions
            )
            .onChange(of: bot.saving) { _, saving in
                // 这次改完了：成了就退出名字编辑，败了留下草稿；原因显示在提示里（开始保存时核心会清掉旧 problem）。
                guard !saving, pending != nil else { return }
                let failed = bot.problem != nil
                pending = nil
                if !failed { isEditingName = false }
            }
            .onChange(of: pickedItem) { _, item in
                guard let item else { return }
                pickedItem = nil
                guard let slot = pickerSlot else { return }
                pickerSlot = nil
                Task { await process(item, slot) }
            }
        }
    }

    /// 核心对同一个 Bot 的资料一次只做一件；转码也别叠着发起。
    private func isBusy(_ bot: BotView) -> Bool {
        bot.saving || processing != nil
    }

    // MARK: 名字

    @ViewBuilder
    private func nameRow(_ bot: BotView, isBusy: Bool) -> some View {
        let isSaving = pending == .name && bot.saving
        if isEditingName {
            HStack(spacing: WontonTheme.spacing.s8) {
                TextField(String(localized: .BotProfile0.botProfileName), text: $draftName)
                    .textFieldStyle(.plain)
                    .accessibilityIdentifier("botProfileNameField")
                    .wontonField()
                    .submitLabel(.done)
                    .onSubmit { saveName(of: bot, isBusy: isBusy) }
                    .disabled(isSaving)
                Button {
                    saveName(of: bot, isBusy: isBusy)
                } label: {
                    Group {
                        if isSaving {
                            ProgressView()
                        } else {
                            Text(String(localized: .Common.commonSave))
                        }
                    }
                    .frame(minWidth: WontonTheme.size.smallButton)
                }
                .buttonStyle(WontonSmallButtonStyle())
                .disabled(isBusy)
                .accessibilityIdentifier("botProfileSave")
                Button(String(localized: .Common.commonCancel)) {
                    pending = nil
                    isEditingName = false
                }
                .buttonStyle(WontonSmallButtonStyle())
                .disabled(isSaving)
            }
        } else {
            Button {
                draftName = bot.name
                isEditingName = true
            } label: {
                HStack(spacing: WontonTheme.spacing.s4) {
                    Text(bot.name)
                        .wontonText(WontonTheme.typography.itemTitle)
                        .foregroundStyle(Color(colors.textPrimary))
                        .lineLimit(1)
                        .multilineTextAlignment(.leading)
                    WontonIcon.compose.view(size: WontonTheme.size.iconSmall)
                        .foregroundStyle(Color(colors.textSecondary))
                }
                .frame(minHeight: WontonTheme.size.target, alignment: .leading)
                .contentShape(Rectangle())
            }
            .buttonStyle(.plain)
            .accessibilityIdentifier("botProfileName")
            .accessibilityHint(String(localized: .BotProfile1.botProfileNameHint))
        }
    }

    private func saveName(of bot: BotView, isBusy: Bool) {
        guard !isBusy else { return }
        guard draftName != bot.name else {
            isEditingName = false
            return
        }
        pending = .name
        dispatch(.setBotName(pluginId: pluginId, name: draftName))
    }

    /// 进展和失败都记在 `BotView.saving`、`BotView.problem` 里（核心立即拒绝时也会先写 problem），这里吞掉
    /// dispatch 的返回值，不再走 `PluginsModel.rejection` 弹窗：同一个错误只在这一节显示一处。
    private func dispatch(_ action: Action) {
        try? plugins.core.dispatch(action: action)
    }
}

/// 头像的编辑：静态头像行、4 个状态格、选取和转码的提交。放在扩展里，类型体长有门禁，也按职责分开。
private extension BotProfileSection {
    // MARK: 静态头像

    @ViewBuilder
    private func staticRow(_ bot: BotView, isBusy: Bool) -> some View {
        let isSaving = (pending == .staticSet || pending == .staticRemove) && bot.saving
        HStack(spacing: WontonTheme.spacing.s8) {
            Group {
                if isSaving {
                    ProgressView()
                } else if let avatar = bot.avatar {
                    AvatarMediaView(image: avatar, core: plugins.core, size: WontonTheme.size.avatar)
                } else {
                    Circle().fill(Color(colors.surfaceSubtle))
                }
            }
            .frame(width: WontonTheme.size.avatar, height: WontonTheme.size.avatar)
            .clipShape(Circle())
            .overlay {
                Circle().strokeBorder(Color(colors.separator), lineWidth: WontonTheme.size.hairline)
            }
            Button(String(localized: .BotProfile0.botProfilePickImage)) {
                pickerSlot = .staticAvatar
                showsPicker = true
            }
            .buttonStyle(WontonSmallButtonStyle())
            .disabled(isBusy)
            .accessibilityIdentifier("botProfileChooseStatic")
            if bot.avatar != nil {
                Button(String(localized: .BotProfile1.botProfilePickRemove)) {
                    dispatch(.clearBotAvatar(pluginId: pluginId, state: nil))
                    pending = .staticRemove
                }
                .buttonStyle(WontonSmallButtonStyle())
                .disabled(isBusy)
                .accessibilityIdentifier("botProfileRemoveStatic")
            }
        }
    }

    // MARK: 状态头像

    private func stateGrid(_ bot: BotView, isBusy: Bool) -> some View {
        let states = [BotActivity.idle, .working, .waiting, .offline]
        return HStack(alignment: .top, spacing: WontonTheme.spacing.s12) {
            ForEach(Array(states.enumerated()), id: \.offset) { _, state in
                stateCell(bot, state, isBusy: isBusy)
            }
        }
    }

    @ViewBuilder
    private func stateCell(_ bot: BotView, _ state: BotActivity, isBusy: Bool) -> some View {
        let image = bot.stateAvatars.first { $0.state == state }?.image
        let slot = AvatarSlot.state(state)
        VStack(spacing: WontonTheme.spacing.s4) {
            GeometryReader { geo in
                cellFace(state: state, image: image, slot: slot, size: geo.size.width, isBusy: isBusy)
            }
            .aspectRatio(1, contentMode: .fit)
            Text(state.text)
                .wontonText(WontonTheme.typography.caption)
                .foregroundStyle(Color(colors.textSecondary))
                .lineLimit(1)
        }
    }

    @ViewBuilder
    private func cellFace(
        state: BotActivity,
        image: AvatarImage?,
        slot: AvatarSlot,
        size: CGFloat,
        isBusy: Bool
    ) -> some View {
        Button {
            if image != nil {
                sheetSlot = slot
            } else {
                pickerSlot = slot
                showsPicker = true
            }
        } label: {
            ZStack {
                if processing == slot {
                    ProgressView(value: processingProgress)
                } else if let image {
                    AvatarMediaView(image: image, core: plugins.core, size: size)
                } else {
                    Circle().fill(Color(colors.surfaceSubtle))
                    WontonIcon.add.view(size: WontonTheme.size.icon)
                        .foregroundStyle(Color(colors.textSecondary))
                }
            }
            .frame(width: size, height: size)
            .clipShape(Circle())
            .overlay {
                Circle().strokeBorder(Color(colors.separator), lineWidth: WontonTheme.size.hairline)
            }
            .contentShape(Circle())
        }
        .buttonStyle(.plain)
        .disabled(isBusy)
        .accessibilityLabel(
            image == nil
                ? String(localized: .BotProfile0.botProfilePickAddState(state.text))
                : state.text
        )
        .accessibilityIdentifier("botProfileState.\(state.identifier)")
    }

    private var sheetActions: [WontonSheetAction] {
        [
            WontonSheetAction(title: String(localized: .BotProfile0.botProfilePickImageOrVideo), icon: .image) {
                pickerSlot = sheetSlot
                showsPicker = true
                sheetSlot = nil
            },
            WontonSheetAction(title: String(localized: .BotProfile1.botProfilePickRemove), icon: .trash,
                isDanger: true) {
                if let state = sheetSlot?.state {
                    dispatch(.clearBotAvatar(pluginId: pluginId, state: state))
                    pending = .stateRemove(state)
                }
                sheetSlot = nil
            }
        ]
    }

    // MARK: 选取和提交

    private func process(_ item: PhotosPickerItem, _ slot: AvatarSlot) async {
        processing = slot
        processingProgress = 0
        localProblem = nil
        defer { processing = nil }
        do {
            let upload = try await AvatarTranscode.upload(from: item, slot: slot, core: plugins.core) { value in
                Task { @MainActor in processingProgress = value }
            }
            dispatch(.setBotAvatar(pluginId: pluginId, state: slot.state, upload: upload))
            if let state = slot.state {
                pending = .stateSet(state)
            } else {
                pending = .staticSet
            }
        } catch {
            localProblem = (error as? AvatarTranscodeError)?.problem
                ?? Problem(title: String(localized: .BotProfile1.botProfileProblemMediaOther))
        }
    }
}
