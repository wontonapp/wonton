import SwiftUI
import WontonClient

struct PluginAudienceSection: View {
    @Binding var audience: PluginAudienceView
    var enabled = true

    var body: some View {
        DetailSection(title: String(localized: .Install.installAudienceTitle)) {
            ForEach(audience.choices.indices, id: \.self) { index in
                Toggle(isOn: Binding(
                    get: { audience.choices[index].required || audience.choices[index].selected },
                    set: { audience.choices[index].selected = $0 }
                )) {
                    Text(audienceRoleName(audience.choices[index]))
                }
                .toggleStyle(WontonToggleStyle())
                .disabled(!enabled || audience.choices[index].required)
            }
            if pluginAudienceNotice(view: audience) == .sharedData {
                WontonNotice(icon: .warning, title: String(localized: .Install.installAudienceSharedData), isError: false)
            }
        }
    }
}

private func audienceRoleName(_ choice: PluginAudienceChoice) -> String {
    switch choice.template {
    case .admin: String(localized: .Install.installAudienceAdmin)
    case .family: String(localized: .Install.installAudienceFamily)
    case .child: String(localized: .Install.installAudienceChild)
    case .friend: String(localized: .Install.installAudienceFriend)
    case .other, nil: choice.name
    }
}

struct InstalledPluginAudience: View {
    let model: PluginsModel
    let id: String
    let view: PluginAudienceView
    let updated: (PluginDetail) -> Void
    @State private var audience: PluginAudienceView
    @State private var busy = false
    @State private var problem: Problem?

    init(model: PluginsModel, id: String, view: PluginAudienceView, updated: @escaping (PluginDetail) -> Void) {
        self.model = model
        self.id = id
        self.view = view
        self.updated = updated
        _audience = State(initialValue: view)
    }

    var body: some View {
        VStack {
            PluginAudienceSection(audience: $audience, enabled: !busy)
            Button(String(localized: .Common.commonSave)) {
                guard let selected = selectedPluginAudience(view: audience) else { return }
                busy = true
                Task {
                    do {
                        updated(try await model.core.updatePluginAudience(action: .setPluginAudience(id: id, audience: selected)))
                        problem = nil
                    } catch { problem = error.problem }
                    busy = false
                }
            }
            .buttonStyle(WontonButtonStyle(kind: .secondary))
            .disabled(busy || audience == view)
            if let problem { ProblemNotice(problem: problem) }
        }
        .onChange(of: view) { _, value in audience = value }
    }
}
