import Foundation

/// 壳负责原生控件的文字；清单标题由核心选择语言。
enum TabText {
    static var chat: String { String(localized: .Tab2.tabChat) }
    static var apps: String { String(localized: .Launcher.launcherTitle) }
    static var settings: String { String(localized: .Tab2.tabSettings) }
    static var bottomBar: String { String(localized: .Tab0.tabBottomBar) }
    static var bottomPanel: String { String(localized: .Tab2.tabBottomPanel) }
    static var showLabels: String { String(localized: .Tab2.tabShowLabels) }
    static var addMore: String { String(localized: .Tab1.tabAddMore) }
    static var addToBar: String { String(localized: .Tab1.tabAddToBar) }
    static var pin: String { String(localized: .Launcher.launcherPinToBar) }
    static var unpin: String { String(localized: .Tab1.tabRemoveFromBar) }
    static var replace: String { String(localized: .Tab2.tabReplace) }
    static var back: String { String(localized: .Design.designBack) }
    static var close: String { String(localized: .Common.commonClose) }
    static var retry: String { String(localized: .Common.commonRetry) }
    static var details: String { String(localized: .Tab0.tabViewDetails) }
    static var open: String { String(localized: .Tab1.tabOpen) }
    static var pinToBar: String { String(localized: .Tab2.tabPinToBar) }
    static var fullscreen: String { String(localized: .Tab2.tabFullscreen) }
    static var alwaysFullscreen: String { String(localized: .Tab0.tabMenuAlwaysFullscreen) }
    static var reload: String { String(localized: .Tab0.tabMenuReload) }
    static var more: String { String(localized: .Tab2.tabMore) }
    static var starting: String { String(localized: .Tab1.tabStarting) }
    static var loading: String { String(localized: .Tab1.tabLoading) }
    static var upgrading: String { String(localized: .Tab2.tabUpgrading) }
    static var uninstalling: String { String(localized: .Tab1.tabUninstalling) }
    static var pageRemoved: String { String(localized: .Tab2.tabPageRemoved) }
    static var pageFailed: String { String(localized: .Tab0.tabPageFailed) }
    static var unknown: String { String(localized: .Tab2.tabStateUnknown) }
    static var configurationFailed: String { String(localized: .Tab1.tabSettingsUnreadable) }
    static var selectChat: String { String(localized: .Tab1.tabSelectChat) }
    static var done: String { String(localized: .Common.commonDone) }
    static var start: String { String(localized: .Tab2.tabStart) }
    static var failed: String { String(localized: .Tab0.tabHasProblem) }
    static var openPanel: String { String(localized: .Panel.panelOpen) }
    static var addPanelPages: String { String(localized: .Panel.panelAddPages) }
    static var editPanel: String { String(localized: .Panel.panelEdit) }
    static var inPanel: String { String(localized: .Panel.panelInPanel) }
    static var canAddToPanel: String { String(localized: .Panel.panelCanAdd) }
    static var removePanelPage: String { String(localized: .Panel.panelRemove) }
    static var panelURL: String { String(localized: .Panel.panelUrl) }
    static var addPanelURL: String { String(localized: .Panel.panelAddUrl) }
    static var pinToPanel: String { String(localized: .Panel.panelPin) }
    static var panelBadURL: String { String(localized: .Panel.panelBadUrl) }
    static var panelAttention: String { String(localized: .Panel.panelAttention) }

    static func stopped(_ title: String) -> String {
        String(localized: .Tab0.tabStopped(title))
    }

    static func uninstalled(_ title: String) -> String {
        String(localized: .Tab2.tabUninstalled(title))
    }
}
