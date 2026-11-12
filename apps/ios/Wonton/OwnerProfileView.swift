import SwiftUI
import WontonClient

/// 设置和安装确认页共用的资料页面；保存成功后回到原来的页面。
struct OwnerProfileView: View {
    let model: PluginsModel
    @Environment(\.dismiss) private var dismiss
    @State private var form = OwnerProfileFormState()

    var body: some View {
        VStack(spacing: 0) {
            ScrollView {
                OwnerProfileForm(state: form) { Task { await form.load(core: model.core) } }
                    .frame(maxWidth: .infinity, alignment: .leading)
                    .padding(WontonTheme.spacing.pageInset)
            }
            .scrollDismissesKeyboard(.interactively)
            if form.profile != nil {
                Button(action: save) {
                    Text("保存").frame(maxWidth: .infinity)
                }
                .buttonStyle(WontonButtonStyle())
                .disabled(!form.canSave)
                .padding(WontonTheme.spacing.pageInset)
            }
        }
        .wontonModalScreen("你的资料")
        .task { await form.load(core: model.core) }
    }

    private func save() {
        Task {
            if await form.save(core: model.core) { dismiss() }
        }
    }
}
