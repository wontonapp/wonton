import PhotosUI
import SwiftUI
import WontonClient

struct ChatRoomView: View {
    let model: ChatModel
    let roomId: String
    let title: String
    @State private var timeline: TimelineModel
    @State private var draft = ""
    @State private var photo: PhotosPickerItem?

    init(model: ChatModel, roomId: String, title: String) {
        self.model = model
        self.roomId = roomId
        self.title = title
        _timeline = State(initialValue: TimelineModel(core: model.core, roomId: roomId))
    }

    var body: some View {
        VStack(spacing: 0) {
            if let failure = timeline.failure {
                ContentUnavailableView("打不开这个聊天", systemImage: "exclamationmark.bubble",
                                       description: Text(failure))
            } else {
                messages
            }
            composer
        }
        .navigationTitle(title)
        .navigationBarTitleDisplayMode(.inline)
        .task { await timeline.observe() }
        .onChange(of: photo) { _, item in
            guard let item else { return }
            photo = nil
            Task { await send(item) }
        }
    }

    private var messages: some View {
        ScrollViewReader { scroller in
            ScrollView {
                LazyVStack(alignment: .leading, spacing: 8) {
                    if timeline.backPagination != .reachedStart {
                        ProgressView()
                            .frame(maxWidth: .infinity)
                            // 滚到顶上时（这一行出现）加载更早的消息；一页加载完，这一行还在就再加载一页。
                            .task(id: PageRequest(timeline)) {
                                if timeline.isLoaded, timeline.backPagination == .idle {
                                    model.send(.paginateBackwards(roomId: roomId))
                                }
                            }
                    }
                    ForEach(timeline.items, id: \.id) { item in
                        TimelineRow(
                            item: item, core: model.core, showsSender: !isDirect,
                            retry: { model.send(.retrySend(roomId: roomId, itemId: item.id)) },
                            cancel: { model.send(.cancelSend(roomId: roomId, itemId: item.id)) }
                        )
                        .id(item.id)
                    }
                }
                .padding(.horizontal)
                .padding(.vertical, 8)
            }
            .defaultScrollAnchor(.bottom)
            .onChange(of: timeline.items.last?.id) { _, last in
                guard let last else { return }
                withAnimation { scroller.scrollTo(last, anchor: .bottom) }
                model.send(.markRoomRead(roomId: roomId))
            }
        }
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
            TextField("消息", text: $draft, axis: .vertical)
                .lineLimit(1...5)
                .textFieldStyle(.roundedBorder)
            Button("发送") {
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
            model.send(.sendImage(roomId: roomId, image: upload))
        } catch {
            model.rejection = String(describing: error)
        }
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
