## ゲートウェイと自宅サーバーが出すWebページが、ブラウザー・App・ボットに送るテキスト（ADR-0043）。
## 完全な文を 1 メッセージにし、コードでの組み立てはしない。データ（ホスト名、ID、状態、
## 上流から返された理由）はパラメーターとして文に埋め込み、翻訳しない。説明はです・ます調。

# ゲートウェイAPI・MCPバス・ツールポリシーで共用。
gateway-mcp-disabled = 自宅サーバーでMCPバスが有効になっていません

# ログインページ。
gateway-login-title = ログイン
gateway-login-heading = Wontonにログイン
gateway-login-token-hint = 自宅サーバーのデータディレクトリーにある <code>core/owner-token</code> のトークンでログインしてください。スマホアプリのペアリング後は、コードのスキャンでログインできます。
gateway-login-failed = トークンが正しくありません。
gateway-login-submit = ログイン

# 状態ページ。
gateway-status-title = 状態
gateway-status-logged-in = ログイン中：{ $user }
gateway-status-apps-heading = アプリ
gateway-status-empty = アプリはまだインストールされていません。
gateway-status-logout = ログアウト

# 状態ページに表示するアプリの状態。
gateway-state-installing = インストール中
gateway-state-running = 実行中
gateway-state-stopped = 停止中
gateway-state-degraded = 異常：{ $reason }
gateway-state-uninstalling = アンインストール中
gateway-state-updating = アップデート中

# エラーページ。
gateway-page-error-title = エラーが発生しました
gateway-page-error-detail = 詳細
gateway-not-found = このページはありません
gateway-cross-site-write = 自宅サーバーは、自分のページからの変更リクエストだけを受け付けます

# Hostによる振り分けとアプリのサブドメインでのエラー。
gateway-unknown-host = 不明なアドレス{ $host }
gateway-oidc-unconfigured = 自宅サーバーにIDプロバイダーが設定されていません
gateway-no-app-for-host = { $host }に対応するアプリがインストールされていません
gateway-no-webui = このアプリにはWebページがありません
gateway-app-not-ready = アプリ{ $id }はまだ準備ができていません
gateway-plugin-not-shared = アプリ{ $id }は家族に共有されていません
gateway-proxy-sso-failed = アプリ{ $id }の代行ログイン資格情報を取得できません：{ $error }
gateway-cross-site-login = ゲートウェイは、他のサイトからのリクエストに付いたログイン状態を信頼しません。アプリ自身のページから操作してください
gateway-login-required = まずWontonにログインしてください
gateway-bot-web-permission = プラグインネットワーク内のアプリはBotのWebページを開いたり、ブラウザーのログインを利用したりできません
gateway-bot-web-storage = BotのWebページの記録を読み取れません。サーバーのストレージを確認してから、もう一度お試しください

# 上流（IDプロバイダーのエンドポイント、モデルゲートウェイ）のエラーに表示する名前。
gateway-name-oidc = IDプロバイダー
gateway-name-models = モデルゲートウェイ

# TLS入口コンテナの後ろにあるクライアント入口（ADR-0021）。
gateway-entry-header-invalid = 入口ゲートウェイがこのリクエストに正しい印を付けていません
gateway-entry-closed = この入口は開いていません。オーナーがWontonアプリで開けられます
gateway-public-access-off = 外部からのアクセスはオフです。オーナーがWontonアプリでオンにできます
gateway-entry-public-denied = 公開アドレスからはこのパスにアクセスできません
gateway-entry-setup-pending = まずこのアプリのウェブページで初回登録を済ませてから、もう一度お試しください
gateway-entry-auth-required = Wontonで発行したこのアプリのパスワードでサインインしてください
gateway-entry-auth-revoked = このパスワードは取り消されました。Wontonで新しく発行してください
gateway-entry-state-unavailable = このアプリの入口設定を読み取れません。オーナーがWontonで確認してください
gateway-entry-setup-closed = アプリ{ $id }に管理者アカウントがなくなりました（データが初期化または復元された可能性があります）。第三者に登録されるのを防ぐため、入口を自動的に閉じました。アプリの準備ができたらWontonアプリで再度開いてください。
gateway-entry-setup-timeout = アプリの初回登録状態の確認がタイムアウトしました
gateway-entry-target-unavailable = アプリの入口は、専用ネットワーク内でアプリ全体が共有する一つのサービスを指定してください

# SSOの引き渡しとOIDCログイン、承認確認。
gateway-sso-invalid-next = リダイレクト先のアドレスが不正です
gateway-sso-local-only = リダイレクトできるのは、このサーバー上のアプリのアドレスだけです
gateway-sso-missing-code = ハンドオフコードがありません
gateway-sso-code-invalid = ハンドオフコードが無効か期限切れです。ページを開き直してください
gateway-sso-user-mismatch = IDプロバイダーが覚えているのは別のユーザーです
gateway-proxy-sso-member-missing = あなたのアカウントはもうホームサーバーにありません。オーナーに招待し直してもらってください
gateway-sso-consent-mismatch = 承認リクエストが、現在ログイン中のユーザーと一致しません

# 上流への転送。
gateway-upstream-unreachable = { $name }に一時的にアクセスできません：{ $error }
gateway-upstream-timeout = { $name }からの応答がありません。しばらくしてからもう一度お試しください
gateway-invalid-path = リクエストのパスが不正です

# プラグイン管理API。
gateway-invalid-body = リクエストボディが不正です：{ $error }
gateway-invalid-package = プラグインパッケージではありません：tar・tar.gz・zip・composeファイルのいずれかが必要です
gateway-denied = このデバイスはペアリングされていないか、削除されています。再ペアリングしてください。コマンドラインではAuthorization: Bearer <トークン> を付けてください
gateway-cross-site-api = アプリ管理APIは他のWebページからのリクエストを受け付けません
gateway-disk-space = ディスク容量を確認できません：{ $error }
gateway-invalid-lan-choice = リクエストボディはローカルネットワーク設定の選択ではありません：{ $error }
gateway-icon-not-found = その画像はありません
gateway-icon-failed = アイコンを処理できません：{ $error }

# デバイスとペアリングコード。
gateway-devices-not-found = このデバイスは登録されていません
gateway-devices-invalid-id = リクエストボディは{"{"}"endpoint_id": "<64桁の16進数>"{"}"}の形式にしてください
gateway-devices-invalid-body = リクエストボディは{"{"}"keep_shared_passwords": true|false{"}"}か空にしてください
gateway-devices-invalid-addrs = リクエストボディは{"{"}"direct_addrs": ["IP:ポート"]{"}"}の形式にしてください：{ $error }
gateway-devices-already-paired = このサーバーには既にペアリング済みのデバイスがあるため、初回ペアリングコードは発行しません
gateway-devices-no-addresses = トンネルがまだリレーに接続しておらず、リクエストにも直接接続用のアドレスがないため、コードに接続できるアドレスがありません

# モデルゲートウェイ。
gateway-models-permission = アプリ{ $id }はmodels権限を申請していません
gateway-plugin-network-only = このアドレスは、プラグインネットワーク内のアプリ専用です
gateway-restoring = コアがまだアプリを起動しているところです。少し待ってやり直してください
gateway-models-down = モデルゲートウェイが起動していません

# セットアップ。
gateway-onboarding-invalid-step = stepはprofileかfinishedだけです（{ $step }が指定されました）
gateway-onboarding-index-missing = 公式インデックスにonboardingのリストがありません

# ワンクリックアップデート。
gateway-upgrade-disabled = この自宅サーバーでワンクリックアップデートが有効になっていません

# pusherの登録。
gateway-pushers-invalid-body = pusher登録のリクエストボディを読み取れません：{ $error }
gateway-pushers-invalid-json = pusher登録のリクエストボディはJSONではありません：{ $error }
gateway-pushers-proxy-only = pusherは自宅サーバーのプッシュプロキシ{ $url }だけを指せます
gateway-pushers-kind-refused = { $kind }のpusherは許可されていません
gateway-pushers-missing-kind = pusher登録のリクエストボディにkindがありません

# MCPバスとツールポリシー（オーナーの言語で書きます。エージェントとボットが読みます）。
gateway-tools-permission = アプリ{ $id }はtools権限を申請していません
gateway-agent-post-only = このエンドポイントはPOSTだけを受け付けます
gateway-body-unreadable = リクエストボディを読み取れません：{ $error }
gateway-body-invalid = リクエストボディが不正です：{ $error }
gateway-bot-not-found = そのボットはありません：{ $caller }
gateway-bot-not-in-plugin = アプリ{ $id }にそのボットはありません：{ $caller }
gateway-danger-owner-bot = 危険モードは主人のボットだけに使えます。
gateway-tool-not-found = アプリ{ $id }にツール{ $tool }はありません
gateway-tool-deletes = ツール{ $tool }はデータを削除または上書きするため、毎回確認が必要で、読み取り専用にはできません
gateway-plugin-not-running = このアプリは起動していないか、ツールを宣言していません

# Appが確認カードに答えるとき。
gateway-confirm-invalid-body = リクエストボディは{"{"}"answer": "approve" | "approve_wider" | "allow_window" | "reject"{"}"}の形式にしてください：{ $error }
gateway-no-allow-window = 有効な許可期間がありません

# ペアリング（コードをスキャンしたスマホに返します。オーナーの言語で書きます）。
gateway-pair-unknown-code = このサーバーは、そのペアリングコードを発行していません
gateway-pair-code-used = このペアリングコードは既に使用済みです。新しいコードを発行してください
gateway-pair-code-expired = ペアリングコードの有効期限が切れています。新しいコードを発行してください
gateway-pair-first-used = このサーバーには既にペアリング済みのデバイスがあるため、初回ペアリングコードは再利用できません。サーバー上でsudo wonton pairを実行して新しいコードを発行してください
gateway-pair-register-failed = サーバーがデバイスを登録できませんでした：{ $error }
gateway-pair-role-unavailable = この招待のロールを利用できません。管理者に再度招待してもらってください。

# オーナーがAppでプラグインのキーを確認する。
gateway-owner-secrets-plugin-not-installed = アプリ{ $id }はインストールされていません
gateway-owner-secrets-record-unreadable = アプリ{ $id }の記録を読み取れないため、キーを一覧できません
gateway-owner-secrets-not-viewable = アプリ{ $id }は、閲覧できるキーとして{ $name }を宣言していません
gateway-owner-secrets-not-generated = キー{ $name }にはまだ値がありません。アプリを一度起動する（またはアップグレードする）と再生成されます
gateway-owner-secrets-store-failed = アプリのキーを読み取れませんでした：{ $error }
gateway-owner-secrets-not-text = キー{ $name }に保存されている値はテキストではありません
gateway-owner-secrets-audit-unavailable = 自宅サーバーに監査ログが接続されていないため、キーの値を表示できません
gateway-plugin-scope-unsupported = このコアではプラグインを使える Bot を変更できません。先にコアを更新してください。

# MCP Apps 宿主页（#35）。
mcp-app-loading = 読み込み中…
mcp-app-failed = インターフェースを読み込めません：{ $error }
mcp-app-page-failed = インターフェースを読み込めませんでした。
mcp-app-retry = 再試行

# 共有リンク経由で友人がページを開く（ADR-0051）。
gateway-share-identity = このサーバーはあなたの身元を確認できません
gateway-share-required = 所有者はこのアプリをあなたに共有していません
gateway-share-revoked = 所有者がこのアプリの共有を止めました
gateway-share-expired = 共有の期限が切れました
gateway-confirmation-gone = その確認はありません。終了して1日以上経過しています
# 静态网页的数据存储（ADR-0080 第 6 节）
gateway-web-storage-undeclared = このアプリはデータを保存しません
gateway-web-storage-cross-site = このアプリのデータを変更できるのは、このアプリ自身のページだけです
gateway-web-storage-not-ready = このアプリのデータを保存しているアプリが実行されていません。しばらくしてからもう一度お試しください

# 第三方客户端和公网访问（ADR-0021、ADR-0089）。
clients-header-missing = リクエストに X-Wonton-Clients ヘッダーがありません
clients-unauthenticated = これを変更できるのはオーナーのペアリング済み端末だけです
clients-unavailable = コアがコンテナで実行されていないため、ドメイン設定は使えません
clients-passwords-unavailable = 今はアプリのパスワードの保存先を読み取れません
clients-password-label-invalid = クライアントには改行を含まない1〜128文字の名前を付けてください
clients-password-not-found = このアプリのパスワードはもう存在しません
clients-plugin-unavailable = このアプリはインストールされていません
clients-password-shared = アプリのパスワード
clients-credentials-shared-exists = このアプリには共有パスワードがすでにあります
clients-credentials-timeout = アプリ内のコマンドがタイムアウトしました。すでに反映されている可能性があり、再試行します
clients-credentials-unavailable = アプリが実行されていないため、今はパスワードを発行できません
clients-credentials-missing-command = マニフェストに { $op } コマンドがありません
clients-credentials-failed = アプリ内のコマンドが失敗しました：{ $error }
gateway-entry-credentials-not-issued = アプリのパスワードがまだないため、入口は閉じたままです
clients-guests-not-declared = このアプリはゲストリンクを受け付けません
clients-guests-entry-closed = 先にこのアプリの LAN 入口か公開入口を開いてください
clients-guests-label = リンクに 1〜128 文字の名前を付けてください。改行は使えません
clients-guests-ttl = リンクの有効期間は最長 7 日です
clients-guests-path = 開いた先のパスは / で始まり、このアプリ内である必要があります
clients-guests-max = このアプリには有効なリンクがすでに 20 本あります。先に 1 本取り消してください
clients-guests-not-found = このリンクは取り消されたか、期限が切れています
clients-guests-unavailable = ゲストリンクを今読めません
gateway-guest-invalid = このリンクは無効か、期限が切れています
clients-domain-read-failed = ドメイン設定を読めませんでした：{ $error }
clients-public-access-save-failed = 外部アクセスのスイッチを保存できませんでした：{ $error }
# 外部アクセスのスイッチの各ゲート：できなかったことをゲートごとに書く（ADR-0090）。
clients-gate-core-not-in-container-connector = コアがコンテナで動いていないため、コネクターを起動できません
clients-gate-core-not-in-container-entry = コアがコンテナで動いていないため、入口コンテナがありません
clients-gate-connector-still-running = コネクターがまだ動いています
clients-gate-connector-not-running = コネクターが起動していません
clients-gate-public-list-still-empty = 公開証明書リストがまだ空です
clients-gate-public-list-not-cleared = 公開証明書リストが空になっていません
clients-gate-entry-not-reloaded = 入口が設定を読み直していません：{ $error }
clients-gate-connector-mismatch = コネクターのコンテナがスイッチと一致しません
clients-gate-public-list-mismatch = 公開証明書リストがスイッチと一致しません
clients-domain-terms-required = 先に Let's Encrypt の加入者契約に同意してください
clients-domain-external-certificate = この家は自前の証明書（WONTON_ENTRY_CERTIFICATE=external）を使うので、コアは発行しません
clients-domain-fixed-suffix = サーバー名は固定です。プロバイダーと認証情報だけ変えられます
clients-domain-suffix-required = ドメインを入力してください
clients-domain-suffix-invalid = 小文字の ASCII ドメインを、ワイルドカードとポートなしで入力してください
clients-domain-suffix-ip = IP アドレスではなくドメイン名を入力してください
clients-domain-credentials-required = 初回設定またはプロバイダー変更時は、そのプロバイダーの認証情報も入力してください
gateway-chains-disabled = 自宅サーバーで呼び出しの連鎖の台帳が有効になっていません

# agent_acp の転送（ADR-0061 第 5 ブロック）：接続時の拒否とフレームごとのエラーを、種別付きで利用者に返す。
gateway-agent-acp-websocket-only = この入口は WebSocket 接続のみを受け付けます
gateway-agent-acp-path = agent_acp の入口は /acp のみです
gateway-agent-acp-initiator-bot-required = 接続は X-Wonton-Bot で呼び出し元の Bot を示す必要があります
gateway-agent-acp-initiator-bot-unrecognized = {$bot} はこのアプリ自身の Bot ではありません
gateway-agent-acp-on-behalf-of = 接続は X-Wonton-On-Behalf-Of で誰の代わりに話すかを示す必要があります
gateway-agent-acp-adopter-not-listened = {$bot} はこの接続が名乗る話し手を聞きません
gateway-agent-acp-target-not-listened = {$bot} はこの人の話を聞きません
gateway-agent-acp-no-bot = {$provider} にはまだ使える Bot がありません。先に Bot を作成してください
gateway-agent-acp-multi-bot = {$provider} には複数の Bot があります。先に詳細ページで使う Bot を選んでください
gateway-agent-acp-token-missing = {$provider} にはまだ提供者トークンがありません。再インストールすると作られます
gateway-agent-acp-provider-unreachable = Agent ランタイムがこの接続を引き受けませんでした：{$detail}

gateway-install-rejected = 管理者が承認しなかったため、{$action}は行われませんでした

# 招待側のロールによるフレンド申請の管理（ADR-0088）：カードは管理者へ、承認後に招待を送る。
gateway-friend-invite-title = フレンド申請が承認待ち
gateway-friend-invite-to = { $inviter } が { $friend } をフレンドに追加したがっています
gateway-friend-invite-rejected = 管理者がこのフレンド申請を承認しませんでした
gateway-friend-invite-expired = 管理者は時間内にこのフレンド申請に答えませんでした
gateway-install-expired = 管理者が期限内に回答しなかったため、{$action}は行われませんでした
gateway-install-card-device = 渡すデバイス：{ $device }
gateway-install-card-secret = リクエストで入力されたシークレット：{ $title }
gateway-install-card-setting = リクエストで入力された設定：{ $title } = { $value }
gateway-install-changed = カードの送信後に { $id } の権限、デバイス、設定が変更されたため、アップグレードしませんでした。もう一度リクエストしてください。
clients-switch-container = コアがコンテナ内で動作していないため、入口ネットワークを利用できません
clients-switch-domain = 入口を開く前にドメインと有効な証明書を設定してください
clients-switch-lan = このホストの LAN アドレスをまだ検出していません。しばらくしてからもう一度お試しください
clients-switch-lan-selection = LAN アドレスを選べないため、LAN 入口を開けません：{ $reason }
clients-switch-lan-not-private = このホストの LAN アドレス（{ $addresses }）はプライベート IPv4 アドレスではありません。LAN 入口は 10.x、172.16–31.x、192.168.x でのみ開けます
clients-switch-lan-wide-subnet = LAN アドレスのサブネット { $subnet } がプライベートアドレスの範囲を超えているため、LAN 内の機器だけを許可できません
clients-switch-lan-unknown-subnet = LAN アドレス { $address } のサブネットが分からないため、LAN 内の機器だけを許可できません
clients-switch-lan-record-wrong = LAN 用ドメインが { $found } を指しています。DNS サービスでレコードを次のように変更してください：{ $record }
clients-switch-lan-record-missing = LAN 用ドメインのレコードがありません。DNS サービスで次を追加してください：{ $record }
clients-switch-lan-record-failed = LAN 用ドメインのレコードを確認できません。しばらくしてからもう一度お試しください。必要なレコード：{ $record }（{ $error }）
clients-switch-setup = 先にアプリのウェブページで初回登録を完了してください
clients-switch-app-unavailable = アプリが動作していないため、初回登録の状態を確認できません
clients-switch-credentials = アプリがまだパスワードを正常に発行していません
clients-switch-ingress = 公開入口の種類が設定されていません
clients-switch-kind = 入口には lan または public を指定してください
clients-switch-uninstalled = クライアントアプリはアンインストールされています
clients-switch-failed = クライアント入口の設定を適用できませんでした：{ $error }
clients-switch-domain-read = ドメインと証明書の状態を読み取れません：{ $error }
clients-switch-no-route = クライアントプラグイン { $plugin } に使える入口がありません
clients-switch-service-down = クライアントプラグイン { $plugin } のサービスが動作していません
clients-switch-setup-check = アプリの初回登録の状態を確認できません：{ $error }
clients-switch-not-found = クライアントプラグイン { $plugin } はインストールされていないか、入口を宣言していません
clients-switch-settings-read = 入口の設定を読み取れません：{ $error }
clients-switch-settings-missing = 入口の設定がないため、入口モデルを確認して適用できません
clients-switch-apply = 入口の設定を書き込みまたは再読み込みできません：{ $error }
clients-switch-record-read = { $plugin } のクライアント入口の記録を読み取れません：{ $error }
clients-switch-no-label = クライアントプラグイン { $plugin } に入口ラベルがありません
clients-switch-label-conflict = 入口ラベル { $label } が重複しています
clients-switch-records-read = プラグインの記録を読み取れません：{ $error }
clients-switch-rollback-entry = 入口設定のロールバックにも失敗しました：{ $error }
clients-switch-rollback-switch = スイッチのロールバックにも失敗しました：{ $error }
# 外のサーバーの人からのフレンド申請：公開ページで受け付け、管理者が答え、承認した人を allowed_users に記録する。
gateway-friend-request-title = フレンド申請
gateway-friend-request-intro = このサーバーは、管理者が承認するまで他のサーバーの人を通しません。自分の Matrix ID を残してください。結果はここに出ます。
gateway-friend-request-you = 自分の Matrix ID
gateway-friend-request-to = 相手の Matrix ID
gateway-friend-request-target = { $target } を追加したい
gateway-friend-request-submit = リクエストを送る
gateway-friend-request-waiting = リクエストを送りました。管理者の回答を待っています（相手：{ $target }）。
gateway-friend-request-approved = 管理者が承認しました。もう一度招待を送ってください。
gateway-friend-request-open = 相手の役割はもう承認不要です。招待を送り直すだけです。
gateway-friend-request-forbidden = このメンバーは役割上フレンドを追加できません。管理者に招待してもらってください。
gateway-friend-request-denied = 管理者はこのフレンド申請を承認しませんでした。
gateway-friend-request-expired = 時間内に管理者が答えませんでした。あとでもう一度どうぞ。
gateway-friend-request-changed = 相手のアカウントや権限が変わりました。あとで試すか、相手から追加してもらってください。
gateway-friend-request-unavailable = このサーバーはいま答えられません。あとでどうぞ。
gateway-friend-request-duplicate = この人へのリクエストがすでに待っています。
gateway-friend-request-too-many = 待っているリクエストが多すぎます。あとでどうぞ。
gateway-friend-request-local = 同じサーバーの人は承認不要です。そのまま招待してください。
gateway-friend-request-unknown = このサーバーにその人はいません。
gateway-friend-request-invalid = @bob:example.org のような完全な Matrix ID を入れてください。
gateway-friend-request-invalid-method = このアドレスは GET と POST だけです。
gateway-friend-request-again = もう一つ送る
gateway-friend-request-card-title = 外の人がフレンドを追加したがっています
gateway-friend-request-card-to = { $target } をフレンドに追加したがっています
