#if DEBUG
import SwiftUI
import WontonClient

@testable import Wonton

/// 截图测试（`SnapshotTests`）的固定数据和界面组装。
/// 模型走 App 里 DEBUG 的固定 ViewModel 初始化，不订阅核心；核心只用来给视图取不用网络也能拿到的空列表。
/// 放在测试目标里：界面的文字检查不查这里，假数据不用进 String Catalog。
@MainActor
enum SnapshotFixtures {
    /// 画布大小和预览一致（ADR-0026 的竖屏尺寸），参考图按这个尺寸录。
    static let size = CGSize(width: 390, height: 844)

    /// 固定的消息时刻，2024-03-08 09:41 UTC。取一年以前，时间永远落在“年月日”这一档，
    /// 不会随运行日期变成“昨天”或星期几；格式化按工作流设的 TZ=UTC 和 en_US。
    static let timestampMs: UInt64 = 1_709_890_860_000

    /// 外观、语言和字号都钉死：参考图不能受模拟器的系统设置影响。
    static func wrapped<V: View>(_ view: V) -> some View {
        let colors = defaultAppearance().light
        return view
            .environment(\.wontonColors, colors)
            .environment(\.colorScheme, .light)
            .environment(\.locale, Locale(identifier: "en_US"))
            .environment(\.timeZone, TimeZone(identifier: "UTC")!)
            .environment(\.dynamicTypeSize, .large)
            .fontDesign(.default)
            .tint(Color(colors.primary))
    }

    static func pairing(core: ClientCore) -> some View {
        NavigationStack {
            PairingView(model: homeModel(core: core), camera: .unsupported, deviceName: "iPhone")
                .toolbar(.hidden, for: .navigationBar)
        }
    }

    static func chatList(core: ClientCore) -> some View {
        ChatListView(model: chatModel(core: core), plugins: PluginsModel(core: core))
            .environment(TabsModel(core: core))
    }

    static func chatRoom(core: ClientCore) -> some View {
        ScrollView {
            VStack(spacing: WontonTheme.spacing.s4) {
                row(stamp(), core: core, showsSender: false)
                row(message(id: "m1", sender: "Ada Lovelace", isOwn: false,
                            text: "The engine can repeat a card sequence twice: any two numbers multiply."),
                    core: core, showsSender: true)
                row(message(id: "m2", sender: "Owner", isOwn: true,
                            text: "Good. Bring the tables to the drawing room tomorrow.",
                            readBy: ["Ada Lovelace"]),
                    core: core, showsSender: true)
                row(TimelineItem(id: "read", content: .readMarker), core: core, showsSender: false)
            }
            .padding(.horizontal, WontonTheme.spacing.pageInset)
            .padding(.vertical, WontonTheme.spacing.s8)
        }
        .wontonPushedScreen("Ada Lovelace")
    }

    static func settings(core: ClientCore) -> some View {
        SettingsTab(
            model: settingsModel(core: core),
            appearance: AppearanceModel(core: core),
            chat: ChatModel(core: core),
            home: HomeModel(core: core),
            plugins: PluginsModel(core: core)
        )
        .environment(OnboardingModel(core: core))
    }

    /// 插件问题视图（#230）：「安装记录不可用」类别，标题和下一步按类别写在壳里，核心的原始说明收进“详情”。
    static func pluginProblem() -> some View {
        ProblemNotice(
            problem: PluginFailure.recordUnusable.problem(
                detail: "install record for app.example.photos is not readable: unknown schema"
            )
        )
    }

    private static func row(_ item: TimelineItem, core: ClientCore, showsSender: Bool) -> some View {
        TimelineRow(
            item: item, core: core, showsSender: showsSender,
            joinsPrevious: false, joinsNext: false,
            retry: {}, cancel: {}, cardAction: { _ in }, reply: { _ in }, forward: {}, redact: {}
        )
    }

    private static func stamp() -> TimelineItem {
        TimelineItem(id: "stamp", content: .dateDivider(timestampMs: timestampMs))
    }

    private static func message(
        id: String, sender: String, isOwn: Bool, text: String, readBy: [String] = []
    ) -> TimelineItem {
        let senderId = isOwn ? "@owner:home.example" : "@ada:home.example"
        return TimelineItem(id: id, content: .message(message: TimelineMessage(
            eventId: "$\(id):home.example", senderId: senderId, senderName: sender, isOwn: isOwn,
            timestampMs: timestampMs, sendState: .sent,
            body: .text(body: text, blocks: [
                RichBlock(kind: .paragraph, depth: 0, quoted: false, spans: [
                    RichSpan(text: text, bold: false, italic: false, code: false, strike: false, link: nil)
                ])
            ], files: []),
            edited: false, readBy: readBy, reply: nil, sideChat: nil, actions: [.copy(text: text)]
        )))
    }

    private static func homeModel(core: ClientCore) -> HomeModel {
        HomeModel(
            core: core,
            viewModel: HomeViewModel(
                deviceId: String(repeating: "0", count: 64), serverId: nil, status: .notConfigured,
                path: nil, notice: nil,
                details: ConnectionDetails(
                    network: nil, relay: .unknown, server: .unknown, lastConnectedAtMs: nil,
                    handshakeMs: nil, disconnects: 0, failedDials: 0, sentPackets: 0, lostPackets: 0,
                    quality: .unknown, lossPerMille: nil, rttMs: nil, lastFailure: nil,
                    addressesStale: false, udpFlows: [], udpBufferEvictedDatagrams: nil
                ),
                proxy: nil, webUrl: nil, pairing: nil, invite: nil, keepConnection: false, justRecovered: false
            ),
            servers: ServersViewModel(servers: [], adding: false, matrixLogin: nil)
        )
    }

    private static func chatModel(core: ClientCore) -> ChatModel {
        let layout = ChatLayout(form: .bubbles, textScale: .standard, code: .scroll)
        return ChatModel(core: core, viewModel: ChatViewModel(
            account: .signedIn(userId: "@owner:home.example", deviceId: "OWNERDEVICE"),
            sync: .running,
            encryption: EncryptionStatus(
                verification: .verified, recovery: .enabled, recoveryKey: nil,
                backupReminder: nil, canVerifyWithOtherDevice: false, canReset: false,
                recoveryNextStep: .none, verificationFlow: nil
            ),
            installingBots: [],
            rooms: [
                RoomRow(
                    roomId: "!direct:home.example", name: "Ada Lovelace", isDirect: true,
                    directUserId: "@ada:home.example", isEncrypted: true, isReadOnly: false,
                    membership: .joined,
                    preview: RoomPreview(
                        senderName: "Ada", isOwn: false,
                        body: .text(text: "See you at the lab tomorrow."), timestampMs: timestampMs
                    ),
                    sideChats: false, unreadCount: 2, layout: layout
                ),
                RoomRow(
                    roomId: "!team:home.example", name: "Analytical Engine", isDirect: false,
                    directUserId: nil, isEncrypted: true, isReadOnly: false, membership: .joined,
                    preview: RoomPreview(
                        senderName: "Owner", isOwn: true, body: .files, timestampMs: timestampMs
                    ),
                    sideChats: false, unreadCount: 0, layout: layout
                ),
                RoomRow(
                    roomId: "!invite:home.example", name: "Charles Babbage", isDirect: true,
                    directUserId: "@charles:home.example", isEncrypted: true, isReadOnly: false,
                    membership: .invited, preview: nil, sideChats: false, unreadCount: 0, layout: layout
                )
            ],
            problem: nil, push: .registered, pendingConfirmations: [], settledConfirmations: [],
            backgroundNotifications: [], bots: [],
            chatLayoutDefault: ChatLayoutChoice(form: nil, textScale: nil, code: nil)
        ))
    }

    private static func settingsModel(core: ClientCore) -> SettingsModel {
        SettingsModel(core: core, viewModel: SettingsViewModel(sections: [
            SettingsSection(entries: [.profile, .server(name: nil, status: .connected, upgrade: true)]),
            SettingsSection(entries: [.appearance, .language]),
            SettingsSection(entries: [.peers(count: 2), .friends(count: 3, requests: 1)]),
            SettingsSection(entries: [
                .bottomBar,
                .notifications(method: .keepalive, push: .registered, categories: []),
                .chatBackup(recovery: .enabled)
            ]),
            SettingsSection(entries: [.about])
        ]))
    }

    // MARK: - 确认卡片（ADR-0016）

    /// 会话里的卡片行（输入栏上方）：两张等回答，点开是第一张的完整卡片。
    static func confirmationsBar(core: ClientCore) -> some View {
        let plugins = PluginsModel(core: core)
        return NavigationStack {
            ConfirmationCardBar(
                views: [
                    confirmation(
                        id: "c1", title: "Hermes wants to install File Search", state: .pending,
                        requester: .bot(bot: BotRef(pluginId: "app.hermes", bot: "owner"), userId: "@hermes:home.example"),
                        lines: [CardLine(text: "The plugin can read your photo library.", emphasis: true)],
                        choices: [
                            CardChoice(answer: .approve, label: "Install", choiceId: nil),
                            CardChoice(answer: .reject, label: "Not now", choiceId: nil),
                        ]
                    ),
                    confirmation(id: "c2", title: "File Sync wants to save into Shared folder", state: .pending, requester: .plugin(pluginId: "app.files")),
                ],
                bots: [hermesBot], plugins: plugins,
                answer: { _ in }, stopWindow: {}
            )
            .environment(TabsModel(core: core))
            .environment(plugins)
        }
    }

    /// “等你确认”列表：等回答的在前（Bot 和插件各一张），结束的在后面。
    static func confirmationsList(core: ClientCore) -> some View {
        let plugins = PluginsModel(core: core)
        return NavigationStack {
            ConfirmationsListView(
                view: ConfirmationsViewModel(
                    pending: [
                        confirmation(id: "c1", title: "Hermes wants to install File Search", state: .pending, requester: .bot(bot: BotRef(pluginId: "app.hermes", bot: "owner"), userId: "@hermes:home.example")),
                        confirmation(id: "c2", title: "File Sync wants to save into Shared folder", state: .pending, requester: .plugin(pluginId: "app.files")),
                    ],
                    finished: [
                        confirmation(id: "c3", title: "Hermes wants to install Grafana", state: .succeeded(message: "Installed Grafana 10.2"), requester: .bot(bot: BotRef(pluginId: "app.hermes", bot: "owner"), userId: "@hermes:home.example")),
                    ],
                    loading: false, problem: nil
                ),
                bots: [hermesBot], plugins: plugins,
                answer: { _, _ in }, stopWindow: { _ in }, retry: {}
            )
            .environment(TabsModel(core: core))
            .environment(plugins)
        }
    }

    /// 断网时的列表：上次的列表还在，顶上是可重试的问题提示。
    static func confirmationsListProblem(core: ClientCore) -> some View {
        let plugins = PluginsModel(core: core)
        return NavigationStack {
            ConfirmationsListView(
                view: ConfirmationsViewModel(
                    pending: [confirmation(id: "c1", title: "Hermes wants to install File Search", state: .pending, requester: .bot(bot: BotRef(pluginId: "app.hermes", bot: "owner"), userId: "@hermes:home.example"))],
                    finished: [],
                    loading: false,
                    problem: ChatProblem(reason: .transportUnavailable, message: "GET /_wonton/api/confirmations: connection refused")
                ),
                bots: [hermesBot], plugins: plugins,
                answer: { _, _ in }, stopWindow: { _ in }, retry: {}
            )
            .environment(TabsModel(core: core))
            .environment(plugins)
        }
    }

    private static func confirmation(
        id: String, title: String, state: ConfirmationState, requester: ConfirmationRequester?,
        lines: [CardLine] = [], choices: [CardChoice] = []
    ) -> ConfirmationView {
        ConfirmationView(
            card: ConfirmationCard(
                id: id, state: state, title: title, lines: lines, choices: choices,
                expiresAtMs: timestampMs + 5 * 60_000,
                window: nil, pluginId: nil, secretInputs: [], settings: nil,
                requester: requester, origin: nil
            ),
            sending: nil, sendingChoiceId: nil, problem: nil, window: nil
        )
    }

    /// Hermes 的 Bot 资料：没取到头像，行里落回插件图标占位。
    private static var hermesBot: BotView {
        BotView(
            userId: "@hermes:home.example",
            bot: BotRef(pluginId: "app.hermes", bot: "owner"),
            web: nil, name: "Hermes", avatar: nil, stateAvatars: [], activity: nil,
            saving: false, problem: nil,
            chat: BotChatSettings(
                default: ChatLayoutChoice(form: nil, textScale: nil, code: nil),
                choice: ChatLayoutChoice(form: nil, textScale: nil, code: nil)
            )
        )
    }
}
#endif
