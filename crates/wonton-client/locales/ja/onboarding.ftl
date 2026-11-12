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

# $error はローカルプロキシの元のバインドエラーです。
home-proxy-start-failed = ローカルプロキシを開始できませんでした：{ $error }

home-tunnel-closed = トンネルクライアントが閉じられました

home-lan-unsupported = このバージョンのクライアントコアはローカルネットワーク設定に対応していません

home-port-mapping-save-failed = ポートマッピングの設定を保存できませんでした：{ $error }

# 接続のローカルストレージ（crates/wonton-client/src/home/storage.rs）。
# $path はファイルのパス、$reason は元のパースまたは入出力エラーです。
home-storage-device-key-corrupt = デバイスキーのファイル{ $path }が破損しています：{ $reason }

home-storage-io-failed = { $path }を読み書きできません：{ $reason }

# ペアリングの詳細。アドレスとリレー URL にペアリングキーは含まれません。error は上流の原文です。
home-pairing-timeout = { $seconds } 秒以内にペアリングを完了できませんでした。

home-pairing-aborted = ペアリングが完了する前に中断しました。もう一度お試しください。

home-pairing-not-pairing = 進行中のペアリングはありません。

home-pairing-failed-detail = 直接接続先：{ $addresses }。リレー：[{ $relays }]。経過時間：{ $milliseconds } ミリ秒、制限時間：{ $seconds } 秒。原因：{ $error }
