## 折りたたまれた技術的な詳細です。原因、エラーコード、上流の原文は変更しません。
## 長さの制限はありません。引数は診断データで、翻訳キーではありません。

# CoreErrorの説明です。$reason は元のエラーの詳細です。
core-error-storage = ローカルストレージを読み書きできません：{ $reason }
core-error-not-ready = まだ準備ができていません：{ $reason }
core-error-transport = 自宅サーバーに接続できません：{ $reason }
core-error-unauthorized = 自宅サーバーがリクエストを拒否しました：{ $reason }
core-error-forbidden = 自宅サーバーがこの操作を許可しませんでした：{ $reason }
core-error-upstream = 自宅サーバーでエラーが発生しました：{ $reason }
core-error-server-too-old = 自宅サーバーが古いため、この操作はまだできません。サーバーを更新してからもう一度お試しください。

# シェルからコアの知らないチャット操作が渡されたときに表示します。
side-chat-not-ready = このバージョンのクライアントコアはサイドチャットにまだ対応していません。

## プラグインモジュールの技術的な詳細です。エラー、アプリのタスク、プロキシ・ダウンロードミラー設定ページの「詳細」に折りたたまれて表示されます。
## 長さの制限はありません。引数は診断データで、翻訳キーではありません。
# プラグイン管理API（crates/wonton-client/src/plugins/api.rs）。
# $reason は元のエラーの詳細です。
plugins-api-invalid-proxy = ローカルプロキシのアドレスが無効です：{ $reason }

plugins-api-http-client = HTTPクライアントを作成できません：{ $reason }

# $reason はJSONパーサーの元のエラーです。
plugins-api-invalid-response = 自宅サーバーから返された内容の形式が正しくありません：{ $reason }

# $reason はJSONシリアライザーの元のエラーです。
plugins-api-invalid-request = リクエストボディを書き出せません：{ $reason }

# $reason は拒否された画像のアドレス、またはURLパーサーの元のエラーです。
plugins-api-not-image-url = アプリ画像のアドレスではありません：{ $reason }

# $status は元のHTTPステータスコードです。
plugins-api-server-status = 自宅サーバーからHTTP { $status }が返されました

plugins-api-not-connected = 自宅サーバーに接続してからもう一度お試しください。

plugins-api-server-too-old = 自宅サーバー { $version } は古すぎます。この機能には { $minimum } 以降が必要です。

# インストール・アップデート・アンインストールのタスク（crates/wonton-client/src/plugins/tasks.rs）。
plugins-tasks-uncertain = 接続が切断されたため、自宅サーバーでの結果は不明です。インストール済みリストを確認してください。

plugins-tasks-server-changed = 自宅サーバーが変わったため、以前のサーバーでの結果は不明です。

plugins-tasks-preview-no-source = このプレビューには提供元がないため、インストールできません。

# $id はそのままのタスクIDです。
plugins-tasks-no-such-task = そのようなタスクはありません：{ $id }

plugins-tasks-task-not-done = タスクがまだ完了していません。

# $title はアプリのタイトルです。ストアの表記のまま表示されます。
plugins-tasks-task-running = { $title }はすでにインストール・アップデート・アンインストールの途中です。

# プロキシ設定（crates/wonton-client/src/plugins/egress.rs）。
plugins-egress-save-in-progress = 前回の変更がまだ保存中です。

plugins-egress-not-loaded = プロキシの設定がまだ取得できていません。

plugins-egress-no-confirmation = 確認待ちの変更はありません。

plugins-egress-env-locked = プロキシはWONTON_EGRESS_PROXYで指定されているため、ここでは変更できません。

plugins-egress-plugin-and-address = プロキシアプリを使う場合、プロキシのアドレスは同時に入力できません。

plugins-egress-plugin-not-installed = プロキシアプリがまだインストールされていません。

# $reason は接続問題の元の詳細です。
plugins-egress-save-uncertain = 変更が保存できたかはっきりしません。接続後に確認してください：{ $reason }

# ダウンロードミラーの設定（crates/wonton-client/src/plugins/registry_mirror.rs）。
plugins-registry-save-in-progress = 前回のダウンロードミラーの変更がまだ保存中です。

plugins-registry-not-loaded = ダウンロードミラーの設定がまだ取得できていません。

# $reason は接続問題の元の詳細です。
plugins-registry-save-uncertain = 変更が保存できたかはっきりしません。接続後に設定をもう一度読み込んでください：{ $reason }

matrix-login-not-current = このログインは終了したか、新しいログインに置き換えられました。

matrix-login-wrong-step = このログインはその段階にありません。

matrix-login-method-not-offered = このサーバーはそのログイン方法に対応していません。

matrix-login-device-name-required = このデバイスの名前を入力してください。

matrix-login-invalid-user-id = @name:matrix.org のような完全な Matrix ID を入力してください。

matrix-login-password-required = パスワードを入力してください。

matrix-login-redirect-not-allowed = アプリはそのアドレスへのログインコールバックを受け取れません。

# 自宅サーバー上のアプリの起動（crates/wonton-client/src/plugins/start.rs）。
# $id はそのままのアプリIDです。
plugins-start-busy = { $id }はすでに起動、インストール、更新、またはアンインストールの実行中です

# 自宅サーバーのワンクリックアップデート（crates/wonton-client/src/plugins/core_upgrade.rs）：不確定な段階の説明と拒否された操作で、アップデートページやエラーの「詳細」に表示されます。
# 引数なし。自宅サーバーがこのアプリでは理解できない結果を返しました。
plugins-core-upgrade-unrecognized = 自宅サーバーが、このバージョンのWontonでは理解できない結果を返しました。Wontonを更新してからもう一度確認してください

# 実行中のダイジェストが欠けている場合の代わりの語。文ではありません。
plugins-core-upgrade-unknown-digest = 不明

# 自宅サーバーはアップデート完了を報告していますが、実行中のバージョンが目標と一致しません。$running は実行中のダイジェストか「不明」、$target はそのままの目標ダイジェストです。
plugins-core-upgrade-completed-mismatch = 自宅サーバーはアップデートが完了したと報告していますが、実行中のバージョンが目標と異なります：実行中 { $running }、目標 { $target }

# 制限時間内に接続できず、結果は不明です。$digest は旧バージョンに戻すときに固定するそのままのダイジェストです。
plugins-core-upgrade-timed-out = 制限時間を過ぎても自宅サーバーに接続できず、アップデートの結果は不明です。接続するとここが更新されます

plugins-core-upgrade-timed-out-rollback = 制限時間を過ぎても自宅サーバーに接続できず、アップデートの結果は不明です。接続するとここが更新されます。旧バージョンに戻すには、ホストでインストールコマンドをもう一度実行し、WONTON_CORE_PIN={ $digest } を付けてください

plugins-core-upgrade-invalid-action = 自宅サーバーのアップデートの操作ではありません

plugins-core-upgrade-already-running = すでにアップデートを実行中です

plugins-core-upgrade-not-cancellable = 今はキャンセルできません

# $reason は接続問題の元の詳細です。
plugins-core-upgrade-start-uncertain = アップデートを開始できたかはっきりしません。接続後にもう一度確認してください：{ $reason }

# アップデート前の互換性チェック（ADR-0085）の詳細：理由ごとに 1 行、{ $raw } はマニフェ
# ストの原文の値。クライアントコアがアプリの言語で書き、アップデートページでは新しいバー
# ジョンが読めないアプリの下に並べます。
plugins-core-upgrade-compat-spec = このアプリは仕様{ $raw }で書かれており、新しいバージョンは対応していません。アップデート後は異常として表示されます。

plugins-core-upgrade-compat-feature = このアプリのマニフェストには、新しいバージョンが認識しない{ $raw }があります。アップデート後は異常として表示されます。

plugins-core-upgrade-compat-capability = このアプリは能力{ $raw }に依存していますが、新しいバージョンでは提供されません。アップデート後は異常として表示されます。

plugins-core-upgrade-compat-other = 新しいバージョンはこのアプリを読めません。アップデート後は異常として表示されます。

plugins-core-upgrade-compat-other-raw = 新しいバージョンはこのアプリの{ $raw }を読めません。アップデート後は異常として表示されます。

## クロスオリジン分離が必要なプラグインのページはシステムのブラウザで開きます（ADR-0033）。後の3つはブラウザに表示されます。
browser-front-failed = ブラウザ用のローカルアドレスを開けませんでした：{ $error }

browser-page-not-isolated = このページはブラウザでは開きません。

browser-not-paired = Wontonはまだ自宅のサーバーとペアリングされていません。

browser-unknown-address = このアドレスは期限切れです。Wontonに戻って、アプリをもう一度開いてください。

browser-not-connected = Wontonは今、自宅のサーバーにつながっていません。Wontonに戻り、つながってからこのページを再読み込みしてください。

browser-bad-request = このページはWontonから開いてください。
