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
permission-models = モデルゲートウェイを使用します
permission-tools = 自宅サーバーやほかのアプリのツールを呼び出します
permission-plugins-health = すべてのアプリの稼働状態を読み取ります
permission-matrix-bot = ボットとしてメッセージを送受信します
permission-notify = 通知を送ります
permission-owner-identity = あなたのユーザー名とメールアドレス、家族が使っている言語を取得します
permission-network-egress = インターネットとローカルネットワークにアクセスします
permission-egress-proxy = プロキシを提供します（高権限）
permission-network-lan = ローカルネットワークにポートを開放します（高権限）
permission-network-host = ホストのネットワークを使用します（非常に高い権限）
permission-capability = カーネル機能{ $name }を追加します
permission-host-pid = ホストのプロセス空間を共有します（最高権限）
permission-host-ipc = ホストのIPCを共有します（最高権限）
permission-security-opt = コンテナのセキュリティオプションを変更します（最高権限）
permission-privileged = 特権モードで実行します（最高権限）
permission-devices-gpu = GPUを使用します（高権限）
permission-device = ホストのデバイス{ $path }にアクセスします（高権限）
permission-docker-socket = Dockerを操作します（最高権限）

# 高い権限の下に付く詳しい説明。
permission-notice-egress-proxy = このアプリをプロキシとして選ぶと、インターネットに接続するほかのアプリの通信はすべてこのアプリを経由します。どのウェブサイトにアクセスしたかが見え、暗号化されていない内容は書き換えられることもあります。
permission-notice-network-host = このアプリは自宅のネットワークとこのマシンのすべてのポートに直接アクセスでき、ローカル専用のサービスも使えます。また、Wontonのログインを経由せずに一部のほかのアプリに接続できます。開いたポートはWi-Fi、Tailscale、IPv6アドレスを含むこのマシンのすべてのネットワークインターフェースに現れ、このマシンにグローバルアドレスがあればインターネットからも接続されます。このマシンでファイアウォールが有効な場合、ローカルネットワークのデバイスからは接続できないため、自分で許可してください。
permission-notice-highest-risk = このアプリはホスト全体を制御できます。すべてのファイルやほかのアプリのデータを読み書きし、Wonton自身を変更したり停止したりできます。これはホストのrootを渡すのと同じです。信頼できるアプリだけをインストールしてください。アンインストールしても、作成されたコンテナや変更された内容は削除されません。
permission-notice-network-lan = ローカルネットワークのデバイスから、Wontonのログインなしでアプリが開放したポートに直接接続できます。認証はアプリ自身が行います。
permission-notice-device = このアプリはホストのデバイス{ $path }にアクセスできます。

# アプリ詳細の通知。
plugin-firewall-notice = このマシンのファイアウォールが外部からの接続を遮断している可能性があり、ローカルネットワークのデバイスから接続できないかもしれません
plugin-egress-proxy-select = プロキシの設定でこのアプリを選んでください
plugin-cross-origin-isolation = このアプリはオリジン間の分離を緩めています。Androidでは、そのページがほかのウェブページとプロセスを共有する可能性があります。

# インストールと更新のプレビューに表示する通知と、インストールできない理由。
install-retained-data = このサーバーには前に{ $id }が使っていたデータとキーが残っており、インストールしたアプリはそれらを読めます。望まない場合は先に完全に削除してください
install-image = { $service }（{ $image }）
install-images-from = イメージの取得元：{ $images }
install-images-unpinned = イメージがダイジェストで固定されていません：{ $images }
install-webui-sso = Wontonでウェブページを開くと自動的にログインします
install-host-network-egress = ホストネットワークのサービスは、Wontonのプロキシを経由せず、ホスト自身の経路でインターネットに接続します
install-system-preinstalled = システムアプリは自宅サーバーにプレインストールされているものに限ります
install-already-installed = { $id }はすでにインストールされています（バージョン{ $version }）
install-unreviewed-provider = Wontonはこのアプリを審査していません。コードとイメージは{ $provider }が提供しています
install-unreviewed-provider-via = Wontonはこのアプリを審査していません。コードとイメージは{ $provider }が提供しており、インデックスは{ $host }のものを使っています
install-unreviewed-upload-author = Wontonはこのアプリを審査していません。アップロードされたファイルのもので、マニフェストに記載された作者は{ $author }ですが、確認できません
install-unreviewed-upload = Wontonはこのアプリを審査していません。アップロードされたファイルのもので、マニフェストに作者の記載がありません
