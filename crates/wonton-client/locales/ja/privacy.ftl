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
