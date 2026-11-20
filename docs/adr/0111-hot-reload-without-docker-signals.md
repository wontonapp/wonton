# ADR-0111 热加载不经 Docker 的 kill 接口

- 状态：已接受（2026-11-20）
- 相关：[ADR-0050](0050-server-pairing.md)、[ADR-0051](0051-private-home-federation.md)、[ADR-0021](0021-entry-containers.md)

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
  `services.config.reload`（`src/admin/server/reload_config.rs`）。管理房间 `#admins:<server>` 由 Tuwunel 在
  `admin_room = true` 时建好，进房间只能靠 `/_synapse/admin/v1/register` 的 `admin: true`（`make_user_admin`
  邀请），所以核心注册一个隐藏账号 `wonton-reload` 并加入，之后每次登录一次发命令（设备名固定，便于在设备列表里认）。
  账号要管理员是 Tuwunel 的硬要求，JWT 登录建号不会被邀请。
- 入口 HAProxy：主进程的管理端口（`-S`，只在内网）上发 `reload`。master 会按同一份 argv 起新进程做预检，起得来
  才切，回应 `Success=1`；起不来回应 `Success=0` 加启动日志，原样进错误。没有文件监视（人工的 `SIGUSR2` 外部入口还在，
  核心不再用）。

两边都保留“配置在文件里”，命令只负责“重读”，不把配置塞进请求。

### 容器重启策略与停止信号仍要兜底

热加载不再碰容器，但 Docker 重启、宿主重启、容器崩溃还要自己回来：Tuwunel 在 compose 里、入口和连接器在核心的
创建请求里都是 `restart: always`，停止信号显式写 `SIGTERM`（不写时 Docker 把任何信号都当停止，热加载过后容器一崩
就不再拉起）。`always` 不看手动停过的标记：手工 `docker stop` 的容器在宿主重启后也会被拉起，这是可接受的代价。

## 理由

- 两条路径都是上游自己的机制，不引额外容器、不读服务器数据、不 exec 进镜像（Tuwunel 镜像没有 shell）。
- 预检由上游完成：HAProxy 新进程起不来时旧进程不受影响，错误带完整启动日志；Tuwunel 的重读失败也返回原错误。
- 保留 `SIGUSR1`/`SIGUSR2` 作为人工入口，但核心不再依赖它们，`restart: always` 让任何一次信号都不再造成不回来。

## 代价与待验证项

- 管理房间依赖 `admin_room = true`：升级时 `core init` 重写 Tuwunel 配置就会带上它；直接拿旧配置启动（没跑 `init`）
  的核心在连接阶段就报找不到房间。
- `wonton-reload` 是第二个本地管理员，能发所有 `!admin` 命令。令牌只在核心进程内存里，每次启动登录一次；Tuwunel
  按登录分配设备号，旧设备不主动注销，会慢慢攒下失效令牌。
- 入口主进程的管理端口只接在 `clients` 网络上，核心不在容器里时热加载连不上（与入口容器本身相同的前提）。
- 契约测试 `restart_contract_tests` 扫生产代码里的 `kill_container(` 和对应容器的重启策略，再发信号时测试会失败，
  提醒按本篇取舍。
