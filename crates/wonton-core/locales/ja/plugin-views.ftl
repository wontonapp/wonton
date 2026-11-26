## 自宅サーバーがアプリに送るテキスト（ADR-0043）。完全な文を 1 メッセージにし、コードでの組み立てはしない。
## データ（パス、ポート、デバイス名、エラーコード）はパラメーターとして文に埋め込み、翻訳しない。
## 説明はです・ます調。アプリのことは「アプリ」と書き、日英の間にスペースを入れない。

# データ項目（イメージの取得元など）を並べる区切り文字。
list-separator = 、

# 権限：インストール確認、ボットの確認カード、ストアの検索結果に表示する。
permission-storage-share =
    { $access ->
        [read] 共有フォルダ{ $name }を読み取ります
       *[write] 共有フォルダ{ $name }を読み書きします
    }
permission-storage-home =
    { $access ->
        [read] あなたのプライベートスペースを読み取ります
       *[write] あなたのプライベートスペースを読み書きします
    }
permission-models = 接続済みのモデルを使います
permission-tools = 自宅サーバーやほかのアプリが提供する小さなツールを使います
permission-plugins-health = すべてのアプリが正常かどうかを見ます
permission-matrix-bot = チャットボットとしてメッセージを送受信します
permission-notify = 家族に通知とメールを送ります
permission-owner-identity = あなたのユーザー名とメールアドレス、家族が使っている言語を取得します
permission-network-egress = インターネットとローカルネットワークにアクセスします
permission-egress-proxy = ほかのアプリのインターネット通信を中継します（高権限）
permission-network-lan = 自宅のデバイスから直接接続させます（高権限）
permission-network-clients = あなたがスイッチを入れると、ほかのアプリが自宅のネットワークやインターネットから接続できます
permission-network-host = 自宅サーバー自身のネットワークを使います（非常に高い権限）
permission-capability = システム機能{ $name }を追加します
permission-host-pid = サーバー上で動いているすべてのプログラムを見ます（最高権限）
permission-host-ipc = サーバー上のプログラムと直接データをやり取りします（最高権限）
permission-security-opt = そのセキュリティ設定を変えます（最高権限）
permission-privileged = 最大の権限で実行します（最高権限）
permission-devices-gpu = グラフィックカードを使います（高権限）
permission-device = サーバーのデバイス{ $path }を使います（高権限）
permission-devices-serial = サーバーのシリアルデバイスを使います（高権限）
permission-devices-usb = サーバーの USB デバイスを使います（高権限）
permission-devices-disk = 選んだディスク全体を読み書きします（とても高い権限）
permission-docker-socket = Docker を管理します（最高権限）

# 高い権限の下に付く詳しい説明。
permission-notice-egress-proxy = これをプロキシとして選ぶと、ほかのアプリのインターネット通信はすべてこれを経由します。どのサイトにアクセスしたかを知られ、暗号化されていない内容は書き換えられます。
permission-notice-network-host = このアプリはサーバー自身と同じようにインターネットに出られ、サーバー専用のサービスも使え、Wonton のログインを経ずに一部のほかのアプリへも接続できます。開いたポートはすべてのネットワークアドレスから接続でき、自宅のネットワークからも、サーバーにグローバルアドレスがあればインターネットからも届きます。サーバーでファイアウォールが有効な場合、自宅のデバイスは許可するまで接続できません。
permission-notice-highest-risk = サーバーの最高の制御を渡すのと同じです。すべてのファイルとほかのアプリのデータを読み書きし、Wonton 自身を変更したり止めたりできます。信頼できるアプリだけを入れてください。アンインストールしても、作られたコンテナや変更されたものは片付きません。
permission-notice-network-lan = 自宅のデバイスは Wonton のログインなしで、このアプリが開いたポートへ直接接続できます。誰を通すかはアプリ自身が決めます。
permission-notice-device = このアプリはサーバーのデバイス{ $path }を使えます。
permission-notice-devices = このアプリは渡したサーバーのデバイスを使えます。
permission-notice-devices-disk = このアプリは選んだディスク上のすべてのデータを読み書きできます。
permission-notice-devices-usb = このアプリはサーバー上のすべての USB デバイス（USB メモリやカードリーダーも）を使えます。

# アプリ詳細の通知。
plugin-firewall-notice = このマシンのファイアウォールが外部からの接続を遮断している可能性があり、ローカルネットワークのデバイスから接続できないかもしれません
plugin-egress-proxy-select = プロキシの設定でこのアプリを選んでください
plugin-cross-origin-isolation = このアプリはクロスオリジン分離が必要です。Androidでは、そのページはブラウザで開きます。

# インストールと更新のプレビューに表示する通知と、インストールできない理由。
install-retained-data = サーバーには前に{ $id }が使っていたデータとパスワードが残っており、インストールしたアプリはそれらを読めます。望まない場合は先に完全に削除してください
install-image = { $service }（{ $image }）
install-images-from = プログラムの提供元：{ $images }
install-images-unpinned = プログラムが特定のバージョンに固定されていません：{ $images }
install-webui-sso = Wonton でウェブページを開くと自動的にログインします
install-host-network-egress = インターネットへはサーバー自身のネットワークで接続し、Wonton のプロキシを通りません
install-host-network-proxied = インターネットへは Wonton のプロキシも通ります
install-already-installed = { $id }はすでにインストールされています（バージョン{ $version }）
install-unreviewed-provider = Wonton はこのアプリを審査していません。プログラムは{ $provider }が提供しています
install-unreviewed-provider-via = Wonton はこのアプリを審査していません。プログラムは{ $provider }が提供し、{ $host }に並んでいます
install-unreviewed-upload-author = Wonton はこのアプリを審査していません。アップロードされたファイルのもので、マニフェストに書かれた作者は{ $author }ですが、確かめられません
install-unreviewed-upload = Wonton はこのアプリを審査していません。アップロードされたファイルのもので、マニフェストに作者の記載がありません
plugin-widget-high = このアプリのページは、開いたルームのチャット内容を読んだり、あなたの名前でメッセージを送信・取り消したりできます。
plugin-widget-normal = このアプリのページはほかに、通話とデバイス宛てのイベントの送受信、イベントの予約送信、通話サーバーの照会、ログイン用トークンの取得ができます。
