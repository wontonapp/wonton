# ADR-0075 图片处理：常驻助手进程代替核心现起子进程

- 状态：已接受（2026-11-11）

## 背景

核心把插件图标、截图、相册缩略图的解码和渲染放在限了内存和 CPU 时间的子进程里（ADR-0017），子进程由核心进程直接启动。

iroh 的 netwatch 在宿主网络大变时在原端口重绑 UDP 套接字。`fork` 到 `exec` 之间，子进程手里有父进程全部
fd 的副本（CLOEXEC 要到 exec 才生效），包括隧道的 UDP 套接字；正好在这时重绑就 `AddrInUse`。

iroh 1.4.1、netwatch 0.19.3 上实测：两个线程不停起子进程时 8880 次重绑里 337 次失败，不起子进程 9738 次
0 失败。重绑失败后 netwatch 停在 `Closed`、`is_broken` 仍是 false，`maybe_rebind()` 只在 `is_broken`
为真时重试，所以子进程退出后也不会自己绑回来：带中继的 endpoint 直连丢掉后一直走中继、地址接口还报旧的
直连地址；只有 IP 传输的 endpoint 约 0.2 毫秒内连续收包失败，整个关掉。`Endpoint::network_change()`
救不了，同样的网络状态被 netwatch 去重。上游同类问题见 n0-computer/iroh#4656，最新版本和 main 都没修。

## 决策

核心启动时、绑隧道之前先起一个常驻的图片助手（同一个二进制的 `image-host` 子命令）。之后的图片任务经
管道交给助手，由它再起限了内存、CPU 时间和墙钟时间的子进程。助手手里没有隧道的 fd，它 fork 多少次都
不影响重绑。

- 助手和核心用带协议版本的文本帧说话，核心启动时核对版本。
- 助手退出时核心如实报“图片处理不可用”，下一个任务前再起一个。
- 任务 future 被丢掉时（缩略图超时就是这样取消）核心向助手发取消帧，助手杀掉对应的子进程；助手
  自己也限同样的并发，取消帧晚到也不会把子进程堆起来。
- 助手和子进程都设 `PR_SET_PDEATHSIG`，对方被杀时跟着退出；处理图片的资源上限、结果格式、语言和
  错误类别都不变。

## 理由

- 只挪走 fork 这一件事，不碰 iroh 和 netwatch，也不改图片处理本身的约束。
- 比把图片处理挪进独立容器轻：少一个容器的生命周期和 RPC；助手仍是核心镜像里的同一个二进制。
- 助手在隧道绑定之前起好，网络平稳运行时核心完全不再 fork。

## 代价与风险

- 助手本身退出后的重起仍由核心 fork，那一下会和重绑撞窗口；助手退出是少数情况，正常启动后不再退出。
- 管道的帧协议是内部的，核心和助手版本对不上时报“不可用”而不是静默降级；两者在同一个镜像里一起升级。
- 助手进程多占一点常驻内存，一次任务的输入在核心和助手各留一份。

## 没选的方案

- 子进程退出后调 `Endpoint::network_change()`、看 `bound_sockets()` 是否为空：前者被 netwatch 去重，
  后者不清空。
- 互斥“起子进程”和“插件网络操作”：只缩窗口，其他容器的网络变化核心锁不住。
- 给 netwatch 打补丁（`SO_REUSEADDR` 加失败后退避重试）：要维护依赖补丁；只设 `is_broken` 会热循环，
  仍会杀掉只有 IP 传输的驱动。
- 图片处理放进独立容器：能消掉 fork，但多一个容器的生命周期和 RPC。
- 用 `posix_spawn` 或 vfork：posix_spawn 也照样复制 fd 表，CLOEXEC 到 exec 才生效。

## 验证结果

行为测试在 `crates/wonton-core/tests/image_worker.rs`（Linux 主机，`cargo test -p wonton-core --test image_worker`，2026-11-11，6/6 通过）：

- `rebinding_the_tunnel_port_works_while_an_image_is_processed`：测试进程绑一个 UDP 端口并清掉 CLOEXEC
  （真隧道的套接字是 CLOEXEC，清掉把 fork 到 exec 的窗口撑成整个任务期间），起任务 200 毫秒后把端口
  放掉再绑回同一端口。临时把实现改回“核心直接起子进程”时，这个测试报 `AddrInUse`；换回助手后通过。
- `slow_images_are_stopped_at_the_deadline_and_the_process_is_gone`：到点只杀处理图片的子进程，助手还在，
  能接着处理下一张。
- `a_dead_helper_is_started_again`：SIGKILL 助手后，下一个任务照常处理。
- `a_cancelled_task_kills_its_child`：丢掉任务 future 后处理图片的子进程停掉，助手还在、能接着用。
