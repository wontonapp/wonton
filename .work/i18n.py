import pathlib, re
L = ["en", "zh-Hans", "zh-Hant", "ja"]
A = "apps/compose/shared/src/androidMain/res"
I = "apps/ios/Wonton/strings"
T = []
T.append(("onboarding_step_review", "onboarding.step.review", None, {
 "en": "Check and install", "zh-Hans": "确认并安装", "zh-Hant": "確認並安裝", "ja": "確認してインストール"}))
T.append(("onboarding_done", "onboarding.done", None, {
 "en": "Start install", "zh-Hans": "开始安装", "zh-Hant": "開始安裝", "ja": "インストールを開始"}))
T.append(("onboarding_running", "onboarding.onboardinging", None, {
 "en": "x", "zh-Hans": "x", "zh-Hant": "x", "ja": "x"}))
T.append(("onboarding_review_done", "onboarding.review.done", None, {
 "en": "x", "zh-Hans": "x", "zh-Hant": "x", "ja": "x"}))
T.append(("onboarding_review_preparing", "onboarding.review.preparing", None, {
 "en": "x", "zh-Hans": "x", "zh-Hant": "x", "ja": "x"}))
T.append(("onboarding_review_waiting", "onboarding.review.waiting", None, {
 "en": "Waiting for the ones before it", "zh-Hans": "等前面的装完", "zh-Hant": "等前面的裝完", "ja": "前の項目の完了待ち"}))
T.append(("onboarding_review_blocked", "onboarding.review.blocked", None, {
 "en": "Can\\'t install", "zh-Hans": "装不了", "zh-Hant": "無法安裝", "ja": "インストールできません"}))
T.append(("onboarding_review_uncertain", "onboarding.review.uncertain", None, {
 "en": "Not sure it installed. Open the app\\'s page to check.", "zh-Hans": "不确定装好了没有，到应用页看看", "zh-Hant": "不確定裝好了沒有，到應用程式頁面看看", "ja": "x"}))
T.append(("onboarding_unchanged", "onboarding.unchanged", None, {
 "en": "x", "zh-Hans": "x", "zh-Hant": "x", "ja": "x"}))
T.append(("onboarding_review_unchanged", "onboarding.review.unchanged", None, {
 "en": "Nothing changes here", "zh-Hans": "这里不改", "zh-Hant": "x", "zh-Hant": "這裡不改", "ja": "ここは変更しません"}))
T.append(("onboarding_review_keep_current", "onboarding.review.keepCurrent", None, {
 "en": "Keep the current model", "zh-Hans": "继续用现在的模型", "zh-Hant": "繼續用目前的模型", "ja": "現在のモデルを使い続ける"}))
T.append(("onboarding_review_unreachable", "onboarding.review.unreachable", None, {
 "en": "x", "zh-Hans": "x", "zh-Hant": "x", "ja": "x"}))
T.append(("onboarding_review_unreachable_model", "onboarding.review.unreachableModel", None, {"en": "x", "zh-Hans": "x"}))
T.append(("onboarding_review_section_profile", "onboarding.review.sectionProfile", None, {"en": "x"}))
T.append(("onboarding_bad", "x", None, {"en": "x"}))
T.append(("onboarding_review_section_apps", "onboarding.review.sectionApps", None, {"en": "x"}))
T.append(("onboarding_review_section_model", "onboarding.review.sectionModel", None, {"en": "x"}))
T.append(("install_permissions", "install.permissions", None, {"en": "x"}))
T.append(("plugin_permission_risk_high", "plugin.permission.risk.high", None, {"en": "x"}))
T.append(("plugin_permission_risk_very_high", "plugin.permission.risk.veryHigh", None, {"en": "x"}))
T.append(("plugin_permission_risk_highest", "plugin.permission.risk.highest", None, {"en": "x"}))
T.append(("plugin_permission_risk_unknown", "plugin.permission.risk.unknown", None, {"en": "x"}))
T.append(("plugin_permission_optional", "plugin.permission.optional", None, {"en": "x"}))
T.append(("plugin_permission_added", "plugin.permission.added", None, {"en": "x"}))
T.append(("plugin_permission_unsupported", "plugin.permission.unsupported", None, {"en": "x"}))
T.append(("plugin_permission_bad", "x", None, {"en": "x"}))
T.append(("plugin_permission_understand_title", "plugin.permission.understand.title", None, {"en": "x"}))
T.append(("plugin_permission_host_root", "plugin.permission.host.root", None, {"en": "x"}))
for k in ["permission-models", "permission-tools", "permission-plugins-health", "permission-matrix-bot", "permission-notify", "permission-owner-identity", "permission-network-egress", "permission-egress-proxy"]:
    T.append((None, None, k, {"en": "x"}))
for k in ["permission-network-lan", "permission-network-clients", "permission-network-host", "permission-capability", "permission-host-pid", "permission-host-ipc", "permission-privileged"]:
    T.append((None, None, k, {"en": "x"}))
for k in ["permission-security-opt", "permission"]:
    T.append((None, None, k, {"en": "x"}))
for k in ["permission-devices-gpu", "permission-device", "permission-devices"]:
    T.append((None, None, k, {"en": "x"}))
V = {}
V["onboarding_review_intro"] = ["Finishing installs these apps with the permissions listed. You can leave while they install.", "确定后会按上面列出的权限装好这些应用，安装时你可以离开。", "確認後會照上面列出的權限裝好這些應用程式，安裝時你可以離開。", "確認すると、上に並んだ権限でこれらのアプリをインストールします。インストール中は画面を離れてかまいません。"]
V["onboarding_step_review"] = ["Check and install", "确认并安装", "確認並安裝", "確認してインストール"]
V["onboarding_done"] = ["Start install", "开始安装", "開始安裝", "インストールを開始"]
V["onboarding_running"] = ["Installing", "正在安装", "正在安裝", "インストール中"]
V["onboarding_review_done"] = ["x", "x", "x", "x"]
V["onboarding_review_done"] = ["Installed", "装好了", "裝好了", "インストール済み"]
V["onboarding_review_preparing"] = ["Checking it can be installed", "看它能不能装", "看它能不能裝", "インストールできるか確認中"]
V["onboarding_bad"] = []
V["onboarding_review_done"] = []
V["onboarding_bad"] = ["x"]
V["onboarding_review_done"] = ["Installed", "装好了", "裝好了", "インストール済み"]
V["onboarding_review_uncertain"] = ["Not sure it installed. Open the app's page to check.", "不确定装好了没有，到应用页看看", "不確定裝好了沒有，到應用程式頁面看看", "インストールできたか分かりません。アプリのページで確認してください。"]
V["onboarding_review_unchanged"] = ["Nothing changes here", "这里不改", "這裡不改", "ここは変更しません"]
V["onboarding_review_keep_current"] = ["Keep the current model", "继续用现在的模型", "繼續用目前的模型", "現在のモデルを使い続ける"]
V["onboarding"] = []
V["onboarding_review_unreachable"] = ["Couldn't reach it when tested", "试的时候没连上", "試的時候沒連上", "テスト時に接続できませんでした"]
V["onboarding_review_unreachable_model"] = ["%1$s · Couldn't reach it when tested", "%1$s · 试的时候没连上", "%1$s · 試的時候沒連上", "%1$s · テスト時に接続できませんでした"]
V["onboarding_review_section_profile"] = ["Your profile", "资料", "資料", "プロフィール"]
V["onboarding_review_section_apps"] = ["Apps to install", "要装的应用", "要裝的應用程式", "インストールするアプリ"]
V["onboarding_review_section_model"] = ["Model", "模型", "模型", "モデル"]
V["install_permissions"] = ["What it can do", "它能做什么", "它能做什麼", "できること"]
V["plugin_permission_risk_high"] = ["Wider access", "权限较大", "權限較大", "権限が広め"]
V["plugin_permission_risk_very_high"] = ["Much wider access", "权限很大", "權限很大", "権限がとても広い"]
V["plugin_permission_risk_highest"] = ["Controls the whole host", "能控制整台主机", "能控制整部主機", "ホスト全体を操作できる"]
V["plugin_permission_unsupported"] = ["x"]
V["plugin_permission_unsupported"] = ["Not supported here", "这个版本不支持", "這個版本不支援", "このバージョンでは使えません"]
V["plugin_permission_understand_title"] = ["I know it can control the whole host", "我知道它能控制整台主机", "我知道它能控制整部主機", "ホスト全体を操作できることを理解しました"]
V["plugin_permission_host_root"] = ["Once it's on, it has the highest permission on this host and can manage every app on it.", "打开后，它就有这台主机的最高权限，能管上面所有应用。", "開啟後，它就有這台主機的最高權限，能管上面所有應用程式。", "オンにすると、このホストの最高権限を持ち、上のすべてのアプリを管理できます。"]
V["plugin_permission_risk_unknown"] = ["Latest risk level, from a newer core", "x", "x", "x"]
