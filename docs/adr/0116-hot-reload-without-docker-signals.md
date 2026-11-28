# ADR-0116 热加载不经 Docker 的 kill 接口

- 状态：已接受（2026-11-20）
- 相关：[ADR-0050](0050-cross-server-chat.md)、[ADR-0051](0051-friends-and-plugin-sharing.md)、[ADR-0021](0021-third-party-clients.md)

## 背景

Tuwunel 的联邦名单热加载和入口证书续期原来都是 `docker kill --signal`：Tuwunel 发 `SIGUSR1` 让配置重读，入口
HAProxy 发 `SIGUSR2` 让 master 软重载。`systemctl restart docker` 之后两个容器都不回来，核心连不上 Matrix
每 100 秒崩一次。

根因在 Docker（moby `daemon/kill.go` 的 `killWithSignal`）：经 kill 接口发的任何信号都当停止处理，非关机时把容器
记成 `HasBeenManuallyStopped` 并调 `RestartManager().Cancel()`；`unless-stopped` 的容器在守护进程重启后看这个标记，
不再拉起（`restartmanager.go` 的 `shouldRestart`）。只有 `docker restart` 这类带重启意图的调用才不置标记。容器没被
停，标记却在，下一个信号又是一次停止。

## 决策

### 两个上游自己的热加载入口，不再让 Docker 转信号

- Tuwunel：在服务器自己的管理房间里发 `!admin server reload-config`。它和 `SIGUSR1` 走同一条
  `services.config.reload`（`src/admin/server/reload_config.rs`）。管理房间 `#admins:<server>` 由 Tuwunel 在空库首次
  启动时建好（`create_admin_room`，默认开），带 `admin: true` 注册的账号由 `make_user_admin` 邀请进去。核心的管理账号
  （[ADR-0009](0009-identity-pairing-sso.md)）本来就是带 `admin: true` 注册的，手里有现成邀请；连上时加入一次，之后发命令都用它。不再另开管理员。
- 入口 HAProxy：主进程的管理端口（`-S`，只在内网）上发 `reload`。master 会按同一份 argv 起新进程做预检，起得来
  才切，回应 `Success=1`；起不来回应 `Success=0` 加启动日志，原样进错误。容器刚起来、master 还在做初始装载时会先回
  `Success=0` / `Another reload is still in progress`（3.4-alpine 实测），核心在 10 秒内隔一小段重发。没有文件监视
  （人工的 `SIGUSR2` 外部入口还在，核心不再用）。镜像跟着从 3.2.8-alpine 换到 3.4-alpine（官方摘要固定）：
  3.2.8 的 alpine 构建里 `-S` 直接打 Usage。

两边都保留“配置在文件里”，命令只负责“重读”，不把配置塞进请求。

### 容器重启策略与停止信号仍要兜底

热加载不再碰容器，但 Docker 重启、宿主重启、容器崩溃还要自己回来：Tuwunel 在 compose 里、入口和连接器在核心的
创建请求里都是 `restart: always`，停止信号显式写 `SIGTERM`（不写时 Docker 把任何信号都当停止，热加载过后容器一崩
就不再拉起）。`always` 不看手动停过的标记：手工 `docker stop` 的容器在宿主重启后也会被拉起，这是可接受的代价。

## 理由

- 两条路径都是上游自己的机制，不引额外容器、不读服务器数据、不 exec 进镜像（Tuwunel 镜像没有 shell）。
- 管理账号是现成的：令牌就是核心的 `matrix-admin-token`（0600，只在核心数据目录），不挂给插件和 Bot，不用第二份凭据。
- 预检由上游完成：HAProxy 新进程起不来时旧进程不受影响，错误带完整启动日志；Tuwunel 的重读失败也返回原错误。
- 保留 `SIGUSR1`/`SIGUSR2` 作为人工入口，但核心不再依赖它们，`restart: always` 让任何一次信号都不再造成不回来。

## 代价与待验证项

- 管理房间依赖 `create_admin_room`（默认开）和核心管理账号仍在房间里：安装时的空库已经建过房间并邀请过核心管理账号；
  被动离开房间时连接阶段报错，热加载跟着失败，不静默降级。
- 管理房间只在空库首次启动时建：从别处搬来的旧库若从没建过管理房间，`core init` 重写配置也补不出房间，除非 Tuwunel
  另建。安装脚本、开发栈和测试都按默认值走。
- 核心管理账号是本地管理员，本来就能发所有 `!admin` 命令，热加载没有为它新增权限。凭据就是宿主数据目录里 core 的
  `matrix-admin-token`（0600）：只有宿主 root 和核心进程读得到，不挂进 Tuwunel、插件或 Bot。
- 入口主进程的管理端口只接在 `clients` 网络上，核心不在容器里时热加载连不上（与入口容器本身相同的前提）。
- 契约测试 `restart_contract_tests` 扫生产代码里的 `kill_container(` 和对应容器的重启策略，再发信号时测试会失败，
  提醒按本篇取舍。

## 验证结果

红（开发机，cargo 经 `cargo-slot.sh`）：

- 入口建容器改回 `unless-stopped`、删 `stop_signal`：`restart_contract_tests::core_created_containers_come_back_after_a_docker_restart`
  报 “入口容器的 restart_policy 只许是 ALWAYS”。
- 已有 `entry/settings.json`、`init.env` 又带 `WONTON_MATRIX_NAME_MODE=none` 的重复安装，之前被
  `EntrySettings::select` 拒掉；补了单测 `saved_settings_survive_a_repeated_none_mode_install`（覆盖这条）。
- `reload_config` 改成直接 `Ok(())`：`admin_reload_tests` 两条都红（发出 0 条命令；应当 Reload 失败却成功）。
- 早一轮取证：生产代码里加 `kill_container(` 时 `no_production_code_signals_containers` 报
  `crates/wonton-core/src/clients/domain/issue.rs:169`、`federation_config/mod.rs:116`，`a_signalled_container_comes_back_after_a_docker_restart` 报 compose
  里 `unless-stopped`。

虚拟机（amd64 qemu + Docker 29.7.2，自己域名 + 外部自签证书）：

- 本分支镜像从零安装：`check.sh restart` 两次、`check.sh stop-start` 一次均 `通过`；Tuwunel 和入口的 PID 1 各
  `kill -9` 一次，都被 `always` 拉起（`restarts=1`，状态 running）。
- 整套 `cargo test -p wonton-core --lib`：2846 通过；唯一失败是 `device_recovery` 的假 Docker 绑
  `127.0.0.1:0` 报 `AddrInUse`（机器临时端口用尽），单跑通过。
- 真 Tuwunel 的 Docker 集成测试：`the_real_server_reloads_the_admin_room_command` 和
  `the_real_server_hot_reloads_the_allowlist_from_the_second_file` 都通过（管理房间命令、名单生效）。
- 旧 `stable` 镜像先装、配对、用 Matrix 客户端接口同步并发一条消息（200），再升级到本分支：同一设备仍能同步和发送（200），`check.sh restart` 绿。
- origin/main 的二进制同样安装：`check.sh restart` 在 360 秒后报 `不通过`，`wonton-tuwunel-1` 保持 `exited`、
  `manually_stopped=true`，核心重启 3 次仍在崩溃循环。
