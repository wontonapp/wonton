import SwiftUI
import WontonClient

/// 会话列表：渲染核心给出的 ViewModel，手势都翻译成 `Action` 发回核心。
struct SessionListView: View {
    @Bindable var model: SessionListModel
    @State private var isNamingSession = false
    @State private var draftTitle = ""

    var body: some View {
        NavigationStack {
            List(model.viewModel.sessions, id: \.id) { row in
                SessionRowView(row: row) {
                    model.send(.selectSession(sessionId: row.id))
                }
                .swipeActions {
                    Button("删除", role: .destructive) {
                        model.send(.deleteSession(sessionId: row.id))
                    }
                }
            }
            .overlay {
                if model.viewModel.sessions.isEmpty {
                    ContentUnavailableView("还没有会话", systemImage: "bubble.left.and.bubble.right")
                }
            }
            .navigationTitle("会话")
            .toolbar {
                Button("新建会话", systemImage: "plus") {
                    draftTitle = ""
                    isNamingSession = true
                }
            }
            .alert("新建会话", isPresented: $isNamingSession) {
                TextField("标题", text: $draftTitle)
                Button("创建") { model.send(.createSession(title: draftTitle)) }
                Button("取消", role: .cancel) {}
            }
            .alert("操作被拒绝", isPresented: isShowingRejection, presenting: model.rejection) { _ in
                Button("好") {}
            } message: { reason in
                Text(reason)
            }
        }
        .task { await model.observe() }
    }

    private var isShowingRejection: Binding<Bool> {
        Binding(
            get: { model.rejection != nil },
            set: { isShowing in
                if !isShowing { model.rejection = nil }
            }
        )
    }
}

private struct SessionRowView: View {
    let row: SessionRow
    let select: () -> Void

    var body: some View {
        Button(action: select) {
            HStack {
                Text(row.title)
                    .foregroundStyle(Color.primary)
                Spacer()
                if row.isSelected {
                    Image(systemName: "checkmark")
                        .foregroundStyle(Color.accentColor)
                }
            }
        }
        .accessibilityAddTraits(row.isSelected ? .isSelected : [])
    }
}
