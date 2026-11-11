import Foundation

/// 壳负责原生控件的文字；清单标题由核心选择语言。
enum TabText {
    static var chat: String { String(localized: .tabChat) }
    static var apps: String { String(localized: .launcherTitle) }
    static var settings: String { String(localized: .tabSettings) }
    static var bottomBar: String { String(localized: .tabBottomBar) }
    static var bottomPanel: String { String(localized: .tabBottomPanel) }
    static var showLabels: String { String(localized: .tabShowLabels) }
    static var addMore: String { String(localized: .tabAddMore) }
    static var addToBar: String { String(localized: .tabAddToBar) }
    static var pin: String { String(localized: .launcherPinToBar) }
    static var unpin: String { String(localized: .tabRemoveFromBar) }
    static var replace: String { String(localized: .tabReplace) }
    static var back: String { String(localized: .designBack) }
    static var close: String { String(localized: .commonClose) }
    static var retry: String { String(localized: .commonRetry) }
    static var details: String { String(localized: .tabViewDetails) }
    static var open: String { String(localized: .tabOpen) }
    static var pinToBar: String { String(localized: .tabPinToBar) }
    static var fullscreen: String { String(localized: .tabFullscreen) }
    static var alwaysFullscreen: String { String(localized: .tabMenuAlwaysFullscreen) }
    static var reload: String { String(localized: .tabMenuReload) }
    static var more: String { String(localized: .tabMore) }
    static var starting: String { String(localized: .tabStarting) }
    static var loading: String { String(localized: .tabLoading) }
    static var upgrading: String { String(localized: .tabUpgrading) }
    static var uninstalling: String { String(localized: .tabUninstalling) }
    static var pageRemoved: String { String(localized: .tabPageRemoved) }
    static var pageFailed: String { String(localized: .tabPageFailed) }
    static var unknown: String { String(localized: .tabStateUnknown) }
    static var configurationFailed: String { String(localized: .tabSettingsUnreadable) }
    static var selectChat: String { String(localized: .tabSelectChat) }
    static var done: String { String(localized: .commonDone) }
    static var start: String { String(localized: .tabStart) }
    static var failed: String { String(localized: .tabHasProblem) }
    static var openPanel: String { String(localized: .panelOpen) }
    static var addPanelPages: String { String(localized: .panelAddPages) }
    static var editPanel: String { String(localized: .panelEdit) }
    static var inPanel: String { String(localized: .panelInPanel) }
    static var canAddToPanel: String { String(localized: .panelCanAdd) }
    static var removePanelPage: String { String(localized: .panelRemove) }
    static var panelURL: String { String(localized: .panelUrl) }
    static var addPanelURL: String { String(localized: .panelAddUrl) }
    static var pinToPanel: String { String(localized: .panelPin) }
    static var panelBadURL: String { String(localized: .panelBadUrl) }
    static var panelAttention: String { String(localized: .panelAttention) }

    static func stopped(_ title: String) -> String {
        String(localized: .tabStopped(title))
    }

    static func uninstalled(_ title: String) -> String {
        String(localized: .tabUninstalled(title))
    }
}
