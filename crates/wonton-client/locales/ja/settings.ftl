# 通知方式の設定（crates/wonton-client/src/push_method.rs）。
# $path は設定ファイルのパス、$reason は元の入出力エラーです。
push-method-write-failed = 通知方式の設定ファイル{ $path }に書き込めません：{ $reason }

# 代理 TUN の偽アドレスを経路選択から外す設定（crates/wonton-client/src/home/path_settings.rs）。
# $path は設定ファイルのパス、$reason は元の入出力エラーです。
path-settings-write-failed = 経路選択の設定ファイル{ $path }に書き込めません：{ $reason }

# プッシュ通知（crates/wonton-client/src/push/{mod,registration,notification}.rs）：拒否されたエンドポイント、失敗した登録、メッセージを取得できなかった場合の詳細情報。
# 通知の本文とプレースホルダー（不明な送信者、招待、[ファイル]など）はまだここにはありません。
# $reason は元のエラーの詳細です。
push-matrix-pusher-failed = Matrixにpusherを登録できません：{ $reason }

# $reason はURLパーサーの元のエラーです。
push-endpoint-invalid-url = プッシュ通知のエンドポイントが有効なURLではありません：{ $reason }

push-endpoint-scheme = プッシュ通知のエンドポイントはhttpまたはhttpsのアドレスである必要があります

push-endpoint-p256dh = p256dhは65バイトの非圧縮P-256公開鍵（base64url）である必要があります

push-endpoint-auth = authは16バイト（base64url）である必要があります

# $reason は上流の原文、または翻訳されたタイムアウトの詳細です。
push-register-failed = 自宅サーバーにプッシュ通知を登録できません：{ $reason }

push-unregister-failed = 自宅サーバーのプッシュ通知の登録を解除できません：{ $reason }

# $reason はJSONパーサーの元のエラーです。
push-register-invalid-response = 自宅サーバーが返した登録結果の形式が正しくありません：{ $reason }

# $reason はローカルプロキシまたはHTTPクライアントの元のエラーです。
push-invalid-proxy = ローカルプロキシのアドレスが無効です：{ $reason }

push-http-client = HTTPクライアントを作成できません：{ $reason }

# $seconds はリクエストのタイムアウト秒数です。
push-timeout = { $seconds }秒以内に応答がありません

# $status はそのままのHTTPステータスコードです。
push-server-status = 自宅サーバーからHTTP { $status }が返されました

# $message と $kind はサーバーの元のエラーテキストとエラーコードです。
push-server-error = { $message }（{ $kind }）

# $reason はJSONパーサーの元のエラーです。
push-payload-invalid = プッシュ通知の内容の形式が正しくありません：{ $reason }

# $reason はIDパーサーの元のエラーです。
push-invalid-room-id = プッシュ通知内のルームIDが無効です：{ $reason }

push-invalid-event-id = プッシュ通知内のイベントIDが無効です：{ $reason }

# $seconds はメッセージの取得で待った秒数です。
push-fetch-timeout = { $seconds }秒以内にこのメッセージを取得できませんでした

# $reason はMatrix SDKの元のエラーです。
push-fetch-failed = このメッセージを取得できません：{ $reason }

# 添付ファイルの通知文（ADR-0118）：種類とファイル名。
push-preview-image = 写真 { $filename }
push-preview-file = ファイル { $filename }

push-core-closed = クライアントコアが終了しました

push-not-paired = まだ自宅サーバーに接続したことがありません

push-home-unreachable = 自宅サーバーに接続できません：{ $reason }

# $seconds はログインで待った秒数です。
push-session-timeout = { $seconds }秒以内にMatrixにログインできませんでした

push-room-key-missing = このメッセージのルームキーがまだ届いていません

# $path is the private device file; $reason is the underlying storage error.
notification-lock-screen-storage-failed = ロック画面の設定ファイル{ $path }に読み書きできません：{ $reason }

# Privacy page (crates/wonton-client/src/about/mod.rs).
privacy-what-title = クラウドなし
privacy-what-own = Wonton に自前のクラウドはありません。サーバーは自宅の（または自分で借りた）マシンで、メッセージも写真もファイルもそこに置かれます。
privacy-what-none = アカウントや端末情報は受け取らず、アプリに解析・広告・クラッシュ報告もありません。

privacy-messages-title = チャットとファイル
privacy-messages-e2ee = チャットとファイルは端末間で暗号化され、開けるのは自分の端末だけです。
privacy-messages-history = 履歴は自分のサーバーに残り、バックアップ鍵は自分で持ちます。

privacy-push-title = 通知（Android のみ）
privacy-push-keepalive = 「接続を保つ」の間は通知が外へ出ません。トンネルの先は自宅です。
privacy-push-unified = UnifiedPush に切り替えると、自宅のコアがまず端末の鍵で通知を暗号化してから配信事業者に渡します。配信事業者に見えるのは暗号文だけで、復号は端末上です。配信事業者は第三者サービスです。

privacy-network-title = 自宅への接続
privacy-network-lan = 同じネットワークなら直接つなぎます。別のネットワークではまず穴あけを試し、だめなときだけ中継を使います。
privacy-network-relay = 中継は暗号化された通信を転送するだけで、中身は見えません。既定は iroh の公開リレーで、自宅側は自分のものに替えられます。

privacy-preview-title = リンクプレビュー
privacy-preview-core = チャットにリンクが出ると、自宅のコアがページのタイトルと画像を取りに行くため、相手サイトには自宅サーバーのアドレスが見えます。

privacy-plugins-title = 外部ログインとプラグインのページ
privacy-plugins-oidc = 別のアカウントでプラグインにログインすると、名前とパスワードはそのプラグインのサーバーにだけ送られ、自宅のコアは外部のパスワードを保存しません。
privacy-plugins-web = プラグインのページは端末のブラウザエンジンで開き、外へ出るかどうかはプラグイン次第です。

privacy-device-title = 端末に残るもの
privacy-device-files = 端末の鍵、接続情報、キャッシュ、ログはアプリの専用ディレクトリにあり、他のアプリからは読めません。アンインストールで消え、サーバーの履歴は残ります。
privacy-device-backup = これらはシステムのバックアップ（iCloud または Google）に含まれ、どのアカウントに置くかはシステムの設定で決まります。

# License list load errors (crates/wonton-client/src/about/mod.rs).
licenses-parse-failed = ライセンス一覧を解析できません：{ $error }
licenses-empty = ライセンス一覧にコンポーネントがありません。
licenses-no-text = ライセンス { $id } に全文がありません。
licenses-none-to-show = 表示できるライセンスが一覧にありません。
