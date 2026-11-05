import SwiftUI
import WontonClient

struct TabSettingsView: View {
    let plugins: PluginsModel
    @Environment(TabsModel.self) private var tabs
    @Environment(\.wontonColors) private var colors
    @State private var pin: TabRef?

    var body: some View {
        List {
            if let problem = tabs.viewModel.problem {
                ProblemNotice(problem: Problem(title: TabText.configurationFailed, detail: problem.message))
            }
            ForEach(tabs.viewModel.items, id: \.tab) { item in
                Toggle(isOn: Binding(
                    get: { tabs.isPinned(item.tab) },
                    set: { if !$0 { tabs.send(.unpinTab(tab: item.tab)) } }
                )) {
                    HStack(spacing: WontonTheme.spacing.s12) {
                        TabIconView(item: item, selected: false, plugins: plugins)
                        Text(item.title).wontonText(WontonTheme.typography.body)
                    }
                }
                .toggleStyle(WontonToggleStyle())
                .disabled(!item.removable)
                .listRowBackground(Color(colors.canvas))
            }
            .onMove(perform: move)
            if !available.isEmpty {
                Section {
                    ForEach(available) { choice in
                        Toggle(choice.title, isOn: Binding(
                            get: { tabs.isPinned(choice.tab) }, set: { if $0 { pin = choice.tab } }
                        ))
                        .toggleStyle(WontonToggleStyle())
                        .listRowBackground(Color(colors.canvas))
                    }
                }
            }
        }
        .listStyle(.plain)
        .scrollContentBackground(.hidden)
        .environment(\.editMode, .constant(.active))
        .tabPinRequests($pin)
    }

    private var available: [TabChoice] {
        var choices = [TabChoice(tab: .settings, title: TabText.settings)]
        choices += plugins.viewModel.installed?.plugins.flatMap(\.tabChoices) ?? []
        return choices.filter { !tabs.isPinned($0.tab) }
    }

    private func move(from source: IndexSet, to destination: Int) {
        var order = tabs.viewModel.items.map(\.tab)
        order.move(fromOffsets: source, toOffset: destination)
        for (index, tab) in order.enumerated() where tabs.viewModel.items[index].tab != tab {
            if !tabs.send(.moveTab(tab: tab, index: UInt32(index))) { return }
        }
    }
}

struct PluginTabPins: View {
    let plugin: InstalledPlugin
    @Environment(TabsModel.self) private var tabs
    @State private var pin: TabRef?

    var body: some View {
        if !plugin.tabChoices.isEmpty {
            DetailSection(title: TabText.pin) {
                ForEach(plugin.tabChoices) { choice in
                    Toggle(choice.title, isOn: Binding(
                        get: { tabs.isPinned(choice.tab) },
                        set: { enabled in
                            if enabled { pin = choice.tab } else { tabs.send(.unpinTab(tab: choice.tab)) }
                        }
                    ))
                    .toggleStyle(WontonToggleStyle())
                }
            }
            .tabPinRequests($pin)
        }
    }
}

struct PluginPinButton: View {
    let plugin: InstalledPlugin
    @State private var showsPages = false

    var body: some View {
        if !plugin.tabChoices.isEmpty {
            Button(TabText.pin) { showsPages = true }
                .buttonStyle(WontonSmallButtonStyle())
                .wontonSheet(isPresented: $showsPages, title: plugin.title) { _ in
                    PluginTabPins(plugin: plugin).padding(WontonTheme.spacing.pageInset)
                }
        }
    }
}

extension View {
    func tabPinRequests(_ request: Binding<TabRef?>) -> some View { modifier(TabPinRequests(request: request)) }
}

private struct TabPinRequests: ViewModifier {
    @Binding var request: TabRef?
    @Environment(TabsModel.self) private var tabs

    func body(content: Content) -> some View {
        content
            .onChange(of: request) { _, tab in
                guard let tab, !tabs.viewModel.full else { return }
                tabs.send(.pinTab(tab: tab, replacing: nil))
                request = nil
            }
            .wontonSheet(isPresented: Binding(
                get: { request != nil && tabs.viewModel.full }, set: { if !$0 { request = nil } }
            ), title: TabText.replace) { close in
                VStack(spacing: 0) {
                    ForEach(tabs.viewModel.items.filter(\.removable), id: \.tab) { item in
                        Button {
                            guard let tab = request else { return }
                            close { tabs.send(.pinTab(tab: tab, replacing: item.tab)) }
                        } label: {
                            WontonRow(title: item.title)
                        }
                        .buttonStyle(.plain)
                    }
                }
            }
    }
}
