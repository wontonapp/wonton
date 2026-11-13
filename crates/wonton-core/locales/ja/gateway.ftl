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

# 上流（IDプロバイダーのエンドポイント、モデルゲートウェイ）のエラーに表示する名前。
gateway-name-oidc = IDプロバイダー
gateway-name-models = モデルゲートウェイ

# TLS入口コンテナの後ろにあるクライアント入口（ADR-0021）。
gateway-entry-header-invalid = 入口ゲートウェイがこのリクエストに正しい印を付けていません
gateway-entry-closed = この入口は開いていません。オーナーがWontonアプリで開けられます
gateway-entry-public-denied = 公開アドレスからはこのパスにアクセスできません
gateway-entry-setup-pending = まずこのアプリのウェブページで初回登録を済ませてから、もう一度お試しください
gateway-entry-auth-unsupported = このアプリはまだ、Wontonが発行したパスワードでのログインに対応していません

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
gateway-devices-invalid-addrs = リクエストボディは{"{"}"direct_addrs": ["IP:ポート"]{"}"}の形式にしてください：{ $error }
gateway-devices-already-paired = このサーバーには既にペアリング済みのデバイスがあるため、初回ペアリングコードは発行しません
gateway-devices-no-addresses = トンネルがまだリレーに接続しておらず、リクエストにも直接接続用のアドレスがないため、コードに接続できるアドレスがありません

# モデルゲートウェイ。
gateway-models-permission = アプリ{ $id }はmodels権限を申請していません
gateway-plugin-network-only = このアドレスは、プラグインネットワーク内のアプリ専用です
gateway-models-down = モデルゲートウェイが起動していません

# セットアップ。
gateway-onboarding-invalid-step = stepはprofileかfinishedだけです（{ $step }が指定されました）
gateway-onboarding-invalid-agent = primary_agentは選択できるエージェントではありません：{ $agent }
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
gateway-agent-acp-busy = このアプリの開いている Agent 接続が上限に達しました。しばらくしてからもう一度お試しください
