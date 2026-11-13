import AppIntents
import SwiftUI
import WidgetKit

@available(iOS 18.0, *)
struct AskBotControl: ControlWidget {
    var body: some ControlWidgetConfiguration {
        StaticControlConfiguration(kind: "WontonAskBotControl") {
            ControlWidgetButton(action: OpenSystemEntryIntent(target: .agent)) {
                Label(
                    String(localized: .SystemEntry.systemEntryAskBot),
                    systemImage: "bubble.left.and.text.bubble.right"
                )
            }
        }
        .displayName(.SystemEntry.systemEntryAskBot)
        .description(.SystemEntry.systemEntryAskBotDescription)
    }
}

@available(iOS 18.0, *)
struct PluginControlConfiguration: ControlConfigurationIntent {
    static let title: LocalizedStringResource = "Open plugin"

    @Parameter(title: LocalizedStringResource("Plugin"))
    var plugin: EntryDestination?
}

@available(iOS 18.0, *)
struct OpenPluginControl: ControlWidget {
    var body: some ControlWidgetConfiguration {
        AppIntentControlConfiguration(
            kind: "WontonOpenPluginControl",
            intent: PluginControlConfiguration.self
        ) { configuration in
            ControlWidgetButton(action: OpenSystemEntryIntent(target: configuration.plugin ?? .apps)) {
                Label(
                    configuration.plugin?.title ?? String(localized: .SystemEntry.systemEntryOpenPlugin),
                    systemImage: "square.grid.2x2"
                )
            }
        }
        .displayName(.SystemEntry.systemEntryOpenPlugin)
        .description(.SystemEntry.systemEntryOpenPluginDescription)
    }
}
