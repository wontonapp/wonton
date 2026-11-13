import SwiftUI
import WontonClient

/// 编辑面板里能加的一项：插件页，或（房间面板）主面板里已有的页面。
struct PanelCandidate: Identifiable {
    let tab: TabRef
    let title: String
    let icon: PluginIcon?
    var id: TabRef { tab }
}

extension TabCandidate {
    var panelCandidate: PanelCandidate? {
        guard let plugin else { return nil }
        let title = plugin.pageTitle.map { "\(plugin.title) · \($0)" } ?? plugin.title
        return PanelCandidate(tab: tab, title: title, icon: plugin.icon)
    }
}

/// 编辑面板（ADR-0052）：上面是面板里的页面，下面是还能放进来的和“填写网址”。
/// 主面板和房间面板共用，改动发回核心，核心写账号数据。
struct PanelEditor: View {
    let items: [PanelItem]
    let candidates: [PanelCandidate]
    let plugins: PluginsModel
    let add: (TabRef) -> Void
    let remove: (TabRef) -> Void
    let move: (TabRef, Int) -> Void
    let addURL: (String) -> Void
    @Environment(\.dismiss) private var dismiss
    @Environment(PagesModel.self) private var pages
    @Environment(\.wontonColors) private var colors
    @State private var url = ""
    @State private var badURL = false

    var body: some View {
        NavigationStack {
            List {
                if let problem = pages.rejection {
                    Section {
                        ProblemNotice(problem: problem) { pages.rejection = nil }
                            .listRowBackground(Color(colors.canvas))
                    }
                }
                if !items.isEmpty {
                    Section(TabText.inPanel) {
                        ForEach(items, id: \.tab) { item in
                            HStack {
                                Text(item.page.title).wontonText(WontonTheme.typography.body)
                                Spacer(minLength: 0)
                                Button(TabText.removePanelPage) { remove(item.tab) }
                                    .buttonStyle(WontonSmallButtonStyle())
                            }
                            .listRowBackground(Color(colors.canvas))
                        }
                        .onMove(perform: reorder)
                    }
                }
                if !candidates.isEmpty {
                    Section(TabText.canAddToPanel) {
                        ForEach(candidates) { candidate in
                            Button { add(candidate.tab) } label: {
                                HStack(spacing: WontonTheme.spacing.s12) {
                                    if let icon = candidate.icon {
                                        PluginIconView(icon: icon, size: WontonTheme.size.icon, model: plugins)
                                            .accessibilityHidden(true)
                                    }
                                    Text(candidate.title).wontonText(WontonTheme.typography.body)
                                    Spacer(minLength: 0)
                                    WontonIcon.add.view().accessibilityHidden(true)
                                }
                                .frame(minHeight: WontonTheme.size.target)
                                .contentShape(Rectangle())
                            }
                            .buttonStyle(.plain)
                            .listRowBackground(Color(colors.canvas))
                        }
                    }
                }
                urlSection
            }
            .listStyle(.plain)
            .scrollContentBackground(.hidden)
            .environment(\.editMode, .constant(.active))
            .navigationTitle(TabText.editPanel)
            .navigationBarTitleDisplayMode(.inline)
            .toolbar {
                ToolbarItem(placement: .confirmationAction) { Button(TabText.done) { dismiss() } }
            }
        }
        .environment(plugins)
    }

    private var urlSection: some View {
        Section(TabText.panelURL) {
            HStack(spacing: WontonTheme.spacing.s8) {
                TextField("https://", text: $url)
                    .textInputAutocapitalization(.never)
                    .autocorrectionDisabled()
                    .keyboardType(.URL)
                    .wontonText(WontonTheme.typography.body)
                    .onSubmit(submit)
                Button(TabText.addPanelURL, action: submit)
                    .buttonStyle(WontonSmallButtonStyle())
                    .disabled(url.trimmingCharacters(in: .whitespaces).isEmpty)
            }
            .listRowBackground(Color(colors.canvas))
            if badURL {
                Text(TabText.panelBadURL)
                    .wontonText(WontonTheme.typography.supporting)
                    .foregroundStyle(Color(colors.textSecondary))
                    .listRowBackground(Color(colors.canvas))
            }
        }
    }

    private func submit() {
        let value = url.trimmingCharacters(in: .whitespacesAndNewlines)
        guard let parsed = URL(string: value), parsed.scheme == "https", parsed.host != nil else {
            badURL = true
            return
        }
        addURL(value)
        url = ""
        badURL = false
    }

    private func reorder(from source: IndexSet, to destination: Int) {
        var order = items.map(\.tab)
        order.move(fromOffsets: source, toOffset: destination)
        for (index, tab) in order.enumerated() where items[index].tab != tab {
            move(tab, index)
        }
    }
}
