#if DEBUG
import SwiftUI
import UIKit
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

    /// 带 Home 条的竖屏 iPhone 的系统栏高度（状态栏 47、Home 条 34）。
    static let safeArea = UIEdgeInsets(top: 47, left: 0, bottom: 34, right: 0)

    /// 侧边栏容器配不画底的主内容和面板：系统栏那两条是什么颜色，只看容器自己铺的底。
    static func sidebar(isOpen: Bool) -> some View {
        SidebarContainer(isOpen: .constant(isOpen), edgeSwipe: false) {
            Text("Chats").frame(maxWidth: .infinity, maxHeight: .infinity, alignment: .top)
        } panel: {
            Text("Sidebar").frame(maxWidth: .infinity, maxHeight: .infinity, alignment: .top)
        }
    }

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
            .environment(PagesModel(core: core))
    }

    static func wideChat(core: ClientCore) -> some View {
        let model = chatModel(core: core)
        let plugins = PluginsModel(core: core)
        return NavigationSplitView(columnVisibility: .constant(.all)) {
            ChatListView(model: model, plugins: plugins, listOnly: true)
                .navigationSplitViewColumnWidth(320)
        } detail: {
            ChatDetailView(model: model, plugins: plugins)
        }
        .navigationSplitViewStyle(.balanced)
        .environment(PagesModel(core: core))
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
            appLock: AppLockModel(core: core),
            chat: ChatModel(core: core),
            home: HomeModel(core: core),
            plugins: PluginsModel(core: core)
        )
        .environment(OnboardingModel(core: core))
        // 版本号钉死：设置页直接读 Bundle 版本，不钉的话参考图随无 tag 的 Debug 构建漂（构建号 1→0）。
        .environment(\.appVersionOverride, "0.0.0 (0)")
    }

    /// 插件问题视图（#230）：「安装记录不可用」类别，标题和下一步按类别写在壳里，核心的原始说明收进“详情”。
    static func pluginProblem() -> some View {
        ProblemNotice(
            problem: PluginFailure.recordUnusable.problem(
                detail: "install record for app.example.photos is not readable: unknown schema"
            )
        )
    }

    /// 分享页（ADR-0097）面板里的内容：分享进来三项和一句话，去向是默认 Agent，提示丢了两项。
    /// 照片的路径故意不存在，缩略图走占位图标，不读磁盘。
    static func shareSheet() -> some View {
        let layout = ChatLayout(form: .bubbles, textScale: .standard, code: .wrap)
        let agent = RoomRow(
            roomId: "!agent:home.example", name: "Agent", isDirect: true, directUserId: "@agent:home.example",
            isEncrypted: true, isReadOnly: false, membership: .joined, preview: nil,
            sideChats: false, unreadCount: 0, layout: layout
        )
        let family = RoomRow(
            roomId: "!family:home.example", name: "Family", isDirect: false, directUserId: nil,
            isEncrypted: true, isReadOnly: false, membership: .joined, preview: nil,
            sideChats: false, unreadCount: 0, layout: layout
        )
        func item(_ id: String, _ name: String, image: Bool) -> ShareInItem {
            ShareInItem(id: id, kind: image ? .image(width: nil, height: nil) : .file, name: name,
                        mimeType: image ? "image/jpeg" : "application/pdf", size: 1_200_000, path: "/nonexistent/\(id)")
        }
        let share = PendingShare(
            id: "snapshot",
            items: [item("0", "IMG_0001.JPG", image: true), item("1", "IMG_0002.JPG", image: true),
                    item("2", "Floor plan.pdf", image: false)],
            text: "Which one looks better?",
            destinations: .ready(rooms: [agent, family], suggestedRoom: agent.roomId),
            dropped: 2
        )
        return VStack {
            Spacer()
            ShareInSheetContent(share: share, sending: false, avatar: { row in
                Text(row.name.prefix(1))
                    .frame(width: WontonTheme.size.avatar, height: WontonTheme.size.avatar)
                    .background(Circle().fill(Color.gray.opacity(0.2)))
            }, pair: {}, remove: { _ in }, send: { _, _ in })
        }
    }

    // MARK: 系统入口（ADR-0100）的小组件

    /// 小组件的假快照，数据同 Android 的 WidgetSnapshotsTest。更新时刻钉在“1 分钟前”：
    /// 底部相对时间（`Text(date, style: .relative)`）渲染成“1 minute ago”，参考图不随日期漂移。
    /// 颜色按 ADR-0026 默认浅色的令牌值写死（扩展侧的 Palette 不链接核心，测试替身照抄协议的 JSON）。
    static func entrySnapshot(account: EntrySnapshot.Account = .signedIn) -> EntrySnapshot {
        let palette = EntrySnapshot.Palette(
            surface: EntrySnapshot.RGB(red: 255, green: 255, blue: 255),
            textPrimary: EntrySnapshot.RGB(red: 17, green: 17, blue: 17),
            textSecondary: EntrySnapshot.RGB(red: 94, green: 94, blue: 94),
            accentFill: EntrySnapshot.RGB(red: 254, green: 235, blue: 234),
            accentInk: EntrySnapshot.RGB(red: 155, green: 73, blue: 74),
            accentInkOnFill: EntrySnapshot.RGB(red: 155, green: 73, blue: 74)
        )
        return EntrySnapshot(
            version: 1,
            updatedAtMs: UInt64((Date.now.addingTimeInterval(-90)).timeIntervalSince1970 * 1_000),
            account: account,
            unreadTotal: 5,
            rooms: [
                EntrySnapshot.Room(
                    roomId: "!family:home.example", name: "Family", unreadCount: 3,
                    preview: EntrySnapshot.Preview(
                        senderName: "Ann", isOwn: false,
                        body: EntrySnapshot.PreviewBody(type: "text", text: "Dinner at seven?", title: nil, state: nil)
                    )
                ),
                EntrySnapshot.Room(
                    roomId: "!hermes:home.example", name: "Hermes", unreadCount: 2,
                    preview: EntrySnapshot.Preview(
                        senderName: "Hermes", isOwn: false,
                        body: EntrySnapshot.PreviewBody(type: "text", text: "Here's the summary you asked for.", title: nil, state: nil)
                    )
                ),
                EntrySnapshot.Room(
                    roomId: "!research:home.example", name: "Research", unreadCount: 0,
                    preview: EntrySnapshot.Preview(
                        senderName: "Hermes", isOwn: false,
                        body: EntrySnapshot.PreviewBody(type: "side_chat", text: nil, title: "Research", state: "done")
                    )
                ),
                EntrySnapshot.Room(
                    roomId: "!ann:home.example", name: "Ann", unreadCount: 0,
                    preview: EntrySnapshot.Preview(
                        senderName: "Me", isOwn: true,
                        body: EntrySnapshot.PreviewBody(type: "text", text: "See you tomorrow", title: nil, state: nil)
                    )
                ),
                EntrySnapshot.Room(roomId: "!trip:home.example", name: "Trip planning", unreadCount: 0, preview: nil),
            ],
            defaultAgent: EntrySnapshot.Agent(name: "Hermes", activity: "working", unreadCount: 2),
            plugins: [EntrySnapshot.Plugin(pluginId: "immich", title: "Immich")],
            light: palette,
            dark: palette
        )
    }

    static func widget(
        kind: EntryWidgetView.Kind,
        family: EntryWidgetView.Family,
        account: EntrySnapshot.Account = .signedIn
    ) -> some View {
        EntryWidgetView(kind: kind, state: .ready(entrySnapshot(account: account)), family: family)
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
            edited: false, readBy: readBy, reply: nil, sideChat: nil, preview: nil, actions: [.copy(text: text)]
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
                .notifications(method: .keepalive, push: .registered, categories: []),
                .chatBackup(recovery: .enabled)
            ]),
            SettingsSection(entries: [.about])
        ]))
    }
}

#Preview("Wide chat") {
    let directory = FileManager.default.temporaryDirectory.appendingPathComponent("wide-chat-preview")
    if let core = try? ClientCore(dataDir: directory.path) {
        SnapshotFixtures.wrapped(SnapshotFixtures.wideChat(core: core))
            .environment(\.horizontalSizeClass, .regular)
            .frame(width: 1194, height: 834)
    }
}
#endif
