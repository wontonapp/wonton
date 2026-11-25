#!/usr/bin/env python3
"""ADR-0120 现状盘点的统计：按页数两个壳的行数和近一段时间的改动。

只统计 `--rev` 可达的历史；`--rev` 必填，免得默认的 main 因本地分支停在哪而漂。
`--since` 只是再缩小时间窗口。

两套口径都打，别混：
- 按提交：范围里的提交数，和其中同时改了 apps/ios 和 apps/compose 的提交数。
- 按（提交，页面）对：同一个提交落在某页算 1 个对，改了多页时每页各算 1。表的各页之和等于合计。
- 表的 iOS 提交、Android 提交两列两端各算一次：同一个提交两端都改了，两列各记 1。
- “两端同改”是同时改了该页 iOS 和 Android 文件的提交。
- 行数是该页 iOS 或 Android 文件的物理行数之和（含空行和注释），文件不存在按 0 算。

用法（仓库根目录）：

    python3 scripts/adr-shell-pages.py --rev 225ea5cb --since 2026-10-25
"""


from __future__ import annotations

import argparse
import os
import subprocess
import sys

IOS = "apps/ios/Wonton/"
ANDROID = "apps/compose/shared/src/androidMain/kotlin/app/wonton/ui/"

# 页 → 两端归到这一页的壳文件和组件文件。改页的边界时两边一起改。
PAGES: dict[str, tuple[list[str], list[str]]] = {
    "聊天房间（时间线）": (
        ["ChatRoomView", "TimelineRow", "TimelineModel", "TimelineOverlays", "ChatRoomBar", "ChatRoomChrome",
         "ChatRoomJump", "ChatWallpaper", "MessageTexts", "MessageMeta", "MessageChoices", "MessageFileView",
         "MessageMedia", "LinkedMessageText", "RoomPresenter"],
        ["RoomScreen", "TimelineItemView", "TimelineList", "TimelineGrouping", "TimelineJump", "RoomScreenNotices",
         "RoomScroll", "RoomBackdrop", "MessageTexts", "MessageMenu", "MessageFile", "MessageChoicesRow",
         "MessageSummaryText", "RoomRequest", "RoomDraft"],
    ),
    "插件详情（含 Bot 资料、联系人、共享、密钥、权限）": (
        ["PluginDetailView", "BotProfileView", "BotProfileSection", "BotContactsView", "BotContactsRows",
         "PluginIconView", "AgentRunRow", "AgentStopControl", "AgentPluginStatusBanner",
         "PluginGrantedDevicesSection", "PluginSharesSection", "PluginShareEditor", "PluginAudienceSection",
         "ToolPolicySection", "PortMappingSection", "InstalledPermissions"],
        ["PluginDetailScreen", "BotProfileScreen", "BotProfileSection", "BotContactsScreen", "BotContactRows",
         "BotContactDialogs", "AgentRunRow", "AgentStopControl", "AgentPluginStatusBanner",
         "GrantedDevicesSection", "PluginSharesSection", "ShareGrantEditor", "PluginAudienceSection",
         "ToolPolicySection", "PortMappingSection", "InstalledPermissions", "PluginBack"],
    ),
    "插件网页": (
        ["TabPageView", "TabWebView", "TabWebNavigation", "TabWebScripts", "TabPageCapsule", "TabPageTopStrip",
         "TabPageStatusLine", "TabPageStateView", "TabReloadTarget", "TabTexts", "TabWidgetPage",
         "TabMediaPermission", "PageTakeover", "PageExitHint"],
        ["PluginWebView", "PluginPagesHost", "PluginPageLayers", "PluginNavigation", "PluginChromeClient",
         "PluginFileChooser", "PluginProxyRoute", "TabPageCapsule", "TabPageChrome", "TabViewport", "TabStatus",
         "TabTexts", "TabMediaPermissionFlow", "TabDraft", "PageTakeover", "PageExitHint", "PageCapsuleState",
         "PageThemeColor", "WidgetPage"],
    ),
    "侧边栏与根导航": (
        ["Sidebar", "SidebarAppMenu", "MainView", "MainView+Sidebar", "MainView+Layout", "MainView+Navigation",
         "MainView+PageLayer", "AppNavigation", "EdgeBack"],
        ["Sidebar", "SidebarRows", "AdaptiveChatLayout", "WontonApp", "AppNavigation", "AppShortcuts",
         "LauncherScreen"],
    ),
    "聊天面板": (
        ["ChatPanel", "ChatPanelGesture", "PanelEditor", "BottomPin"],
        ["ChatPanel", "ChatPanelEditor", "ChatPanelGestures", "ChatPanelPage", "ChatPanelState", "ChatPullPanel",
         "ChatDatePill"],
    ),
    "服务器、登录、邀请、配对": (
        ["ServersSection", "MatrixLoginView", "MatrixLoginBrowser", "InviteView", "PairingView", "QRImage"],
        ["ServerSettings", "ServerHost", "Servers", "MatrixLogin", "MatrixLoginScreen", "InviteScreen",
         "PairingScreen", "PairingDraft"],
    ),
    "图片查看、文件播放": (
        ["ImageViewer", "MessageMedia", "FilePlayer", "FileCardView"],
        ["ImageViewer", "ImageViewerPaging", "ImageViewerGestures", "ImageViewerTransform", "ImageGallery",
         "ImageOperations", "FilePlayer", "FileCardView", "MessageImage", "OriginalCache"],
    ),
    "确认卡片、待办": (
        ["ConfirmationCardView", "ConfirmationCardRow", "ConfirmationsListView", "ConfirmationsPage",
         "ConfirmationsModel", "PendingCardBanner", "RoomConfirmations"],
        ["ConfirmationCard", "ConfirmationCardRow", "ConfirmationsScreen", "ConfirmationsUpdates",
         "ConfirmationContactsState", "ConfirmationRemaining", "PendingCardBanner", "RoomConfirmations"],
    ),
    "问题与空态": (
        ["ProblemViews", "ProblemPreview"],
        ["ProblemViews"],
    ),
    "应用商店": (
        ["StoreView", "StoreUpdateAll"],
        ["StoreScreen", "StoreUpdateAll", "StoreLoadViews", "StoreUnreadable"],
    ),
    "其他服务器": (
        ["PeersView", "PeersRows", "PeersScanner", "PeersPreview", "PeerCodeDialog"],
        ["PeersScreen", "PeerRequests", "PeerScanScreen", "PeerPageContent", "PeerCodeDialog"],
    ),
    "好友": (
        ["FriendsView", "FriendsRows", "FriendCodeViews"],
        ["FriendsScreen", "FriendRows", "FriendRequests", "FriendDialogs", "FriendsSubscription"],
    ),
    "引导": (
        ["OnboardingView", "OnboardingGate", "OnboardingModel", "OnboardingAppsStep", "OnboardingModelStepView",
         "OnboardingProfileStep", "OnboardingReviewStep", "OnboardingTexts", "OnboardingHints",
         "OnboardingPreviews"],
        ["OnboardingScreen", "OnboardingAppsStep", "OnboardingModelStep", "OnboardingProfileStep",
         "OnboardingReviewStep", "OnboardingGuidance", "OnboardingProblems", "OnboardingContext",
         "OnboardingGatewayScreen"],
    ),
    "资料、密钥": (
        ["OwnerProfileView", "OwnerProfileForm", "OwnerSecretsDetail", "SecretDetail", "SecretInputs"],
        ["OwnerProfileScreen", "OwnerProfileForm", "OwnerSecretsDetail", "SecretDetail", "SecretInputs"],
    ),
    "核心升级": (
        ["CoreUpgradeView", "CoreUpgradeCompatPreview"],
        ["CoreUpgradeScreen", "CoreUpgradeCompat", "CoreUpgradeTexts"],
    ),
    "应用首页": (
        ["HomeView", "HomeModel", "HomeStatusPanel"],
        ["HomeScreen", "LauncherScreen", "HomeStatusPanel"],
    ),
    "聊天列表": (
        ["ChatListView", "ChatListRows"],
        ["ChatScreen", "ChatNavigation"],
    ),
    "输入框": (
        ["ComposerView", "ComposerField", "ComposerModel", "ComposerTray", "ComposerSendSlot", "ComposerRejection",
         "CaptionSendBar", "PasteFilesPreview", "AttachSheet"],
        ["Composer", "ComposerTray", "ComposerCommands", "ComposerEdit", "ComposerPaste", "ComposerKeyboard",
         "ComposerUpdates", "AttachSheet", "PhotoSend"],
    ),
    "插件设置（表单）": (
        ["PluginSettingsScreen", "PluginSettingFieldView", "PluginSettingValues"],
        ["PluginSettingsScreen", "PluginSettingFields", "PluginSettingValues"],
    ),
    "设备验证": (
        ["DeviceVerificationView", "DeviceChoicesSection", "PermissionConfirmation"],
        ["DeviceVerificationScreen", "DeviceVerificationDialogs", "DeviceChoicesSection"],
    ),
    "我的应用": (
        ["MyAppsSettingsView", "MyAppChoiceSheet", "SharedAppsSection"],
        ["MyAppsSettingsScreen", "MyAppChoiceSheet", "SharedAppsSection", "MyAppMenu", "MyAppsSyncHint"],
    ),
    "分享收进": (
        ["ShareInSheet", "ShareInModel"],
        ["ShareSheet", "ShareSheetParts", "ShareIntake", "ShareExpiryPicker", "ShareProblems"],
    ),
    "外观": (
        ["AppearanceView", "AppearanceModel"],
        ["AppearanceScreen"],
    ),
    "聊天备份": (
        ["ChatBackupView", "BackupReminderDismissals"],
        ["ChatBackupScreen", "BackupReminderPrefs"],
    ),
    "设置首页": (
        ["SettingsView", "SettingsModel"],
        ["SettingsScreen", "SettingsSubpage", "SettingsToolbarButton"],
    ),
    "App 锁": (
        ["AppLockSettingsView", "AppLock"],
        ["AppLockSettingsScreen", "AppLock", "AppLockUpdates"],
    ),
    "插件来源": (
        ["PluginSourcesView"],
        ["PluginSourcesScreen"],
    ),
    "出网代理": (
        ["EgressSettingsView"],
        ["EgressScreen"],
    ),
    "镜像源": (
        ["RegistryMirrorSettingsView"],
        ["RegistryMirrorScreen", "RegistryMirrorProbes"],
    ),
    "关于、隐私、许可": (
        ["AboutViews"],
        ["AboutPages", "LicensesLoad"],
    ),
    "通知": (
        ["NotificationSettingsView", "NotificationSettings"],
        ["NotificationSettingsScreen", "NotificationPermission"],
    ),
}


def git(*args: str) -> str:
    return subprocess.run(["git", *args], check=True, capture_output=True, text=True).stdout


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    parser.add_argument("--rev", required=True, help="只统计这个 rev 可达的历史，例如 225ea5cb")
    parser.add_argument("--since", default=None)
    args = parser.parse_args()

    root = os.path.dirname(os.path.dirname(os.path.abspath(__file__)))
    os.chdir(root)

    log_args = ["log", args.rev, "--format=@@%H", "--name-only", "--", "apps/ios", "apps/compose"]
    if args.since:
        log_args.insert(1, f"--since={args.since}")
    commits: list[set[str]] = []
    for line in git(*log_args).splitlines():
        if line.startswith("@@"):
            commits.append(set())
        elif line:
            commits[-1].add(line)

    both_commits = sum(1 for files in commits if any(f.startswith("apps/ios/") for f in files)
                       and any(f.startswith("apps/compose/") for f in files))
    total = max(len(commits), 1)
    print(f"口径一（按提交）：提交 {len(commits)} 个；两端同改的提交 {both_commits} 个"
          f"（{both_commits * 100 // total}%）")

    rows = []
    for page, (ios, android) in PAGES.items():
        ios_files = set(IOS + name + ".swift" for name in ios)
        android_files = set(ANDROID + name + ".kt" for name in android)
        ios_lines = sum(sum(1 for _ in open(path, errors="ignore")) for path in sorted(ios_files)
                        if os.path.exists(path))
        android_lines = sum(sum(1 for _ in open(path, errors="ignore")) for path in sorted(android_files)
                            if os.path.exists(path))
        pairs = sum(1 for files in commits if files & (ios_files | android_files))
        both_pairs = sum(1 for files in commits if files & ios_files and files & android_files)
        ios_commits = sum(1 for files in commits if files & ios_files)
        android_commits = sum(1 for files in commits if files & android_files)
        rows.append((pairs, both_pairs, ios_commits, android_commits, ios_lines, android_lines, page))

    print(f"口径二（按（提交，页面）对）：对 {sum(r[0] for r in rows)} 个；"
          f"两端同改的对 {sum(r[1] for r in rows)} 个")
    print()
    print("| 页面 | iOS 行 | Android 行 | iOS 提交 | Android 提交 | （提交，页面）对 | 其中两端同改 |")
    print("| --- | --- | --- | --- | --- | --- | --- |")
    for pairs, both_pairs, ios_commits, android_commits, ios_lines, android_lines, page in sorted(rows, reverse=True):
        print(f"| {page} | {ios_lines} | {android_lines} | {ios_commits} | {android_commits} | {pairs} | {both_pairs} |")
    return 0


if __name__ == "__main__":
    sys.exit(main())
