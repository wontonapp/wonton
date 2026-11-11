## ストア、提供元、ネットワーク設定、モデルプロバイダー、アプリのアップデートで自宅サーバーが
## App に見せる文章（ADR-0043）。完全な文を1メッセージにし、コードで文を組み立てない。
## データ（パス、ポート、イメージ名、ID、アドレス、Dockerの応答）は引数として文に埋め込み、翻訳しない。

# ── 共通 ─────────────────────────────────────────────────────────────────────

## with_note は元のメッセージと補足を1文につなげる。
plugin-settings-note-appended = { $message }（{ $note }）

# ── Webページのローカルネットワークポートのリスク説明（lan/webui.rs、詳細で表示）──

plugin-settings-lan-webui-risk-impersonate = ローカルネットワーク上のデバイスや、このホスト上でインターネットに出られるアプリが、パスワードなしであなたとしてこのページを開けます
plugin-settings-lan-webui-risk-register = 管理者が登録する前に、ローカルネットワークのデバイスやこのホスト上のアプリが先に登録してしまうことがあります
plugin-settings-lan-webui-risk-client = ローカルネットワークのデバイスは、ユーザー名とパスワードがあれば接続できます。アプリ自身のページでパスワードを変えても、ここには反映されません

# ── マニフェストのlanとlan.client（lan/mod.rs、lan/client.rs）───────────────

plugin-settings-lan-client-prefix = lan.client：{ $message }
plugin-settings-lan-client-address-host = addressには{ $host }を書いてください
plugin-settings-lan-client-template = { $name }はここでは使えません
plugin-settings-lan-need-ports = network.lanを申請した場合は、開放するポートをlan.portsで指定してください
plugin-settings-lan-need-permission = lanを宣言した場合は、network.lan権限の申請も必要です
plugin-settings-lan-no-matrix-bot = network.lanとmatrix.botは同時に申請できません。アドレスが変わるとコンテナの再構築が必要になりますが、自宅サーバーはBotのアクセストークンを保存しません
plugin-settings-lan-ports-empty = lan.portsには1項目以上必要です
plugin-settings-lan-port-zero = lan.portsのポートに0は使えません
plugin-settings-lan-port-duplicate = lan.portsの{ $port }が重複しています
plugin-settings-lan-port-mcp = mcpのポートは自宅サーバー経由でのみアクセスでき、lan.portsには入れません（{ $service }の{ $port }）
plugin-settings-lan-network-none = lan.portsのサービスにnetwork_mode: noneは指定できません

# ── lan.clientの接続情報（lan/client.rs、詳細ページに表示）────────────────────

plugin-settings-lan-client-secret-missing = このアプリのシークレットが保存されていません
plugin-settings-lan-client-container = コンテナ{ $container }を参照できません：{ $error }
plugin-settings-lan-client-not-published = ポートがどのローカルネットワークアドレスにも公開されていません

# ── ローカルネットワークの検出と選択（lan/detect.rs、select.rs、monitor.rs）────

plugin-settings-lan-fixed-by-env = ローカルネットワークのアドレスは自宅サーバーのWONTON_LAN_ADDRESSESで指定されているため、設定では変更できません
plugin-settings-lan-announce-by-env = スマートフォンが接続するアドレスは自宅サーバーのWONTON_ANNOUNCE_ADDRESSESで指定されているため、設定では変更できません
plugin-settings-lan-desktop-vm = 自宅サーバーはコンピューター上の仮想マシンで動いているため、スマートフォンから仮想マシンのアドレスに接続できません。ランチャーがWONTON_ANNOUNCE_ADDRESSESにコンピューターのローカルネットワークのアドレスを設定する必要があります。カーネル：{ $evidence }
plugin-settings-lan-announced = 通知先
plugin-settings-lan-fixed-missing = WONTON_LAN_ADDRESSESの{ $ip }がホスト上にありません
plugin-settings-lan-detect-failed = ホストのネットワークインターフェースを検出できません：{ $failure }
plugin-settings-lan-detect-error = ホストのネットワークインターフェースの検出中にエラーが発生しました：{ $error }
plugin-settings-lan-detect-core-image = 自宅サーバーのコンテナ{ $container }のイメージが見つかりません
plugin-settings-lan-detect-exit = 検出コンテナが終了コード{ $code }で終了しました：{ $reason }
plugin-settings-lan-detect-output = 検出コンテナの出力を解釈できません：{ $error }
plugin-settings-lan-detect-timeout = コンテナ{ $container }が{ $seconds }秒以内に終わりませんでした
plugin-settings-lan-detect-no-exit-code = Dockerがコンテナ{ $container }の終了コードを返しませんでした
plugin-settings-lan-listen-error = ポートの使用状況の確認中にエラーが発生しました：{ $error }
plugin-settings-lan-choice-unknown = 保存されているローカルネットワークの設定はこのバージョンの自宅サーバーでは認識できません。選び直してください
plugin-settings-lan-no-default-route = ホストにIPv4のデフォルトルートがありません
plugin-settings-lan-default-virtual = デフォルトルートは{ $name }（{ $why }）を使っているため、自動では選択しません
plugin-settings-lan-default-no-private = デフォルトルートのインターフェース{ $name }にプライベートIPv4アドレスがありません
plugin-settings-lan-interface-gone = 選択したインターフェース{ $name }がホストに存在しないか、プライベートアドレスがありません
plugin-settings-lan-interface-no-private = 選択したインターフェース{ $name }にプライベートIPv4アドレスがありません
plugin-settings-lan-address-gone = 選択したアドレス{ $address }がホスト上にありません
plugin-settings-lan-no-selection = インターフェースもアドレスも選択されていません
plugin-settings-lan-choice-unrecognized = この選び方は認識できません
plugin-settings-lan-interface-not-selectable = { $name }はホストで選べるインターフェースではありません
plugin-settings-lan-address-not-selectable = { $address }はホストで選べるアドレスではありません
plugin-settings-lan-kind-tunnel = トンネルインターフェース（Tailscaleなど）
plugin-settings-lan-kind-wireguard = WireGuardインターフェース
plugin-settings-lan-kind-interface = インターフェース
plugin-settings-lan-bridge-or-veth = Dockerのブリッジまたはveth
plugin-settings-lan-bound = { $interface }の{ $ip }
plugin-settings-lan-select-failed = ローカルネットワークのアドレスを選べません：{ $reason }
plugin-settings-lan-lookup-failed = ローカルネットワークポートの公開先を特定できません：{ $error }
plugin-settings-lan-rebuild-failed = 新しいローカルネットワークアドレスへの再構築に失敗しました：{ $error }
plugin-settings-lan-check-failed = アプリの確認に失敗しました：{ $error }

# ── インストールプレビューのローカルネットワークポート（lan/grant.rs）──────────

plugin-settings-lan-port-line = ローカルネットワークポート：{ $service }の{ $key }、{ $place }で公開
plugin-settings-lan-port-line-plain = ローカルネットワークポート：{ $service }の{ $key }
plugin-settings-lan-direct-notice = ローカルネットワークのデバイスはこれらのポートに直接接続します。自宅サーバーへのサインインは不要で、身元確認はアプリ自身が行います
plugin-settings-lan-conflict-taken = ローカルネットワークポート{ $key }はアプリ{ $id }が既に使っています
plugin-settings-lan-note-reselect = ローカルネットワークの設定で別のインターフェースやアドレスを選べます

# ── Webページのローカルネットワークポートのスイッチ（lan/webui.rs）────────────

plugin-settings-lan-webui-no-switch = アプリ{ $id }にはlan_webuiスイッチがありません。ローカルネットワークポートにスイッチは不要です
plugin-settings-lan-webui-busy = アプリ{ $id }はインストール・アンインストール・再構築のいずれかを実行中です。終わってから再度行ってください
plugin-settings-lan-webui-close-failed = Webページのローカルネットワークポートを閉じるときに失敗しました：{ $error }
plugin-settings-lan-webui-rollback-failed = スイッチ変更に失敗した後の巻き戻しも失敗しました：{ $error }

# ── インストールエラーの文（install_error.rs 各バリアントの構築点）────────────

plugin-settings-not-installed = アプリ{ $id }はインストールされていません
plugin-settings-permission-not-granted = インストールには次の権限の確認が必要です：{ $permissions }
plugin-settings-core-upgrading = 自宅サーバーはアップデート中です。終わってからもう一度お試しください
plugin-settings-bot-matrix-unavailable = 自宅サーバーがMatrixに接続していないため（WONTON_MATRIX_URL）、matrix.botを宣言するアプリはインストールできません
plugin-settings-note-revoke-device-failed = インストール用デバイスの失効に失敗しました：{ $error }
plugin-settings-note-migration-containers-gone = ネットワークの移行中に古いコンテナは既に削除されています。この再構築をやり直すか、アプリを（データを残して）再インストールしてください
plugin-settings-note-registry-mirror = 設定でダウンロードミラーを有効にできます
plugin-settings-upgrade-rolled-back = { $new }へのアップデートに失敗し、{ $version }に戻しました
plugin-settings-upgrade-rollback-failed = { $new }へのアップデートに失敗し、{ $version }への巻き戻しも失敗しました：{ $error }
plugin-settings-upgrade-cleanup-unfinished = 後片付けが完了していません。残骸が残っている可能性があります：{ $failures }
plugin-settings-upgrade-cleanup-model-credentials = モデルゲートウェイのクレデンシャル：{ $error }

# ── プロキシ（egress/）──────────────────────────────────────────────────────

plugin-settings-egress-plugin-and-url = プロキシアプリを使うときは、プロキシアドレスは同時に設定できません
plugin-settings-egress-restarts-changed = 再起動するアプリが確認時と変わっています。もう一度確認してください
plugin-settings-egress-cidr-invalid = ネットワーク範囲{ $cidr }が不正です：{ $error }
plugin-settings-egress-cidr-overlaps-network = ネットワーク範囲{ $cidr }がDockerネットワーク{ $network }（{ $subnet }）と重なっています。許可するとプロキシコンテナが他のアプリに直接接続してしまいます
plugin-settings-egress-env-locked = プロキシアドレスはWONTON_EGRESS_PROXYで指定されているため、ここでは変更できません。インストールディレクトリの設定を変えてから、インストールスクリプトをもう一度実行してください
plugin-settings-egress-address-invalid = プロキシアドレスが不正です：{ $error }
plugin-settings-egress-address-scheme = プロキシの方式{ $scheme }には対応していません。http、https、socks5のいずれかを使ってください
plugin-settings-egress-address-no-host = プロキシアドレスにホスト名がありません
plugin-settings-egress-address-no-path = プロキシアドレスは方式・ホスト・ポートのみを書きます。パスやパラメータは付けません
plugin-settings-egress-address-utf8 = プロキシアドレスのユーザー名またはパスワードが正しいUTF-8ではありません：{ $error }
plugin-settings-egress-no-response = プロキシ{ $address }が{ $seconds }秒応答しません：ホストのファイアウォールでDockerのネットワーク範囲からこのポートへの通信を許可してください
plugin-settings-egress-refused = プロキシ{ $address }に接続を拒否されました：ポートで待ち受けていないか、ループバックアドレスのみで待ち受けています
plugin-settings-egress-unreachable = プロキシ{ $address }に接続できません：{ $error }
plugin-settings-egress-upstream-failed = プロキシ自体は動いていますが、外に接続できません：{ $error }
plugin-settings-egress-rejected = プロキシでこの段階が通過できませんでした：{ $error }
plugin-settings-egress-disconnected = プロキシ{ $address }が接続を切断しました：{ $error }
plugin-settings-egress-not-http = プロキシ{ $address }の応答がHTTPではありません
plugin-settings-egress-auth-407 = プロキシ{ $address }が407を返しました：ユーザー名とパスワードが必要か、誤っています
plugin-settings-egress-status = プロキシ{ $address }が{ $code }を返しました
plugin-settings-egress-relay-not-running = プロキシコンテナが動いていません
plugin-settings-egress-relay-detail = プロキシには直接接続できますが、プロキシコンテナ経由では接続できません：{ $detail }。コンテナ{ $container }のログを確認してください
plugin-settings-egress-diagnose-exit = 原因を調べるコンテナが終了コード{ $code }で終了しました：{ $reason }
plugin-settings-egress-provider-missing = プロキシとして動作できるアプリがインストールされていません
plugin-settings-egress-provider-manifest = プロキシアプリ{ $id }の宣言を読み取れません
plugin-settings-egress-provider-conflict = プロキシアプリ{ $id }が既にインストールされています。同時に1つだけです
plugin-settings-egress-provider-not-running = プロキシアプリが動いていません：{ $detail }
plugin-settings-egress-relay-cidr-overlap = ネットワーク範囲{ $cidr }がプロキシコンテナの接続先Dockerネットワークと重なっているため、許可しませんでした。許可すると他のアプリに直接接続してしまいます
plugin-settings-egress-token-unreadable = サービス{ $service }がMatrixトークンをcommandやconfigsに書いているため、読み戻せません
plugin-settings-egress-token-reinstall = アプリを（データを残して）再インストールすると新しいデバイスが発行され、ボットは暗号化IDを再構築する必要があります
plugin-settings-egress-token-read-back = ボットのトークンを読み戻せません：{ $reason }。{ $reinstall }

plugin-settings-egress-bot-record-missing = このアプリのボット記録がありません
plugin-settings-egress-old-container-gone = 古いコンテナ{ $container }がありません
plugin-settings-egress-token-not-in-env = 古いコンテナ{ $container }の環境変数{ $variable }にトークンがありません
plugin-settings-egress-token-mismatch = 古いコンテナ間でトークンが一致しません
plugin-settings-egress-token-not-used = 旧バージョンではトークンを使うサービスがありません

# ── ダウンロードミラー（registry_mirror/）────────────────────────────────────

plugin-settings-mirror-address-invalid = ダウンロードミラーのアドレス{ $address }が不正です。ホスト名と任意のポートのみを書き、方式・パス・認証情報は付けません
plugin-settings-mirror-policy-never = イメージ{ $image }はpull_policy: neverですが、ローカルにありません
plugin-settings-mirror-all-failed = イメージ{ $image }はどのミラーからも取得できませんでした。{ $tries }。設定でテストして、別のミラーを選んでください
plugin-settings-mirror-try-failed = { $mirror }：{ $error }
plugin-settings-mirror-digest-mismatch = ローカルイメージのRepoDigestsに必要なダイジェスト{ $digest }がありません。使用を拒否します
plugin-settings-mirror-probe-timeout = テスト全体がタイムアウトしました（{ $ms }ミリ秒）：{ $error }
plugin-settings-mirror-no-challenge = 有効なBearer WWW-Authenticateチャレンジがありません
plugin-settings-mirror-token-request-rejected = 匿名トークンのリクエストが拒否されました
plugin-settings-mirror-token-response-invalid = 匿名トークンの応答が不正です：{ $error }
plugin-settings-mirror-token-missing = 匿名トークンの応答にtoken/access_tokenがありません
plugin-settings-mirror-manifest-rejected = マニフェストのリクエストが拒否されました
plugin-settings-mirror-manifest-digest = マニフェストのダイジェストが一致しません：期待{ $digest }、実際{ $actual }
plugin-settings-mirror-status = HTTP { $status }：{ $detail }

# ── ストアと提供元（storefront/、store.rs、store_info.rs）────────────────────

plugin-settings-store-source-unreadable = 追加した提供元を読み込めません：{ $why }
plugin-settings-store-source-not-added = { $address }は追加されていません
plugin-settings-store-source-limit = 提供元は最大{ $max }件です。使わないものを先に削除してください
plugin-settings-store-source-save-failed = 追加した提供元を保存できません：{ $error }
plugin-settings-store-source-in-official = { $repo }は既に公式ストアにあります
plugin-settings-store-source-neither = { $repo }のルートに{ $compose }も{ $index }もありません。アプリでもインデックスでもありません
plugin-settings-store-source-already-added = { $address }は既に追加されています
plugin-settings-store-source-index-missing = { $provider }にインデックスファイルがありません
plugin-settings-store-source-not-an-index = { $address }はインデックスではありません：{ $error }
plugin-settings-store-source-plugin-missing = インデックス{ $provider }に{ $id }がありません
plugin-settings-store-community-partial = 読めなかったものがあります：{ $missing }
plugin-settings-store-community-page-failed = 検索結果の{ $page }ページ目（{ $error }）
plugin-settings-store-community-unavailable = コミュニティのアプリを取得できません：{ $error }
plugin-settings-store-system-app = システムアプリは自宅サーバーに同梱されているものだけです
plugin-settings-store-permission-unsupported = この自宅サーバーは権限{ $permission }にまだ対応していません
plugin-settings-store-index-newer = インデックスの形式は{ $format }です。このバージョンの自宅サーバーが読めるのは{ $supported }までです。新しいインデックスを使うには自宅サーバーをアップデートしてください
plugin-settings-store-index-invalid = インデックスを読み込めません：{ $error }
plugin-settings-store-index-compose-digest = composeとfilesの{ $file }のsha256が一致しません
plugin-settings-store-index-version-mismatch = version{ $version }がマニフェストのバージョン{ $manifest }と一致しません
plugin-settings-store-index-source-duplicate = sourceが重複しています
plugin-settings-store-index-id-duplicate = アプリID{ $id }が重複しています
plugin-settings-store-index-source-with-version = sourceにバージョンは付けられません
plugin-settings-store-index-commit-length = commitは40桁のコミットハッシュである必要があります
plugin-settings-store-index-file-digest = { $repo }の{ $path }がインデックスに記録されたsha256と一致しません
plugin-settings-store-index-file-missing = インデックスに{ $source }の{ $path }がありません
plugin-settings-store-index-blob-invalid-sha = 公式インデックスのsha256が不正です：{ $sha256 }
plugin-settings-store-index-blob-missing = インデックスディレクトリに{ $path }がありません：{ $error }
plugin-settings-store-record-id-mismatch = 記録のIDは{ $record }で、ファイル名{ $file }と一致しません

# ── ストアのマニフェスト情報（store_info.rs、公式アプリのインストール時に表示）──

plugin-settings-store-info-spec-missing = マニフェストにはspecが必要です。例：spec: "{ $version }"
plugin-settings-store-info-spec-string = specは文字列である必要があります。例："{ $version }"（YAMLでは引用符で囲まないと数値として読まれます）
plugin-settings-store-info-spec-version = specの{ $text }は仕様バージョンではありません（例："{ $version }"）
plugin-settings-store-info-author = author.nameは空にできず、最大{ $max }文字です
plugin-settings-store-info-license = licenseの{ $license }はSPDXライセンス式ではありません（例：MIT、AGPL-3.0-only）：{ $error }
plugin-settings-store-info-screenshots-count = screenshotsには1〜{ $max }枚必要です
plugin-settings-store-info-screenshot-format = スクリーンショット{ $raw }はPNGまたはJPEGである必要があります
plugin-settings-store-info-screenshot-duplicate = スクリーンショット{ $raw }が重複しています
plugin-settings-store-info-tags-count = tagsは最大{ $max }個です
plugin-settings-store-info-tag-length = タグ{ $tag }は1〜{ $max }文字で、前後に空白を付けません
plugin-settings-store-info-tag-control = タグ{ $tag }に制御文字が含まれています
plugin-settings-store-info-tag-duplicate = タグ{ $tag }が重複しています
plugin-settings-store-info-field-length = { $field }.{ $lang }が{ $max }文字を超えています
plugin-settings-store-info-https-url = { $field }の{ $value }はhttps://で始まるURLである必要があります

# ── アップデート確認（upgrade/）───────────────────────────────────────────

plugin-settings-upgrade-none = アプリ{ $id }の新バージョンはありません
plugin-settings-upgrade-no-origin = アプリ{ $id }には提供元がなく、新バージョンを確認できません
plugin-settings-upgrade-id-changed = アプリのパッケージは{ $package }ですが、アップデート対象は{ $record }です。アップデートでidは変えられません
plugin-settings-upgrade-same-version = インストール済みの{ $version }と同じため、入れ替えは不要です
plugin-settings-upgrade-changed = { $id }の新バージョンがプレビュー後に変わりました（パッケージのダイジェストは現在{ $digest }）。プレビューし直して、もう一度確認してください
plugin-settings-upgrade-older = インストール済みの{ $version }より古いバージョンです。アプリのデータは新バージョン向けに移行済みの可能性があり、旧バージョンでは読めないことがあります。先にバックアップしてください
plugin-settings-upgrade-source-changed = 提供元が{ $old }から{ $new }に変わりました。以後、Wontonは元の提供元ではアップデートを通知しません
plugin-settings-upgrade-origin-upload = あなたがアップロードしたファイル
plugin-settings-upgrade-origin-index = { $via }のインデックス
plugin-settings-upgrade-index-unavailable = 公式インデックスを取得できず、新バージョンを確認できません：{ $details }
plugin-settings-upgrade-system-app = システムアプリは自宅サーバーと一緒にアップデートされ、ここからはアップデートできません
plugin-settings-upgrade-lan-ports-closing = ローカルネットワークのポート{ $ports }はアップデート後に閉じます。開いたままにするには、下のスイッチをオンにしてください
plugin-settings-upgrade-check-failed = { $id }の新バージョンが見つかりませんでした：{ $error }
plugin-settings-upgrade-official-commit-moved = 公式インデックスのバージョン{ $version }が別のコミットに変わりました。新バージョンとしては扱わず、インストール済みのものは変わりません
plugin-settings-upgrade-tag-moved = このバージョンのtag { $tag }が作者によって別のコミットに移されました。新バージョンとしては扱わず、インストール済みのものは変わりません
plugin-settings-upgrade-index-removed = インデックス{ $provider }はストアから削除され、新バージョンはもう通知されません
plugin-settings-upgrade-compose-not-utf8 = { $file }はUTF-8テキストではありません
plugin-settings-upgrade-manifest-id-changed = マニフェストのidが{ $id }に変わりました
plugin-settings-upgrade-manifest-version = マニフェストのバージョンは{ $version }です
plugin-settings-upgrade-bundled-failed = サーバーに同梱の{ $version }をアップデートできず、引き続き{ $from }が動いています。次回の起動時に再試行します：{ $error }

# ── GitHubとのやり取り（github.rs、github_source.rs）──────────────────────

plugin-settings-github-tags-what = { $repo }のtagを読み取る
plugin-settings-github-no-repo = GitHubに公開リポジトリ{ $repo }はありません
plugin-settings-github-status = { $what }とき、GitHubが{ $status }を返しました
plugin-settings-github-tags-parse = GitHubが返した{ $repo }のtagを読み取れません：{ $error }
plugin-settings-github-file-what = コミット{ $commit }の{ $repo }から{ $path }を取得する
plugin-settings-github-missing = { $what }が見つかりません
plugin-settings-github-url-what = { $url }を取得する
plugin-settings-github-api-what = API経由で{ $reference }の{ $repo }から{ $path }を取得する
plugin-settings-github-rate-limited = { $what }：GitHubにレート制限されました。数分後に再試行してください
plugin-settings-github-rate-limited-plain = GitHubにレート制限されました。数分後に再試行してください
plugin-settings-github-api-status = { $what }：GitHubが{ $status }を返しました
plugin-settings-github-search-url = 検索のアドレスが正しくありません：{ $error }
plugin-settings-github-search-what = コミュニティアプリを検索する
plugin-settings-github-search-status = コミュニティアプリの検索中にGitHubが{ $status }を返しました
plugin-settings-github-search-parse = GitHubが返した検索結果を読み取れません：{ $error }
plugin-settings-github-context = { $what }：{ $error }
plugin-settings-github-timeout = { $what }：リクエストがタイムアウトし、制限時間内に応答がありませんでした
plugin-settings-github-unreachable = { $what }：GitHubに接続できません（{ $error }）
plugin-settings-github-too-big = { $max } KiBを超えています
plugin-settings-github-read-what = レスポンスを読み取る
plugin-settings-github-no-tag = { $repo }にtag { $tag }はありません
plugin-settings-github-no-release = { $repo }には正式リリースがありません。v<セマンティックバージョン>のtag（例：v1.0.0）が必要です
plugin-settings-github-no-release-tag = v<セマンティックバージョン>の正式リリースtagがありません
plugin-settings-github-tag-version-mismatch = tag { $tag }がマニフェストのバージョン{ $version }と一致しません。公開時のtagはv<マニフェストのバージョン>にしてください
plugin-settings-github-tag-version-short = tag { $tag }がマニフェストのバージョン{ $version }と一致しません
plugin-settings-github-source-invalid = 提供元`{ $text }`が正しくありません：{ $why }。github:<owner>/<repo>[/<サブディレクトリ>]の形式で、@<tag>や@<40桁のコミット>を付けられます
plugin-settings-github-source-scheme = github:で始まる提供元のみ対応しています
plugin-settings-github-source-no-name = リポジトリ名がありません
plugin-settings-github-source-name-chars = ownerとリポジトリ名には英数字、`-`、`_`、`.`のみ使えます
plugin-settings-github-source-subdir-empty = サブディレクトリに空のパス要素があります。先頭や末尾の`/`も空の要素として扱います
plugin-settings-github-source-subdir-long = サブディレクトリが200文字を超えています
plugin-settings-github-source-subdir-dot = サブディレクトリに`.`または`..`の要素があります
plugin-settings-github-source-subdir-chars = サブディレクトリには英数字、`-`、`_`、`.`、`/`のみ使えます
plugin-settings-github-source-bad-tag = @の後のtagが不正です
plugin-settings-github-sibling-url = { $url }の隣の{ $path }は有効なアドレスではありません：{ $error }

# ── 提供元とインデックスのアドレス（source_address.rs、index_location.rs）──

plugin-settings-source-address-invalid = アドレス`{ $text }`が正しくありません：{ $why }
plugin-settings-source-address-tls = httpsのみ対応：未審査の提供元では、TLSで内容が改ざんされていないことを確認します
plugin-settings-source-address-https-only = httpsアドレスとGitHubリポジトリのみ対応しています
plugin-settings-source-address-formats = リポジトリのアドレス、owner/repo、またはインデックスのhttpsアドレスで書いてください
plugin-settings-source-not-a-repo = `{ $text }`はアプリのリポジトリではありません。インデックスはストアで追加し、そこからインストールしてください
plugin-settings-source-page-invalid = GitHubアドレス`{ $path }`が正しくありません。https://github.com/<owner>/<repo>の形式で、/tree/<tag>[/<サブディレクトリ>]や/commit/<コミット>を付けられます
plugin-settings-index-address-invalid = インデックスのアドレス`{ $text }`が正しくありません：{ $why }
plugin-settings-index-address-https = httpsのみ対応しています
plugin-settings-item-source-invalid = 提供元`{ $source }`が正しくありません

# ── モデルゲートウェイとモデルプロバイダー（models.rs、model_providers.rs）──

plugin-settings-models-gateway-not-running = モデルゲートウェイ{ $id }が動いていません
plugin-settings-models-gateway-absent = モデルゲートウェイ（provides.modelsを宣言するシステムアプリ）がインストールされていません
plugin-settings-models-revoke-not-running = モデルゲートウェイが動いていないため、{ $id }の資格情報を取り消せません
plugin-settings-models-revoke-provider-not-running = モデルゲートウェイ{ $provider }が動いていないため、{ $id }の資格情報を取り消せません
plugin-settings-models-reconcile-failed = モデルゲートウェイの資格情報の照合に失敗しました：{ $error }
plugin-settings-models-credential-rejected = モデルゲートウェイ{ $provider }がサーバーが発行した資格情報{ $name }を受け付けません。発行後にモデル一覧を要求しても401が返ります
plugin-settings-models-unreachable = モデルゲートウェイ{ $provider }に接続できません：{ $error }
plugin-settings-models-list-error = モデルゲートウェイ{ $provider }がモデル一覧の応答でエラーを返しました：HTTP { $status }
plugin-settings-models-provider-error = デフォルトモデルのプロバイダーでエラーが発生しました：{ $error }
plugin-settings-models-provider-invalid-reply = プロバイダーが有効なチャット完了を返しませんでした
plugin-settings-models-list-malformed = モデルゲートウェイ{ $provider }のモデル一覧の形式が不正です
plugin-settings-models-connect-first = Bot が返信できるように、「モデル設定」でモデルを接続してください。
plugin-settings-models-not-ready = モデルゲートウェイ{ $provider }の準備ができていません：{ $error }
plugin-settings-models-command-rejected = モデルゲートウェイ{ $provider }が資格情報のコマンドを拒否しました（{ $command }、終了コード{ $code }）：{ $stderr }
plugin-settings-model-action-save = 保存
plugin-settings-model-action-test = テスト
plugin-settings-model-disconnected-saved = プロバイダーに接続できませんでしたが、keyは保存しました
plugin-settings-model-disconnected-unsaved = プロバイダーに接続できず、何も保存していません
plugin-settings-model-no-provider-commands = モデルゲートウェイ{ $provider }はプロバイダーのコマンドを宣言していません
plugin-settings-model-provider-unsupported = モデルゲートウェイはプロバイダー{ $id }に対応していません
plugin-settings-model-custom-unsupported = モデルゲートウェイはカスタムプロバイダーに対応していません
plugin-settings-model-custom-encode-failed = カスタムプロバイダーのリクエストのエンコードに失敗しました
plugin-settings-model-command-timeout = モデルゲートウェイ{ $provider }のコマンドが失敗したか、タイムアウトしました
plugin-settings-model-invalid-endpoint = プロバイダーのアドレスかモデル名が不正です
plugin-settings-model-key-rejected = プロバイダーがkeyを拒否したため、keyは保存していません
plugin-settings-model-provider-command-failed = モデルゲートウェイ{ $provider }でプロバイダーの{ $action }に失敗しました。終了コード{ $code }
plugin-settings-model-list-failed = モデルゲートウェイ{ $provider }がプロバイダー一覧の取得に失敗しました：{ $error }
plugin-settings-model-list-exit = モデルゲートウェイ{ $provider }がプロバイダー一覧の取得に失敗しました。終了コード{ $code }
plugin-settings-model-list-not-json = モデルゲートウェイ{ $provider }のプロバイダー一覧は有効なJSONではありません
plugin-settings-model-key-invalid = keyは空でなく、改行を含まず、4 KiB以下である必要があります
plugin-settings-model-url-invalid = プロバイダーのアドレスはhttpかhttpsのURLで、空白を含まず、2 KiB以下である必要があります
plugin-settings-model-name-invalid = モデル名は空でなく、空白や制御文字を含まず、200バイト以下である必要があります
plugin-settings-model-request-format = モデルプロバイダーのリクエスト形式が正しくありません

# ── ストアのインデックス（storefront）────────────────────────────────────

plugin-settings-store-index-fetch-failed = 公式インデックスを取得できません：{ $error }

# ── ヘルスチェック（health.rs）───────────────────────────────────────────

plugin-settings-health-timeout = コンテナ{ $container }が制限時間内に準備できませんでした（最後の状態：{ $state }）
plugin-settings-health-unhealthy = コンテナ{ $container }のヘルスチェックが失敗しました
plugin-settings-health-exited = コンテナ{ $container }が終了しました（終了コード{ $code }）
plugin-settings-health-no-healthcheck = コンテナ{ $container }はservice_healthyが求められていますが、ヘルスチェックが設定されていません
plugin-settings-health-webui-timeout = Webページの入り口{ $url }が制限時間内に応答しませんでした：{ $last_error }
plugin-settings-health-log-tail = 最近のログ：
plugin-settings-health-probes = 最近のプローブ：
plugin-settings-health-probe-line = [終了コード{ $code }] { $output }
plugin-settings-health-probe-line-unknown = [終了コード ?] { $output }
plugin-settings-health-container-missing = コンテナ{ $container }が見つかりません
plugin-settings-health-one-shot-exit = 1回限りサービスのコンテナ{ $container }が終了コード0で終了しませんでした（{ $status }、終了コード{ $exit_code }）
plugin-settings-health-not-running = コンテナ{ $container }が動いていません（{ $status }）

# ── アプリのシークレット（secrets.rs）─────────────────────────────

plugin-settings-secret-missing = アプリのシークレット{ $name }が見つかりません
plugin-settings-secret-not-text = アプリのシークレット{ $name }がテキストではありません
plugin-settings-secret-invalid = アプリのシークレット{ $name }：コンテナ{ $container }の{ $path }は{ $reason }
plugin-settings-secret-not-regular = 通常のファイルではありません
plugin-settings-secret-too-big = 64 KiBを超えています

# ── 外部プラグインのBot（bots.rs）────────────────────────────────────────

plugin-settings-bot-avatar-missing = bot.avatarが指す{ $path }がアプリのパッケージにありません
plugin-settings-bot-avatar-type = bot.avatarはpng・jpg・gif・webpのみ使えます：{ $path }
plugin-settings-bot-device-name = Wontonアプリ{ $id }（ボット{ $bot }）
plugin-settings-bot-account-deactivated = ボットのアカウント{ $user }は停止済みです。データを削除してこのアプリをアンインストールし、再インストールすると新しいアカウントが割り当てられます
plugin-settings-bot-device-record-missing = ボット{ $bot }のデバイスの記録がありません。アプリを再インストールしてください（データは削除しなくてかまいません）
plugin-settings-bot-device-revoked = ボット{ $user }のデバイスが取り消されたため、アプリが持っているMatrixトークンは無効になりました。アプリを再インストールすると（データは削除しなくてかまいません）再発行されます

# ── LAN検出・送信コンテナ・ストアインデックスの補足（lan/detect.rs、docker_limits.rs、egress、storefront）──

plugin-settings-lan-detect-wait-exit = 1回限りコンテナの終了を待つ
plugin-settings-lan-detect-read-output = 1回限りコンテナの出力を読み取る
plugin-settings-lan-detect-list-networks = ネットワークの一覧を取得する
plugin-settings-limits-update-container = コンテナ{ $name }を更新する
plugin-settings-mirror-docker-proxy = Dockerのプロキシ設定を照会する
plugin-settings-mirror-inspect = イメージを検査する
plugin-settings-mirror-inspect-image = イメージ{ $image }を検査する
plugin-settings-mirror-pull-image = イメージ{ $image }を取得する
plugin-settings-egress-rebuild-failed = 新しいプロキシ設定での再構築に失敗しました：{ $error }
plugin-settings-egress-gost-config = gostの設定の生成に失敗しました：{ $error }
plugin-settings-egress-prepare-dir = { $dir }の準備に失敗しました：{ $error }
plugin-settings-egress-write-file = { $path }への書き込みに失敗しました：{ $error }
plugin-settings-egress-remove-dir = { $dir }の削除に失敗しました：{ $error }
plugin-settings-store-index-no-format = formatがありません
plugin-settings-store-index-docs-problem = 公式インデックスのdocsを読み取れません：{ $error }
plugin-settings-store-index-onboarding-problem = 公式インデックスのonboardingを読み取れません：{ $error }
plugin-settings-store-index-mirrors-problem = 公式インデックスのregistry_mirrorsを読み取れません：{ $error }
plugin-settings-store-index-read-failed = 公式インデックス{ $path }を読み取れません：{ $error }

plugin-secret-unknown = リクエストに知らない鍵があります: { $names }
plugin-secret-required-missing = 必須の鍵がまだ入力されていません: { $names }
plugin-secret-required-clear = 必須の鍵 { $name } は空にできません
plugin-secret-not-input = { $name } はオーナーが入力する鍵ではありません
chat-card-secrets-not-used = この確認では鍵を受け取りません
