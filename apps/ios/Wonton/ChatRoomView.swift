import PhotosUI
import SwiftUI
import WontonClient

struct ChatRoomView: View {
    let model: ChatModel
    let roomId: String
    let title: String
    @State private var timeline: TimelineModel
    @State private var presenter: RoomPresenter
    @State private var draft = ""
    @State private var pin = BottomPin()
    @State private var pageAnchor: String?
    @State private var photo: PhotosPickerItem?
    @State private var isPickingFiles = false

    init(model: ChatModel, roomId: String, title: String) {
        self.model = model
        self.roomId = roomId
        self.title = title
        _timeline = State(initialValue: TimelineModel(core: model.core, roomId: roomId))
        _presenter = State(initialValue: RoomPresenter(files: model.files))
    }

    var body: some View {
        VStack(spacing: 0) {
            if let failure = timeline.failure {
                ContentUnavailableView("打不开这个聊天", systemImage: "exclamationmark.bubble",
                                       description: Text(failure))
            } else {
                messages
            }
            TransfersStrip(roomId: roomId)
            composer
        }
        .modifier(RoomPresentations(presenter: presenter))
        .environment(model.files)
        .environment(presenter)
        .navigationTitle(title)
        .navigationBarTitleDisplayMode(.inline)
        // 留着标签栏时，键盘弹出后输入框和发送按钮被盖在键盘下面（iOS 26.5 模拟器实测）。
        .toolbar(.hidden, for: .tabBar)
        .task { await timeline.observe() }
        .onChange(of: photo) { _, item in
            guard let item else { return }
            photo = nil
            Task { await send(item) }
        }
        .fileImporter(isPresented: $isPickingFiles, allowedContentTypes: [.item], allowsMultipleSelection: true) {
            switch $0 {
            case let .success(urls):
                pin.pin()
                for url in urls { model.files.upload(url, room: roomId) }
            case let .failure(error):
                model.rejection = error.localizedDescription
            }
        }
    }

    private var messages: some View {
        ScrollViewReader { scroller in
            // 不用 ScrollView + LazyVStack：行高相差悬殊时（一行和六十行）它的内容高度全靠估算，同样的数据几毫秒内差出几千点，
            // 滚动位置落在没有画任何行的地方，整屏空白（iOS 26.5 模拟器实测）。List 量过的行高不再变。
            List {
                // 这一行常驻，只换内容：List 的结构变了（条件行增删）会把滚动位置重置到顶上（iOS 26.5 模拟器实测）。
                Group {
                    if timeline.backPagination != .reachedStart {
                        ProgressView()
                            .frame(maxWidth: .infinity)
                            // 滚到顶上时（这一行出现）加载更早的消息；一页加载完，这一行还在就再加载一页。
                            .task(id: PageRequest(timeline)) {
                                // 第一批消息到时这一行闪现一下，紧接着就滚到底了：停得住才算翻到了顶，滚走时这里被取消。
                                guard (try? await Task.sleep(for: .milliseconds(300))) != nil else { return }
                                if timeline.isLoaded, timeline.backPagination == .idle {
                                    pageAnchor = oldestEvent
                                    model.send(.paginateBackwards(roomId: roomId))
                                }
                            }
                            .onDisappear { pageAnchor = nil }
                    } else {
                        Color.clear.frame(height: 1)
                    }
                }
                // 进房间时列表从头排起，这一行一定会出现。
                .background(EnclosingScrollView { pin.attach($0) })
                .timelineListRow()
                ForEach(timeline.items, id: \.id) { item in
                    TimelineRow(
                        item: item, core: model.core, showsSender: !isDirect,
                        retry: { model.send(.retrySend(roomId: roomId, itemId: item.id)) },
                        cancel: { model.send(.cancelSend(roomId: roomId, itemId: item.id)) },
                        answer: { model.send(.answerConfirmation(id: $0, answer: $1)) }
                    )
                    .id(item.id)
                    .timelineListRow()
                }
            }
            .listStyle(.plain)
            .scrollContentBackground(.hidden)
            .environment(\.defaultMinListRowHeight, 1)
            .scrollDismissesKeyboard(.interactively)
            // 翻页把更早的消息插到最上面，List 给的位置不对（iOS 26.5 模拟器实测差了一千多点）：把原来最上面那条留在原位。
            // 第一行变了不一定是翻页（日期分隔线的 id 会重新生成），所以只在翻页途中做。这条离末尾很远，条目变少也不会越界。
            .onChange(of: timeline.items.first?.id) {
                guard let pageAnchor, !pin.isPinned else { return }
                DispatchQueue.main.async { scroller.scrollTo(pageAnchor, anchor: .top) }
            }
            .onChange(of: timeline.items.last?.id) { _, last in
                if last != nil { model.send(.markRoomRead(roomId: roomId)) }
            }
            .onGeometryChange(for: CGFloat.self) { $0.size.height } action: { _ in
                pin.stick()
            }
        }
    }

    /// 分隔线这类虚拟条目的 id 不稳定，定位用消息。
    private var oldestEvent: String? {
        timeline.items.first { item in
            switch item.content {
            case .dateDivider, .readMarker, .timelineStart: false
            default: true
            }
        }?.id
    }

    private var isDirect: Bool {
        model.viewModel.rooms.first { $0.roomId == roomId }?.isDirect ?? false
    }

    private var composer: some View {
        HStack(alignment: .bottom, spacing: 8) {
            PhotosPicker(selection: $photo, matching: .images) {
                Image(systemName: "photo").font(.title3)
            }
            .accessibilityLabel("发照片")
            Button { isPickingFiles = true } label: {
                Image(systemName: "paperclip").font(.title3)
            }
            .accessibilityLabel("发文件")
            TextField("消息", text: $draft, axis: .vertical)
                .lineLimit(1...5)
                .textFieldStyle(.roundedBorder)
            Button("发送") {
                pin.pin()
                model.send(.sendText(roomId: roomId, body: draft))
                draft = ""
            }
            .disabled(draft.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty)
        }
        .padding(8)
        .background(.bar)
    }

    /// 解码和转码放到后台：大照片在主线程上转码会卡住界面。
    private func send(_ item: PhotosPickerItem) async {
        guard let data = try? await item.loadTransferable(type: Data.self) else {
            model.rejection = PhotoUpload.Failure.unreadable.description
            return
        }
        let type = item.supportedContentTypes.first
        do {
            let upload = try await Task.detached(priority: .userInitiated) {
                try PhotoUpload.make(data: data, type: type)
            }.value
            pin.pin()
            model.send(.sendImage(roomId: roomId, image: upload))
        } catch {
            model.rejection = String(describing: error)
        }
    }
}

private extension View {
    func timelineListRow() -> some View {
        // 默认样式的按钮放在 List 的行里，点这一行任何地方都会触发。
        buttonStyle(.borderless)
            .listRowSeparator(.hidden)
            .listRowBackground(Color.clear)
            .listRowInsets(EdgeInsets(top: 4, leading: 16, bottom: 4, trailing: 16))
    }
}

/// 翻页的时机：时间线就绪、条目数或翻页状态变了，再看一次要不要加载更早的消息。
private struct PageRequest: Equatable {
    let isLoaded: Bool
    let count: Int
    let state: BackPagination

    @MainActor
    init(_ timeline: TimelineModel) {
        isLoaded = timeline.isLoaded
        count = timeline.items.count
        state = timeline.backPagination
    }
}
