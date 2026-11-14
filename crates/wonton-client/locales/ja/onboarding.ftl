# $error はJSONパーサーの元のエラーです。
home-ticket-invalid-json = 接続情報が有効なJSONではありません：{ $error }

# 引数なし。スキャンした接続情報にサーバーへ到達できるアドレスがありません。
home-ticket-no-addresses = 接続情報にサーバーのリレーアドレスも直接接続アドレスもありません

home-ticket-relays-invalid = リレーテーブルが有効ではありません：{ $reason }

# $domain は接続情報に書かれた元のベースドメインです。
home-ticket-invalid-domain = ベースドメインが無効です：{ $domain }

# $error はURLパーサーの元のエラーです。
home-ticket-invalid-web-url = アプリのウェブアドレスが無効です：{ $error }

# $domain は接続情報のベースドメイン、$url は接続情報に書かれた元のアドレスです。
home-ticket-web-url-under-domain = アプリのウェブアドレスは{ $domain }の下のhttp(s)アドレスである必要があります：{ $url }

# $error はURLパーサーの元のエラーです。
home-ticket-invalid-core-url = 自宅サーバーのアドレスが無効です：{ $error }

# $domain は接続情報のベースドメイン、$url は接続情報に書かれた元のアドレスです。
home-ticket-core-url-under-domain = 自宅サーバーのアドレスは{ $domain }のhttp(s)アドレスである必要があります：{ $url }

# $error はネットワーク設定の元のエラーです。
# 引数なし。ペアリング画面のデバイス名欄が空です。
home-pairing-name-empty = デバイス名が空です

home-pairing-bind-failed = ネットワーク接続を確立できません：{ $error }

# $seconds はバインドの制限秒数。本機のネットワークが時間内に準備できず、自動で再試行します。
home-bind-timeout = 端末のネットワークが { $seconds } 秒以内に準備できませんでした。

# $reason は接続情報の解析の元のエラーです。
home-ticket-corrupt = 保存された接続情報が壊れています：{ $reason }

home-no-connection-info = 自宅サーバーの接続情報がまだありません

# 端末内の接続準備の失敗。元のエラーは診断情報に分けて表示します。
home-proxy-start-failed = アプリのローカル接続プロキシを起動できませんでした。もう一度お試しください。

home-tunnel-closed = トンネルクライアントが閉じられました

home-lan-unsupported = このバージョンのクライアントコアはローカルネットワーク設定に対応していません

home-port-mapping-save-failed = ポートマッピングの設定を保存できませんでした：{ $error }

# 接続のローカルストレージ（crates/wonton-client/src/home/storage.rs）。
# $path はファイルのパス、$reason は元のパースまたは入出力エラーです。
home-storage-device-key-corrupt = デバイスキーのファイル{ $path }が破損しています：{ $reason }

home-storage-io-failed = { $path }を読み書きできません：{ $reason }

# ペアリング監視の制限時間。$seconds は処理全体の監視時間で、交換の各段階の制限時間とは異なります。
home-pairing-timeout = { $seconds } 秒以内にペアリングを完了できませんでした。

home-pairing-aborted = ペアリングが完了する前に中断しました。もう一度お試しください。

home-pairing-not-pairing = 進行中のペアリングはありません。

# 読みやすい詳細。$reason は翻訳済み。elapsed は実際の合計時間、stage は交換の各段階の制限時間で、どちらも秒です。
home-pairing-failed-detail = { $reason }合計経過時間は { $elapsed_seconds } 秒です。ペアリング交換の各段階の制限時間は { $stage_seconds } 秒です。

home-pairing-joining-pending = 参加用のコードです。管理者の承認を待っています。
home-pairing-joining-pending-role = 参加用のコードです。管理者の承認を待っています。承認されると { $role } として参加します。

# 失敗の種類だけを表示し、元のエラーは診断情報に残します。
home-failure-transport = 自宅サーバーへの接続を確立できないか、接続が切断されました。端末とサーバーのネットワークを確認して再試行してください。
home-failure-unauthorized = 自宅サーバーがこのデバイスを拒否しました。承認済みのデバイスまたはサーバーから再度ペアリングしてください。
home-failure-upstream = 自宅サーバーには接続できましたが、必要なサービスの準備ができていません。しばらくしてから再試行してください。
home-failure-protocol = アプリと自宅サーバーの接続プロトコルが一致しません。両方のアップデートを確認してください。
home-network-setup-failed = アプリが端末のネットワーク接続を準備できませんでした。自動的に再試行します。
home-pairing-network-setup-failed = アプリがペアリングに必要な端末のネットワーク接続を準備できませんでした。もう一度お試しください。
home-proxy-route-failed = アプリのローカル接続経路を登録できませんでした。もう一度お試しください。

# ユーザーが開いたコピー可能な診断情報のみ。ペアリングキーとリレートークンは伏せます。
home-tunnel-diagnostic = エラーコード：{ $code }。元のエラー：{ $error }
home-diagnostic-unavailable = 保存された接続情報を安全に読み取れないため、診断情報を表示できません。
home-pairing-diagnostic = 直接接続先：{ $addresses }。リレー：[{ $relays }]。{ $error }

# ペアリングコードの検証。スキャン内容やキーは含めません。
home-code-not-pairing = これは Wonton のペアリングコードではありません。自宅サーバーのコードをスキャンしてください。
home-code-unsupported-version = このペアリングコードの形式を認識できません。アプリを更新して再試行してください。
home-code-invalid-version = ペアリングコードに形式のバージョンがありません。新しいコードを取得して再試行してください。
home-code-invalid-address = ペアリングコードのサーバー接続情報がないか無効です。新しいコードを取得して再試行してください。
home-code-invalid-relay = ペアリングコードのリレー情報が無効です。リレーアドレスには HTTPS が必要です。新しいコードを取得して再試行してください。
home-code-invalid-domain = ペアリングコードのサーバーアドレスが無効です。新しいコードを取得して再試行してください。
home-code-invalid-secret = ペアリングコードのペアリングキーがないか無効です。新しいコードを取得して再試行してください。
home-code-invalid = ペアリングコードの接続情報が無効です。新しいコードを取得して再試行してください。
home-code-rejected = サーバーはこのペアリングコードを受け付けませんでした。新しいコードを取得して再試行してください。
home-code-expired = このペアリングコードの有効期限が切れています。新しいコードを取得して再試行してください。
home-code-used = このペアリングコードは使用済みか無効になっています。新しいコードを取得して再試行してください。
home-code-closed = サーバーはこの種類のペアリングコードを受け付けなくなりました。承認済みのデバイスまたはサーバーから追加してください。
home-code-request-invalid = サーバーがペアリング要求を処理できませんでした。アプリとサーバーのアップデートを確認してください。
home-code-server-error = サーバーがペアリングを完了できませんでした。しばらくしてから再試行してください。
