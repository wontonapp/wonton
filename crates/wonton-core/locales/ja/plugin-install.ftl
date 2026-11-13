## 自宅サーバーがアプリのインストールと実行時に送るテキスト（ADR-0043）。完全な文を1メッセージにし、
## コードでの組み立てはしない。データ（パス、ポート、イメージ名、コンテナ名、Dockerの応答）は
## パラメータとして文中に入れ、翻訳しない。説明はです・ます調、短い項目は名詞形。

# ── 実行中の操作（manager.rs busy）──────────────────────────────────────────

plugin-install-busy-installing = アプリ{ $id }をインストールしています
plugin-install-busy-uninstalling = アプリ{ $id }をアンインストールしています
plugin-install-busy-upgrading = アプリ{ $id }をアップデートしています

# ── インストールとアンインストールの競合と権限（manager.rs、install_entry.rs、written_by.rs）

plugin-install-conflict-already-installed = アプリ{ $id }はすでにインストールされています
plugin-install-conflict-slug = アプリ{ $id }は、インストール済みの{ $other }と同じウェブアドレスを使っています
plugin-install-conflict-models-gateway = モデルゲートウェイ{ $id }はすでにインストールされています。同時に1つしか持てません
plugin-install-conflict-egress-proxy-in-use = このプロキシアプリは使用中です。先にプロキシの設定で別のものに切り替えてからアンインストールしてください
plugin-install-conflict-lan-rebuilding = アプリ{ $id }は新しいローカルネットワークアドレスで再構築中です
plugin-install-uninstall-unfinished = アンインストールが完了しませんでした：{ $error }
plugin-install-permission-not-declared = マニフェストに宣言されていない権限{ $permission }が許可されました
plugin-install-permission-unsupported = 自宅サーバーはまだ権限{ $permission }に対応していません
plugin-install-record-unreadable = このアプリのインストール記録を読み取れません。アンインストール（データは残せます）してからインストールし直してください：{ $error }
plugin-install-record-detail-failed = アプリ{ $id }の詳細ページを作成できません
plugin-install-not-agent-written = { $id }はエージェントが作ったアプリではないため、「毎回確認」の設定はありません

# ── ロールバック（lifecycle.rs）：失敗リストの項目とまとめ ───────────────────

plugin-install-rollback-item = { $what }：{ $error }
plugin-install-rollback-unfinished = ロールバックが完了せず、一部が残っている可能性があります：{ $failures }
plugin-install-rb-container = コンテナ{ $name }
plugin-install-rb-home-room = あなたのメインルーム{ $room }
plugin-install-rb-bot-device = ボット{ $bot }のデバイス
plugin-install-rb-model-credentials = アプリ{ $id }のモデルゲートウェイ認証情報
plugin-install-rb-oidc-client = OIDCクライアント{ $id }
plugin-install-rb-volume = ボリューム{ $volume }
plugin-install-rb-network = ネットワーク{ $network }
plugin-install-rb-core-leave-network = 自宅サーバーをネットワーク{ $network }から退出させる
plugin-install-rb-egress-leave-network = プロキシコンテナをネットワーク{ $network }から退出させる
plugin-install-rb-directory = ディレクトリ{ $dir }
plugin-install-rb-secret = アプリ{ $id }のキー

# name_takenに入る語（どの種類のリソースか）。
plugin-install-word-volume = ボリューム
plugin-install-word-network = ネットワーク
plugin-install-word-container = コンテナ

# ── デプロイ中の失敗（lifecycle.rs）─────────────────────────────────────────

plugin-install-oidc-unconfigured = 自宅サーバーにIDプロバイダーが設定されていません（WONTON_OIDC_ADMIN_URL / WONTON_OIDC_PUBLIC_URL）
plugin-install-shared-folder-mkdir-failed = 共有フォルダーのディレクトリ{ $dir }を作成できません：{ $error }
plugin-install-shared-folder-prepare-failed = 共有フォルダーを準備できません：{ $error }
plugin-install-app-data-prepare-failed = アプリデータディレクトリを準備できません：{ $error }
plugin-install-configs-pack-failed = { $service }の設定ファイルをまとめられません：{ $error }
plugin-install-teardown-delete-failed = { $dir }を削除できません：{ $error }
plugin-install-teardown-leftovers = アンインストール後に一部が残っています：{ $left }

# ── 名前の占有と不明なリソース（leftovers.rs）───────────────────────────────

plugin-install-name-taken-owned = { $what }{ $name }はアプリ{ $owner }のもので、このアプリの{ $what }と名前が重なっています
plugin-install-name-taken-unknown = { $what }{ $name }はすでに存在し、自宅サーバーがアプリのために作ったものではありません
plugin-install-stranger-attached = ネットワーク{ $network }に{ $names }が接続されたままです。別の自宅サーバーか、あなた自身のコンテナの可能性があります。1台のDockerで複数の自宅サーバーを動かすときは、それぞれにインスタンス名（WONTON_INSTANCE）を設定してください

# ── 突き合わせと起動（reconcile.rs、start.rs）───────────────────────────────

plugin-install-network-mismatch = アプリのネットワーク{ $network }は{ $actual }が、マニフェストと自宅サーバーの動作からすると{ $expected }はずです：アプリを再インストール（データは残せます）すると反映されます
plugin-install-resident-stopped = このアプリの常駐サービスが停止しています
plugin-install-resident-partial = 一部の常駐サービスがまだ動いており、次のコンテナが停止しました：{ $containers }
plugin-install-containers-unhealthy = 一部のコンテナが不健康です：{ $failures }
plugin-install-container-unhealthy = コンテナ{ $container }のヘルスチェックが失敗しました（最後のプローブ、終了コード { $code }）：{ $output }
plugin-install-container-unhealthy-nocode = コンテナ{ $container }のヘルスチェックが失敗しました（最後のプローブ、終了コード不明）：{ $output }
plugin-install-container-unhealthy-noprobe = コンテナ{ $container }のヘルスチェックが失敗しました（Docker にプローブの結果が残っていません）
plugin-install-container-health-probe-failed = コンテナ { $container } のヘルスプローブに失敗しました（GET { $path }）：{ $reason }
plugin-install-container-not-running = コンテナ{ $container }が動いていません（{ $status }）
plugin-install-one-shot-failed = 一度きりのサービスのコンテナ{ $container }が 0 で終了しませんでした（{ $status }、終了コード { $code }）
plugin-install-reach-none = 存在しない
plugin-install-reach-egress = インターネットに出られる
plugin-install-reach-internal = インターネットに出られない
plugin-install-reach-isolated = インターネットに出られず、ホストから分離されている
plugin-install-start-failed = 起動できませんでした：{ $error }
plugin-install-conflict-rebuilding = アプリ{ $id }は再構築中です。終わってから起動してください
plugin-install-record-unusable = このアプリのインストール記録が壊れているため、起動できません
plugin-install-container-missing = コンテナ{ $name }は存在しません
plugin-install-container-not-owned = コンテナ{ $name }はアプリ{ $id }のものではありません

# ── メモリ不足（oom.rs）────────────────────────────────────────────────────

plugin-install-oom-notice = サービス{ $service }がメモリ不足で、システムに強制終了されました{ $limit }
plugin-install-oom-reason = サービス{ $service }がメモリ不足で、システムに強制終了されました{ $limit }
plugin-install-oom-limit-both = （サービスの上限は{ $size }。全アプリの合計が上限に達した可能性もあります）
plugin-install-oom-limit-service = （上限{ $size }）
plugin-install-oom-limit-total = （全アプリの合計が上限に達した可能性があります）

# ── CPUとメモリの上限（limits_update.rs、limits.rsのAPI経路）─────────────

plugin-install-limits-busy = アプリ{ $id }はインストール・アンインストール・再構築のいずれかの実行中です。終わってから操作してください
plugin-install-limits-min-memory = memoryは{ $mib } MiB以上にしてください
plugin-install-limits-ncpu-missing = Docker /infoがNCPUを報告していません
plugin-install-limits-ncpu-invalid = Docker /infoが報告したNCPUが不正です
plugin-install-limits-ncpu-out-of-range = Docker /infoのNCPUが範囲外です
plugin-install-limits-cpu-percent-range = cpu_percentは1から{ $max }の間にしてください
plugin-install-limits-host-memory-unknown = ホストの合計メモリを読み取れないため、既定値に戻すのは再構築後になります
plugin-install-limits-ncpu-bad = Dockerが報告したCPUコア数{ $ncpu }が範囲外です
plugin-install-service-error = サービス{ $service }：{ $error }
plugin-install-limits-mem-reservation-order = mem_reservationはmem_limitより大きくできません
plugin-install-limits-memory-format = requirements.memoryの`{ $value }`が不正です
plugin-install-limits-field-forward = { $field }：{ $error }
plugin-install-limits-positive = { $field }は0より大きい必要があります
plugin-install-limits-integer-positive = { $field }は0より大きい整数である必要があります
plugin-install-limits-cpus-format = cpusの`{ $text }`が不正です
plugin-install-limits-cpus-positive = cpusは0より大きい必要があります
plugin-install-limits-cpus-range = cpusの`{ $text }`が範囲外です
plugin-install-limits-memory-range = メモリ上限がDockerの対応範囲外です

# ── アプリパッケージ（package.rs）───────────────────────────────────────────

plugin-install-pkg-yaml = composeファイルは正しいYAMLではありません：{ $error }
plugin-install-pkg-not-mapping = composeファイルの最上位はマッピングである必要があります
plugin-install-pkg-missing-manifest = composeファイルに`x-wonton`拡張フィールドがありません。マニフェストのないcomposeは、先に権限宣言を補ってください
plugin-install-pkg-manifest = `x-wonton`が不正です：{ $error }
plugin-install-pkg-service-mapping = サービス定義はマッピングである必要があります
plugin-install-pkg-unsupported-key = `{ $key }`には対応していません：{ $reason }
plugin-install-pkg-network-mode-unsupported = network_mode`{ $mode }`には対応していません：{ $reason }
plugin-install-pkg-configs-mapping = マッピングで、fileかcontentを指定する必要があります
plugin-install-pkg-configs-name-chars = 構成名に使えるのは英数字と-、_、.だけです
plugin-install-pkg-configs-key-unsupported = `{ $key }`には対応していません
plugin-install-pkg-configs-file-content-one = fileとcontentはどちらか一方だけ書いてください
plugin-install-pkg-configs-file-content-required = fileかcontentを書く必要があります
plugin-install-pkg-configs-file-content-string = fileとcontentは文字列である必要があります
plugin-install-pkg-mapping-required = マッピングである必要があります
plugin-install-arch-gzip-failed = gzipを展開できません：{ $error }
plugin-install-arch-unpacked-too-big = 展開後に{ $mib } MiBを超えています
plugin-install-arch-too-many-entries = アーカイブに{ $max }項目を超えて含まれています
plugin-install-pkg-top-level = 最上位のフィールド`{ $key }`は対応していません：{ $reason }
plugin-install-pkg-service = サービス`{ $service }`：{ $error }
plugin-install-pkg-interpolate = サービス`{ $service }`の補間に失敗しました：{ $error }
plugin-install-pkg-unknown-service = { $field }が参照するサービス`{ $service }`は存在しません
plugin-install-pkg-gpu-image-same = gpu.images.{ $service }.{ $vendor }がサービスの共通イメージと同じです。共通イメージを使うなら書く必要はありません
plugin-install-pkg-volume = 名前付きボリューム`{ $name }`が不正です。オプションなしのローカルボリュームだけに対応しています
plugin-install-pkg-config = 構成`{ $name }`：{ $error }
plugin-install-pkg-config-interpolate = 構成`{ $name }`の補間に失敗しました：{ $error }
plugin-install-pkg-reference = { $field }：{ $error }
plugin-install-pkg-file = { $field }が参照するファイル`{ $path }`：{ $error }
plugin-install-pkg-compose-file = アプリパッケージのcompose.yaml：{ $error }
plugin-install-pkg-archive = アプリパッケージのアーカイブ：{ $error }
plugin-install-pkg-too-big = アプリパッケージの合計は{ $kib } KiBで、上限の{ $max } KiBを超えています
plugin-install-pkg-not-utf8 = UTF-8テキストではありません
plugin-install-pkg-needs-service = サービスが少なくとも1つ必要です
plugin-install-install-icon-file-missing = { $field } { $raw }：ファイルが存在しないか、PNG/SVGではありません
plugin-install-install-icon-error = アイコン{ $path }：{ $reason }
plugin-install-pkg-icon-file = `{ $icon }`はアプリパッケージ内のPNGまたはSVGファイルである必要があります
plugin-install-pkg-config-undeclared = 最上位のconfigsで宣言されていません
plugin-install-pkg-config-missing = アプリパッケージに`{ $path }`がありません
plugin-install-pkg-ref-bot-service = { $field }をボットサービスに向けることはできません。ボットサービスはボットごとに1つです
plugin-install-pkg-egress-network-mode = provides.egress_proxy.serviceにはnetwork_modeを書けません
plugin-install-pkg-not-in-subset = 自宅サーバーが対応するComposeのサブセット外です
plugin-install-pkg-reserved-service = このサービス名は自宅サーバーの中継が予約しているため、アプリでは使えません
plugin-install-pkg-service-name-dns = サービス名は正しいDNSラベルである必要があります。使えるのは小文字、数字、-だけです
plugin-install-pkg-service-name-dashes = サービス名を-で終わらせたり、--を含めたりすることはできません

# ── アプリパッケージのアーカイブ（package_archive.rs）───────────────────────

plugin-install-arch-read-failed = アーカイブを読めません：{ $error }
plugin-install-arch-path-invalid = アーカイブ内のパスが不正です：{ $error }
plugin-install-arch-zip-read-failed = zipを読めません：{ $error }
plugin-install-arch-symlink = シンボリックリンク
plugin-install-arch-zip-path-invalid = zip内のパスが不正です：{ $error }
plugin-install-arch-two-roots = { $a }と{ $b }の両方にcompose.yamlがあります。アーカイブ1個に入れられるアプリは1つです
plugin-install-arch-missing-compose = アーカイブにcompose.yamlがありません
plugin-install-arch-not-regular = `{ $rel }`は通常のファイルではありません（{ $what }）。アプリパッケージにシンボリックリンク、ハードリンク、デバイスは置けません
plugin-install-arch-duplicate-file = アーカイブに`{ $rel }`が2つあります
plugin-install-arch-not-tar = tarアーカイブではありません：{ $error }
plugin-install-arch-extract-failed = `{ $path }`を展開できません：{ $error }
plugin-install-arch-read-entry-failed = アーカイブ内の`{ $path }`を読めません：{ $error }

# ── composeの書き方（compose.rs）───────────────────────────────────────────

plugin-install-compose-ports-forbidden = アプリはポートをホストに公開しません。ウェブページは必ず自宅サーバーのゲートウェイを経由します
plugin-install-compose-networks-forbidden = アプリのネットワークは自宅サーバーが作成・管理します
plugin-install-compose-container-name-forbidden = コンテナ名は自宅サーバーが割り当てます
plugin-install-compose-build-forbidden = アプリは公開済みのイメージだけを使い、ホスト上でビルドしません
plugin-install-compose-env-file-forbidden = アプリパッケージのcomposeファイルは1つです。環境変数はenvironmentに書いてください
plugin-install-compose-extends-forbidden = アプリパッケージのcomposeファイルは1つで、extendsには対応していません
plugin-install-compose-links-forbidden = 同じアプリのサービスは同じネットワークにあり、サービス名で直接つながります
plugin-install-compose-external-links-forbidden = アプリ同士は既定では互いにアクセスできません
plugin-install-compose-volumes-from-forbidden = データの共有には名前付きボリュームかアプリデータディレクトリを使ってください
plugin-install-compose-top-networks-forbidden = アプリのネットワークは自宅サーバーが作成・管理します
plugin-install-compose-top-secrets-forbidden = キーは自宅サーバーが注入します。composeでsecretsを宣言することはできません
plugin-install-compose-network-host = ホストネットワークは高権限です（network.host）
plugin-install-compose-network-mode = network_modeに書けるのはnone、host、service:<同じアプリの別のサービス>だけです。アプリのネットワークは自宅サーバーが作成・管理します
plugin-install-compose-config-external-forbidden = 構成はアプリパッケージからだけ読み込めます
plugin-install-compose-config-environment-forbidden = 自宅サーバーは自分の環境変数をアプリに渡しません。必要なときはcontent補間を使ってください
plugin-install-compose-config-name-forbidden = 自宅サーバーはDockerの構成オブジェクトを作らないため、nameは不要です
plugin-install-compose-pair-missing = { $field }の`{ $key }`に値がありません（自宅サーバーはホストの環境変数を渡しません）
plugin-install-compose-device-format = デバイスのマッピング`{ $text }`の形式が不正です
plugin-install-compose-device-missing-source = デバイスのマッピングにsource/path_on_hostがありません
plugin-install-compose-device-missing-target = デバイスのマッピングにtarget/path_in_containerがありません
plugin-install-compose-duration-invalid = 期間`{ $text }`が不正です（例：30s、1m30s、500ms）
plugin-install-compose-bytes-invalid = サイズ`{ $text }`が不正です（例：64m、1g）

# ── configs（configs.rs）───────────────────────────────────────────────────

plugin-install-cfg-undeclared = 構成`{ $name }`が最上位のconfigsで宣言されていません
plugin-install-cfg-target-duplicate = configsのターゲット`{ $raw }`が重複しています
plugin-install-cfg-target-overlaps-mount = configsのターゲット`{ $raw }`がマウントポイント{ $mount }と重なっています。構成はコンテナ自身のファイルシステムに置かれるため、マウントポイントには書き込めません
plugin-install-cfg-target-absolute = configsのターゲット`{ $raw }`は絶対パスである必要があり、..を含められません
plugin-install-cfg-target-root = configsのターゲットに/は使えません
plugin-install-cfg-id-invalid = configsの{ $field }`{ $value }`は0以上の整数である必要があります
plugin-install-cfg-mode-invalid = configsのmode`{ $value }`が不正です。0から0777の8進数の権限で指定してください
plugin-install-cfg-parent-is-file = configsのtargetの親`{ $parent }`がコンテナ{ $service }の中でファイルです。そこには書き込めません

# ── マウント（mounts.rs）───────────────────────────────────────────────────

plugin-install-mnt-short-format = ボリューム`{ $text }`の形式が不正です
plugin-install-mnt-option-unsupported = ボリュームのオプション`{ $bad }`には対応していません
plugin-install-mnt-relative-path = 相対パス`{ $path }`には対応していません。{ $var }の下のパスを使ってください
plugin-install-mnt-target-absolute = マウントターゲット{ $target }は絶対パスである必要があります
plugin-install-mnt-volume-undeclared = 名前付きボリューム{ $name }が最上位のvolumesで宣言されていません
plugin-install-mnt-bind-missing-source = バインドマウントにsourceがありません
plugin-install-mnt-host-path-parent = ホストのパス`{ $path }`に..は含められません
plugin-install-mnt-host-path-outside = ホストのパス`{ $path }`は、アプリデータディレクトリでも許可された共有フォルダーでもありません
plugin-install-mnt-shared-folder-whole = 共有フォルダーは丸ごとマウントしてください。その中の`{ $path }`だけをマウントすることはできません
plugin-install-mnt-shared-folder-readonly = 共有フォルダー{ $path }は読み取りだけ許可されているため、:roでマウントする必要があります
plugin-install-mnt-nested = ホストのパス`{ $path }`が、サービス{ $outer }がマウントしている{ $outer_path }の中にあります。同じアプリのバインドマウントは入れ子にできません（コンテナが内側のディレクトリをシンボリックリンクに差し替えて、アプリデータディレクトリの外へ逃げられるため）。並んだサブディレクトリを使ってください
plugin-install-mnt-owner-conflict = { $path }がサービス{ $other }と異なる所有者でマウントされており、自宅サーバーは誰に渡すべきか判断できません
plugin-install-mnt-numeric-user = アプリデータディレクトリをマウントしたので、user{ $user }は数値のuid:gidで書いてください。自宅サーバーはそれでディレクトリの渡し先を決めます

# ── 権限（privileges.rs）───────────────────────────────────────────────────

plugin-install-priv-agent-high-risk = エージェントとボットのアプリは、高権限または最高権限の{ $permission }を要求できません
plugin-install-priv-fixed-device = 固定デバイスの権限で指定できるのは/dev/net/tunと/dev/fuseだけです。{ $permission }は指定できません
plugin-install-priv-host-network-ports = ホストネットワークのサービスはlan.portsも宣言できません。ポートはアプリが直接ホストネットワークで待ち受けます
plugin-install-priv-host-network-sso = ホストネットワークのwebuiではsso: proxyを使えません。oidcかnoneを使ってください
plugin-install-priv-host-network-healthcheck = ホストネットワークのwebuiサービスには、有効なhealthcheckの記載が必要です
plugin-install-priv-host-network-mail = ホストネットワークを使うアプリはメールを送れません。送信元はネットワークセグメントで識別され、ホストネットワークからのメールは特定できません。アプリケーションネットワークを使うか、メール送信を外してください
plugin-install-priv-cap-invalid = cap_addのケーパビリティ`{ $cap }`が不正です
plugin-install-priv-cap-drop-invalid = cap_dropのケーパビリティ`{ $cap }`が不正です
plugin-install-priv-security-opt-invalid = security_optの`{ $opt }`が不正です（no-new-privilegesの値はtrueまたはfalseのみ）
plugin-install-priv-device-not-allowed = デバイス{ $source }は、許可された固定デバイスではありません
plugin-install-priv-device-absolute = デバイスのターゲット{ $target }は絶対パスである必要があります
plugin-install-priv-device-mode = デバイス{ $source }の権限{ $permissions }が不正です
plugin-install-priv-pid-host-only = pidはhostだけに対応しています
plugin-install-priv-ipc-host-only = ipcはhostだけに対応しています
plugin-install-priv-socket-mount = Docker socketは/var/run/docker.sock:/var/run/docker.sockとして書き込み可能でマウントする必要があります
plugin-install-priv-missing-cap-add = 権限{ $permission }に対応するcap_addフィールドがありません
plugin-install-priv-missing-devices = 権限{ $permission }に対応するdevicesフィールドがありません
plugin-install-priv-field-requires = { $field }にはマニフェスト権限{ $permission }が必要です
plugin-install-priv-missing-field = マニフェスト権限{ $permission }に対応する{ $field }フィールドがありません
plugin-install-optional-not-allowed = 権限{ $permission }は任意にできません
plugin-install-optional-declared-twice = 権限{ $permission }が permissions と optional_permissions の両方にあります
plugin-install-optional-off-loss = 任意の権限{ $permission }には off_loss（切ったとき失うもの）を中国語と英語の両方で書きます
plugin-install-optional-gpu-required = 任意の権限{ $permission }は切るとGPUを渡さないので、requirements.gpu.need は required にできません
plugin-install-optional-not-enabled = 任意権限{ $permission }は任意として宣言されていません
plugin-install-optional-duplicate = 任意権限{ $permission }が二度あります
upgrade-optional-became-required = このバージョンは{ $permission }を必須にします。確認すると付与されます
upgrade-optional-removed = このバージョンは任意権限{ $permission }を削除します

# ── provides（provides.rs）─────────────────────────────────────────────────

plugin-install-prv-egress-needs-permission = provides.egress_proxyにはegress.proxy権限の申請が必要です
plugin-install-prv-permission-needs-egress = egress.proxy権限にはprovides.egress_proxyの宣言が必要です
plugin-install-prv-egress-needs-network = provides.egress_proxyにはnetwork.egress権限の申請が必要です
plugin-install-prv-egress-port = provides.egress_proxy.portを0にはできません
plugin-install-prv-empty = providesには少なくとも1つの能力の宣言が必要です
plugin-install-prv-models-system-only = provides.modelsを宣言できるのはkind: systemのアプリだけです
plugin-install-prv-models-port = provides.models.portを0にはできません
plugin-install-prv-models-apis-path = provides.models.apisのパスは/で始まる必要があります
plugin-install-prv-models-default-model = provides.models.default_modelを空にはできません
plugin-install-prv-credentials-argv = provides.models.credentials.{ $field }は空でないargvで、{ $placeholder }で認証情報の名前を示す必要があります

# ── プロキシアプリのCookieとヘッダー（proxy_auth.rs）──────────────────────

plugin-install-proxy-secret-unknown = { "{{" }secret:{ $name }{ "}}" }は、secretsのうち自宅サーバーが生成する（generate）{ $min }バイト以上の項目を参照する必要があります
plugin-install-proxy-cookie-template = Cookieの値に使えるのは{ "{{" }secret:…{ "}}" }と固定テキストだけです：`{ $template }`
plugin-install-proxy-cookie-chars = Cookieの値に空白、引用符、カンマ、セミコロン、バックスラッシュは使えません：`{ $template }`
plugin-install-proxy-header-control-chars = ヘッダーの値に制御文字は使えません：`{ $template }`
plugin-install-proxy-header-forbidden = アプリの代わりにヘッダー{ $name }を追加することはできません
plugin-install-proxy-cookie-forbidden = アプリの代わりにCookie{ $name }を追加することはできません
plugin-install-proxy-mcp-prefix = mcp.headers：{ $error }
plugin-install-proxy-mcp-header-forbidden = MCP上流にヘッダー{ $name }を追加することはできません
plugin-install-proxy-mcp-secret-undeclared = { "{{" }secret:{ $name }{ "}}" }は、secretsで宣言された項目を参照する必要があります
plugin-install-proxy-mcp-identity-placeholder = MCPヘッダーに{ "{{" }{ $placeholder }{ "}}" }のような識別情報のプレースホルダーは使えません：`{ $template }`
plugin-install-proxy-mcp-header-control-chars = ヘッダーの値に制御文字は使えません：`{ $template }`
plugin-install-proxy-template-open = `{ $template }`の中の{ "}}" }に対応する{ "{{" }がありません
plugin-install-proxy-template-close = `{ $template }`の中の{ "{{" }に対応する{ "}}" }がありません
plugin-install-proxy-template-unknown-var = `{ $template }`の中の{ "{{" }{ $var }{ "}}" }は未知のプレースホルダーです。使えるのは{ "{{" }user{ "}}" }、{ "{{" }name{ "}}" }、{ "{{" }email{ "}}" }、{ "{{" }secret:<名前>{ "}}" }です
plugin-install-proxy-secret-missing = アプリのキー{ $name }がキーストアにありません
plugin-install-proxy-secret-not-text = mcp.headersの{ $name }で、キー{ $secret }はテキストではないためヘッダーに入れられません
plugin-install-proxy-secret-newline = mcp.headersの{ $name }で、キー{ $secret }の内容に改行か制御文字が含まれています（末尾の改行も含みます）
plugin-install-proxy-secret-no-value = アプリのキー{ $secret }にはまだ値がなく、mcp.headersの{ $name }がそれを参照しています
plugin-install-proxy-mcp-no-identity = mcp.headersの{ $name }に識別情報のプレースホルダーは使えません
plugin-install-proxy-mcp-bad-value = mcp.headersの{ $name }は、埋め込むと正しいヘッダーの値になりません
plugin-install-proxy-mcp-bad-name = mcp.headersの{ $name }は正しいヘッダー名ではありません：{ $error }

# ── リソース要件（requirements.rs、host_check.rs）───────────────────────────

plugin-install-req-size-format = { $field }の`{ $size }`は512M、2Gのようなサイズで書いてください
plugin-install-req-vendor-unknown = requirements.gpu.vendorsに、このホームサーバーが知らないベンダーがあります。ホームサーバーを更新してください（amd、nvidia、intelに対応）
plugin-install-req-vendor-duplicated = requirements.gpu.vendorsに{ $vendor }が2回書かれています
plugin-install-req-arch-line = 対応するのは{ $arches }アーキテクチャだけです
plugin-install-req-memory-line = メモリ{ $memory }が必要です
plugin-install-req-gpu-vram-suffix = 、VRAMは{ $memory }以上
plugin-install-req-gpu-required = { $card }{ $memory }が必要です
plugin-install-req-gpu-optional = { $card }{ $memory }があれば使います
plugin-install-req-card-generic = GPU
plugin-install-req-card-named = { $names }のGPU
plugin-install-or-separator = または
plugin-install-host-unknown = { $what }を確認できません：{ $why }
plugin-install-host-arch-unknown-what = このホストのアーキテクチャ
plugin-install-host-arch-mismatch = このホストは{ $arch }です
plugin-install-host-memory-wanted-what = 必要なメモリ量
plugin-install-host-memory-unknown-what = このホストのメモリ
plugin-install-host-memory-low = このホストのメモリは{ $size }しかありません
plugin-install-host-gpu-unknown-what = このホストのGPU
plugin-install-host-gpu-problem = { $card }：{ $why }
plugin-install-host-gpu-not-among = このマシンの{ $host }は、このプラグインが対応する{ $wanted }にありません
plugin-install-host-gpu-missing = このホストには{ $card }がありません
plugin-install-host-fit-vram-unreadable = VRAMの要件{ $text }を読み取れません
plugin-install-host-fit-unknown = VRAMを確認できません：{ $why }
plugin-install-host-fit-integrated = 内蔵GPUはメモリと共有しており、割り当ての{ $size }は上限ではありません。足りるかどうかは判定できません
plugin-install-host-fit-vram-low = VRAMは{ $size }しかありませんが、{ $text }が必要です

# ── ホストの状態（host_facts.rs）───────────────────────────────────────────

plugin-install-facts-gpu-nvidia = NVIDIA { $device }
plugin-install-facts-gpu-amd = AMD { $device }
plugin-install-facts-gpu-intel = Intel { $device }
plugin-install-facts-gpu-other = ディスプレイデバイス{ $vendor }:{ $device }
plugin-install-facts-meminfo-missing = /proc/meminfoに{ $field }がありません
plugin-install-facts-meminfo-unreadable = /proc/meminfoを読めません：{ $error }
plugin-install-facts-docker-not-reported = Dockerが報告していません
plugin-install-facts-pci-unreadable = { $path }を読めません：{ $error }
plugin-install-facts-driver-nouveau = nouveauドライバーが読み込まれています。計算には使えず、NVIDIAの公式ドライバーが必要です
plugin-install-facts-driver-bound = { $driver }ドライバーにバインドされています
plugin-install-facts-driver-none = ドライバーが読み込まれていません
plugin-install-facts-runtime-missing = Dockerにnvidiaランタイムが登録されていません。NVIDIA Container Toolkitをインストールしてください
plugin-install-facts-runtime-unknown = Dockerにnvidiaランタイムがあるか判断できません：{ $reason }
plugin-install-facts-driver-no-render = { $driver }ドライバーにレンダーノードがなく、計算には使えません
plugin-install-facts-no-host-dev = 自宅サーバーはコンテナ内にあり、ホストの/devがマウントされていない（WONTON_HOST_DEV）ため、デバイスノードが見えません
plugin-install-facts-dri-missing = ホストの/dev/driに{ $render }がありません
plugin-install-facts-model-unknown = ドライバーがモデル名を返さず、デバイスIDしかわかりません
plugin-install-facts-vram-not-in-sysfs = NVIDIAのVRAMはsysfsにありません
plugin-install-facts-vram-unknown = ドライバーがVRAMの大きさを返しません
plugin-install-facts-core-not-in-container = 自宅サーバーはコンテナ内にないため、NVIDIAドライバーに問い合わせるためのイメージがありません
plugin-install-facts-container-trial-failed = コンテナ内でのテスト実行に失敗しました：{ $reason }
plugin-install-facts-container-image-unknown = 自宅サーバーのコンテナ{ $core }のイメージが見つかりません
plugin-install-facts-nvidia-smi-exit = nvidia-smiが{ $code }で終了しました：{ $reason }

# ── GPU（gpu.rs）───────────────────────────────────────────────────────────

plugin-install-gpu-needs-permission = gpuを宣言した場合はdevices.gpu権限の申請が必要です
plugin-install-gpu-needs-services = devices.gpuを申請した場合は、gpu.servicesでどのサービスがGPUを使うかを書く必要があります
plugin-install-gpu-needs-all-three = requirements.gpuは、devices.gpu権限とgpu.servicesと一緒に宣言する必要があります
plugin-install-gpu-services-nonempty = gpu.servicesには1つ以上の項目が必要で、重複はできません
plugin-install-gpu-images-need-vendors = gpu.imagesを書くにはrequirements.gpu.vendorsが必要です。プラグインが対応するベンダーをすべて並べてください。このマシンにある最初のベンダーのイメージを使います
plugin-install-gpu-images-service-unknown = gpu.imagesのサービス{ $service }がgpu.servicesにありません
plugin-install-gpu-images-vendor-unknown = gpu.images.{ $service }のベンダー{ $vendor }がrequirements.gpu.vendorsにありません
plugin-install-gpu-images-empty = { $field }が空です
plugin-install-gpu-images-whitespace = { $field }に空白があります。イメージ参照は1語です
plugin-install-gpu-describe-vendors = { $vendors }のGPU：{ $devices }。コンテナはグループ{ $groups }に入ります
plugin-install-gpu-image-line = サービス{ $service }はイメージ`{ $image }`を使います{ $size }
plugin-install-gpu-image-size = （約{ $size }）
plugin-install-gpu-describe-nvidia = NVIDIA GPU：すべて。NVIDIA Container Toolkit経由
plugin-install-gpu-problem-host-dev = 自宅サーバーはコンテナ内にあり、ホストの/devがマウントされていない（WONTON_HOST_DEV）ため、GPUデバイスが見えません。現行バージョンのcomposeファイルで自宅サーバーを再起動してください
plugin-install-gpu-problem-unreadable = { $path }を読めません：{ $error }
plugin-install-gpu-problem-no-device = GPUが見つかりません（PCIにディスプレイデバイスがありません）
plugin-install-gpu-problem-no-driver = GPUはあるものの、使えるドライバーがありません：{ $detail }
plugin-install-gpu-problem-no-runtime = NVIDIAドライバーはあるものの、Dockerにnvidiaランタイムが登録されていません。NVIDIA Container Toolkitをインストールし、`nvidia-ctk runtime configure --runtime=docker`を実行してください
plugin-install-gpu-problem-no-wanted-vendor = このマシンのGPU（{ $host }）は、このプラグインが対応する{ $wanted }にありません
plugin-install-gpu-problem-all-unusable = 使えるGPUがありません：{ $reasons }
plugin-install-gpu-problem-image-arch = サービス{ $service }の{ $image }は{ $arch }に対応していません
plugin-install-gpu-problem-image-unknown = { $image }がこのマシンのアーキテクチャで使えるか分かりません：{ $error }
plugin-install-gpu-vendor-other = ベンダー{ $id }
plugin-install-gpu-no-driver-none = ドライバーが読み込まれていません
plugin-install-gpu-no-driver-nouveau = nouveauドライバーが読み込まれています。計算には使えず、NVIDIAの公式ドライバーが必要です
plugin-install-gpu-no-driver-no-render = { $driver }ドライバーが読み込まれていますが、レンダーノードがありません
plugin-install-gpu-device-entry = { $address }（{ $vendor }）{ $state }
plugin-install-gpu-no-device-nodes = 中にデバイスノードがありません
plugin-install-gpu-preview-granted = GPUをサービス{ $services }に割り当てます：{ $describe }
plugin-install-gpu-preview-missing = このホストに条件に合うGPUがないため（{ $problem }）、サービス{ $services }はGPUなしで動きます
plugin-install-gpu-preview-required = サービス{ $services }にはGPUが必要です：{ $reason }

# ── 描画（render.rs）───────────────────────────────────────────────────────

plugin-install-render-no-owner-identity = このアプリはowner.identityを要求していないため、`{ $var }`を参照できません。マニフェストのpermissionsに追加してください
plugin-install-render-bot-dependency = 全アプリで共有されるサービスは、ボットサービス`{ $service }`に依存できません。ボットサービスはボットごとに1つです
plugin-install-render-one-shot-restart = service_completed_successfullyで依存される1回限りのサービスはrestart: "no"を書く必要があります。書かないと、Dockerが終了のたびに再起動を繰り返します
plugin-install-render-network-mode-none = { $field }が指すサービスにはnetwork_mode: noneを書けません
plugin-install-render-dependency-missing = depends_onが、存在しないサービス`{ $missing }`を参照しています
plugin-install-render-dependency-cycle = depends_onに循環依存があります
plugin-install-render-reserved-label = `wonton.`で始まるラベルは自宅サーバーが予約しています
plugin-install-render-quotes-unbalanced = コマンド`{ $text }`の引用符が対応していません
plugin-install-render-restart-invalid = restartの値`{ $value }`が不正です
plugin-install-render-healthcheck-format = healthcheck.testのリストはCMD、CMD-SHELL、NONEのいずれかで始める必要があります

# ── Docker（docker.rs、docker_exec.rs、core_address.rs）─────────────────────

plugin-install-docker-unavailable = Dockerに接続できません：{ $error }
plugin-install-docker-error-join = { $context }：{ $error }
plugin-install-docker-timed-out = Dockerが時間内に処理しませんでした（{ $context }）。操作はまだ続いている可能性があります
plugin-install-docker-not-found = Docker内に存在しません：{ $what }
plugin-install-docker-rejected = Dockerがリクエストを拒否しました（{ $context }）：{ $error }
plugin-install-port-taken = ポート { $place } は使用中です。使っているコンテナかプログラムを止めてから、もう一度お試しください。（{ $error }）
plugin-install-port-taken-unknown = アプリに必要なポートが使用中です。使っているコンテナかプログラムを止めてから、もう一度お試しください。（{ $error }）
plugin-lan-taken-by-plugin = アプリ { $id }
plugin-lan-taken-by-container = コンテナ { $name }
plugin-lan-taken-by-host = ホスト上のプログラム
plugin-install-lan-port-taken = ローカルネットワークのポート { $place } は{ $who }が使用中です。止めてからもう一度インストールしてください。
plugin-install-lan-port-hinted = ローカルネットワークのポート { $place } は{ $who }が使用中です。コンテナの起動で失敗します。
plugin-install-lan-port-probe-failed = ホスト上のプログラムがこれらのポートを使っているか確認できませんでした：{ $reason }
plugin-install-probe-image-replaced = 自宅サーバーのイメージが差し替えられたため、検査コンテナは作成時のイメージを使用しました。自宅サーバーを再起動すると復旧します（{ $reason }）
plugin-install-probe-image-missing = 自宅サーバーのイメージが Docker に存在せず、検査コンテナを起動できません。自宅サーバーを再起動すると復旧します。Docker の応答：{ $error }
plugin-install-docker-pull-failed = イメージの取得に失敗しました（{ $image }）：{ $error }
plugin-install-docker-context-version = バージョンの照会
plugin-install-docker-context-host-info = Docker情報の照会
plugin-install-docker-context-image-healthcheck = イメージ{ $image }のチェック
plugin-install-docker-context-image-platforms = イメージのプラットフォームを取得{ $image }
plugin-install-docker-context-network = ネットワーク{ $name }の照会
plugin-install-docker-no-ipv4-gateway = ネットワークにIPv4ゲートウェイがありません
plugin-install-docker-no-subnets = ネットワークにサブネットがありません
plugin-install-docker-context-remove-network = ネットワーク{ $name }の削除
plugin-install-docker-context-list-published-ports = 公開ポートの一覧取得
plugin-install-docker-context-connect-network = コンテナ{ $container }をネットワーク{ $network }に接続
plugin-install-docker-context-disconnect-network = コンテナ{ $container }をネットワーク{ $network }から切断
plugin-install-docker-context-create-volume = ボリューム{ $name }の作成
plugin-install-docker-context-volume-labels = ボリューム{ $name }の照会
plugin-install-docker-context-remove-volume = ボリューム{ $name }の削除
plugin-install-docker-context-create-container = コンテナ{ $name }の作成
plugin-install-docker-context-upload-archive = コンテナ{ $container }へのファイル書き込み
plugin-install-docker-context-container-missing = コンテナ{ $container }
plugin-install-docker-context-read-file = コンテナ{ $container }内の{ $path }の読み取り
plugin-install-docker-archive-unparseable = アーカイブを解析できません：{ $error }
plugin-install-docker-context-start-container = コンテナ{ $name }の起動
plugin-install-docker-context-inspect-container = コンテナ{ $name }の照会
plugin-install-docker-context-kill-container = コンテナ{ $name }に{ $signal }を送信
plugin-install-docker-context-stop-container = コンテナ{ $name }の停止
plugin-install-docker-context-remove-container = コンテナ{ $name }の削除
plugin-install-docker-context-list-containers = コンテナの一覧
plugin-install-docker-context-list-networks = ネットワークの一覧
plugin-install-docker-context-list-volumes = ボリュームの一覧
plugin-install-docker-engine-too-old = Docker Engine { $version }は古すぎます。自宅サーバーには{ $min }以上が必要です。バージョン28からは、ローカルネットワークのほかのホストがコンテナの未公開ポートに直接アクセスできなくなり、これによってアプリのネットワークもホストから完全に分離されます。Dockerをアップデートしてください
plugin-install-docker-engine-version-unrecognized = Docker Engineのバージョン`{ $version }`を認識できません
plugin-install-docker-context-exec = コンテナ{ $container }内で{ $argv }を実行
plugin-install-docker-exec-no-io = コマンドに入出力が接続されていません
plugin-install-docker-exec-timed-out = { $context }：{ $seconds }秒たっても終わりません
plugin-install-coreaddr-no-ipv4-layout = ネットワークに、自宅サーバー用のアドレスを残せるIPv4の範囲がありません
plugin-install-coreaddr-pool-exhausted-context = ネットワーク{ $name }の作成時、自宅サーバーがアプリのネットワーク用に確保しているアドレス範囲を使い切ったため、アプリをインストールできません。使っていないアプリをアンインストールするか、自宅サーバーの設定{ $var }のアドレス範囲を広げてください
plugin-install-coreaddr-pool-exhausted = 範囲{ $subnet }：/26の範囲{ $count }個のうち{ $count }個を使用中
plugin-install-coreaddr-kept-taken = { $context }。{ $attempts }回とも、同時に作成されたネットワークと範囲が重なりました
plugin-install-coreaddr-reserved-taken = 予約していた範囲{ $subnet }が、別のプログラムに取られていました。この再構築をやり直すと、別の範囲が選ばれます
plugin-install-coreaddr-context-create-network = ネットワーク{ $name }の作成
plugin-install-coreaddr-context-inspect-network = ネットワーク{ $name }の照会
plugin-install-coreaddr-no-core-address = ネットワークにはまだ、自宅サーバー用のアドレスが確保されていません

# ── 上流とLAN（upstreams.rs、host_relay.rs）───────────────────────────────

plugin-install-upstream-container-missing = コンテナ{ $container }が存在しないか、ネットワーク{ $network }内にアドレスがありません
plugin-install-upstream-mcp-headers = アプリ{ $id }のMCPヘッダーを解決できません：{ $error }
plugin-install-relay-prepare-dir-failed = 中継ディレクトリを準備できません：{ $error }
plugin-install-relay-path-too-long = 中継ソケットのディレクトリのパスが長すぎてunixソケットが収まりません。WONTON_RELAY_DIR をもっと短いディレクトリにしてください（{ $path }：{ $bytes }バイト、上限{ $max }バイト）
plugin-install-relay-listens-unknown = 待ち受け一覧を読めないため、ファイアウォールの判定は行いません
plugin-install-egress-host-no-address = 出口コンテナにプラグインネットワーク { $network } のアドレスがないため、ホストネットワークのサービスはプロキシ経由で出られません

# ── 画像（images.rs、images/preflight.rs、image_worker.rs）──────────────────

plugin-install-img-tab-icon-alpha = タブアイコンのアルファチャンネルは、完全に不透明にも完全に透明にもできません
plugin-install-img-too-many-kib = { $kib } KiBを超えています
plugin-install-img-too-many-mib = { $mib } MiBを超えています
plugin-install-img-not-png-or-jpeg = PNGでもJPEGでもない画像です
plugin-install-img-is-format = これは{ $format }画像です。スクリーンショットにはPNGかJPEGしか使えません
plugin-install-img-encode-jpeg-failed = JPEGのエンコードに失敗しました：{ $error }
plugin-install-img-not-supported-format = 対応していない画像形式です
plugin-install-img-format-no-thumbnail = { $format }からサムネイルは作れません
plugin-install-img-over-edge-or-memory = { $max }×{ $max }ピクセルかメモリ上限を超えています
plugin-install-img-decode-failed = デコードに失敗しました：{ $error }
plugin-install-img-png-too-big = PNGアイコンが{ $kib } KiBを超えています
plugin-install-img-png-content = ファイル名は.pngですが、中身がPNGではありません
plugin-install-img-not-square = アイコンは正方形である必要があります。この画像は{ $width }×{ $height }です
plugin-install-img-png-too-small = PNGアイコンは{ $min }×{ $min }以上である必要があります。この画像は{ $width }×{ $height }です
plugin-install-img-svg-too-big = SVGアイコンが{ $kib } KiBを超えています
plugin-install-img-svg-not-utf8 = SVGはUTF-8テキストである必要があります（圧縮したSVGは受け付けません）
plugin-install-img-svg-invalid = 正しいSVGではありません：{ $error }
plugin-install-img-svg-parse-failed = SVGを解析できません：{ $error }
plugin-install-img-svg-viewbox = SVGのviewBoxは正方形である必要があります。このSVGは{ $width }×{ $height }です
plugin-install-img-canvas-alloc-failed = キャンバスを確保できません
plugin-install-img-canvas-convert-failed = キャンバスを変換できません
plugin-install-img-over-max-edge = { $max }×{ $max }ピクセルを超えています
plugin-install-img-encode-png-failed = PNGのエンコードに失敗しました：{ $error }
plugin-install-img-svg-root = ルート要素は<svg xmlns="{ $ns }">である必要があります
plugin-install-img-style-rules = スタイルシートのルールは最大{ $max }件です。このSVGには{ $count }件あります
plugin-install-img-style-combinator = スタイルシートのセレクター「{ $selector }」に結合子が付いています。.cls-1のような結合子のないセレクターだけが使えます
plugin-install-img-nesting-depth = SVGの入れ子が{ $max }段を超えています
plugin-install-img-use-cycle = SVG内の<use>が循環参照しています
plugin-install-img-use-expanded = SVGの<use>を展開すると{ $max }個を超えます
plugin-install-img-use-href-count = <use>にはhrefが1つだけ必要です（hrefかxlink:href）
plugin-install-img-use-href-format = <use>のhrefは#idの形で、idに空白は使えません：{ $href }
plugin-install-img-element-forbidden = アイコンに<{ $name }>は使えません
plugin-install-img-clip-path-units = <clipPath>のclipPathUnitsはuserSpaceOnUseである必要があります
plugin-install-img-mask-units = <mask>にはmaskUnits="userSpaceOnUse"を書き、maskContentUnitsはuserSpaceOnUseにしてください
plugin-install-img-gradient-stops = グラデーションの<stop>は最大{ $max }個です。このグラデーションには{ $count }個あります
plugin-install-img-worker-usage = 使い方：wonton-core { $command } <icon.png|icon.svg|tab-icon.png|tab-icon.svg|screenshot|thumbnail|preview>（自宅サーバー内部用）
plugin-install-img-worker-unavailable = 画像処理のプロセスを使えません：{ $error }
plugin-install-img-worker-rlimit = リソース上限を設定できません：{ $error }
plugin-install-img-worker-stdin = 標準入力を読めません：{ $error }
plugin-install-img-worker-stdout = 標準出力に書き込めません：{ $error }
plugin-install-img-worker-spawn-failed = { $program }を起動できません：{ $error }
plugin-install-img-worker-communicate = 子プロセスと通信できません：{ $error }
plugin-install-img-worker-timeout = 処理が{ $seconds }秒を超えたため、停止しました
plugin-install-img-worker-crashed = 処理中にクラッシュしました（終了コード{ $code }）
plugin-install-img-worker-cpu-timeout = 処理に{ $seconds }秒を超えるCPU時間がかかったため、停止しました
plugin-install-img-worker-memory-limit = 処理に必要なメモリが{ $mib } MiBを超えました
plugin-install-img-worker-killed = 画像処理のプロセスが強制終了されました（SIGKILL）。メモリ不足の可能性が高いです
plugin-install-img-worker-signal = 画像処理のプロセスがシグナル{ $signal }で終了しました
plugin-install-img-worker-malformed = 画像処理のプロセスの出力形式が不正です
plugin-install-img-worker-host-exited = 画像処理のヘルパーが終了しました：{ $error }
plugin-install-img-worker-host-not-ready = 画像処理のヘルパーが準備できませんでした：{ $error }
plugin-install-img-worker-host-protocol = 画像処理のヘルパーのプロトコルバージョンは { $got } ですが、コアは { $want } です

## 预装（启动时随服务器带的包）失败的原因。
plugin-install-preinstall-retrying = プリインストールに失敗しました（{ $attempts } 回試行、自動で再試行します）
plugin-install-preinstall-failed = プリインストールに失敗しました

# ── 能力依存（capabilities.rs、ADR-0053）──────────────────────────────────

plugin-capability-unknown = このコアは能力 { $capability } を認識しません。コアを更新してからインストールしてください
plugin-capability-third-party-format = 能力 { $capability } が不正です。サードパーティー能力は x-<作者>.<名前> の形で、作者と名前は小文字・数字・ハイフンで書きます
plugin-capability-no-consumer = { $capability } には利用者宣言がなく、requires や wants には書けません
plugin-capability-duplicate = { $field } の能力 { $capability } が重複しています
plugin-capability-in-both-lists = 能力 { $capability } を requires と wants の両方に入れられません
plugin-capability-provider-requires = { $capability } はこのプラグイン自身が提供している能力ではありません：能力を提供するプラグインは自分も提供している能力しか requires できません
plugin-capability-port-zero = provides.{ $capability }.port は 0 にできません
plugin-capability-credentials-required = provides.{ $capability } には credentials の宣言が必要です。利用者ごとに専用の資格情報を発行するためです
plugin-capability-credentials-forbidden = provides.{ $capability } には credentials を宣言できません。この能力の提供元はコアが転送した要求だけを受け付けます
plugin-capability-credentials-argv = provides.{ $capability }.credentials.{ $field } は空でない argv で、{ $placeholder } で資格情報名を指し示す必要があります
plugin-capability-service-unknown = { $field } が参照するサービス `{ $service }` が compose ファイルにありません
plugin-capability-service-not-forwardable = { $field } は network_mode: none のサービスを指せません。コアが転送し、そこで資格情報を発行するためです
plugin-install-capability-deps-unsupported = このコアはプラグイン間の依存関係にまだ対応していません。コアを更新してからインストールしてください
plugin-install-runtime-services = runtime が { $runtime } の場合、services は宣言できません
plugin-install-runtime-unsupported = このコアは runtime が { $runtime } のプラグインのインストールや更新に対応していません。先にコアを更新してください
plugin-install-matrix-rtc-needs-matrix = コアが Matrix サーバーに接続していないため、matrix_rtc（通話の認可）を提供するアプリはインストールできません
plugin-install-matrix-rtc-conflict = matrix_rtc（通話の認可）を提供するアプリ { $id } が既にインストールされています。同時に1つだけです
plugin-install-matrix-rtc-ca-failed = 通話の認可アプリが信頼する証明書バンドルを書き込めませんでした：{ $error }
plugin-install-capability-dropped = ホストネットワークのサービスでは、ホームサーバーがデフォルトで{ $capability }を外します。イメージがICMP pingなどで必要とするなら、{ $permission }権限を宣言し、サービス{ $service }に{ $field }を書いてください。

# 静态网页和托管形态（ADR-0080）
plugin-install-static-dir = `{ $root }` の静的ファイル：{ $problem }
plugin-install-static-too-many = ファイルとフォルダーが { $max } 個を超えています
plugin-install-static-too-big = 合計 { $mib } MiB で、上限の { $max } MiB を超えています
plugin-install-static-file-too-big = `{ $path }` が1ファイルの上限 { $max } MiB を超えています
plugin-install-static-hidden = `{ $path }`：. で始まる名前は使えません
plugin-install-static-reserved = _wonton/ フォルダーはアプリのデータ用に予約されています
plugin-install-static-no-index = index.html がありません
plugin-install-static-no-fallback = fallback に指定された `{ $path }` がフォルダーにありません
plugin-install-static-symlink = `{ $path }` はシンボリックリンクです。静的ファイルにシンボリックリンクは使えません
plugin-install-static-not-file = `{ $path }` は通常のファイルではありません
plugin-install-static-missing = フォルダーがありません
plugin-install-static-read-failed = `{ $path }` を読み込めません：{ $error }
plugin-hosted-provider-not-running = このアプリをホストしている { $provider } が実行されていません
plugin-hosted-provider-missing = このアプリに必要な { $capability } を提供するアプリがインストールされていません
plugin-hosted-provider-taken = { $other } がすでに { $capability } を提供しています。提供できるアプリは1つだけです
plugin-hosted-provider-in-use = 次のアプリはこのアプリがホストしており、アンインストールすると開けなくなります：{ $plugins }。先にそれらをアンインストールしてください
plugin-hosted-dir-missing = このアプリの { $capability } のファイルが見つかりません。再インストールしてください
plugin-hosted-storage-failed = ホストするファイルを書き込めません：{ $error }
plugin-hosted-data-missing = このアプリの { $capability } に保存したデータが見つかりません。再インストールすると空の状態から始まります

plugin-recovery-interrupted = アプリの起動または再構築が完了しませんでした。コアは既存のコンテナーを確認してから再試行します
plugin-recovery-owner-stopped = このアプリは明示的に停止されています。続けるには起動してください
plugin-recovery-not-due = このアプリは次の復旧時刻に達していないか、自動再試行の上限に達しています
plugin-recovery-stop-interrupted = アプリの停止が完了しませんでした。自動起動は無効になっています
