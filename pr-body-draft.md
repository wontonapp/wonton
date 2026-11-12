# PR 描述（最终版）

## 改了什么、为什么

CI run 42222286966（job 127907019338，CI 机器）里 `stall::revalidating_a_proven_slow_path_keeps_the_connection` 在 support.rs:260 报「等待响应超时: Elapsed(())」：GET /after 经本地代理 15 秒（WAIT）没读完。#519 只改插件健康检查，与隧道无关；同套 33 条其余 32 条都过。

### 根因（本地实测）

1. **复现**：main（6146e72）上只编 wonton-tunnel，测试钉在 24-27 号核、nice -n 19、同核 4 个空转负载。两种表现都复现：
   - 与 CI 相同（support.rs:260）：`num_conns_opened==1` 通过后 GET 卡死（约 1/12）；
   - 兄弟表现（num_conns 2≠1）：revalidate 的 8 秒 ping 没回来，客户端按 ADR-0032 换连接。
2. **包级定位**（iroh=trace + noq_proto=trace + 测试内插桩）：失败那次，手机 14:07:06.514 把 GET 请求包发出（iroh 的日志在 sendto 之后打），服务端 14:07:17.749 才在重传包（pn=166）里收到 STREAM；原始包（pn=144）从未到达。服务端立刻回响应，panic（15 秒超时）前手机没等到。QUIC 层全程健康：无拥塞门（只有毫秒级 pacing）、无 PTO 过期、连接活着、num_conns==1。测试内插桩显示测试的 tokio runtime（单线程）被 OS 饿住：心跳探针 14 次 >200ms 停顿、最长 6.7 秒；Flaky 转发任务 21 次 >200ms 没被轮询，两个方向各停摆约 9.5 秒。每包 900ms 的假延迟让每个包都要在饿住的 runtime 上睡一觉再发，往返从 1.8 秒被拉到 8-11 秒。
3. **结论**：stall 组的时限全是墙钟（probe 的 1.5 秒证据线、revalidate 的 8 秒窗、support 的 15 秒 WAIT），而两端 endpoint、本地代理、上游、Flaky 转发全在测试的单线程 runtime 上。CI runner（与开发同宿主）高负载时这颗核被占住几秒，所有任务停摆，一次往返拖过时限。产品的两个表现都符合 ADR-0032：该保的保住（num_conns==1）、该换的换（8 秒无回应）。**是测试的假设（runtime 不会被饿几秒）不成立，不是产品缺陷；trace 也证明不是 iroh 的 PTO/拥塞问题。**

### 修法（三层，断言、时限、重试一概未动）

1. **stall 组 6 条测试改 `multi_thread` runtime（worker_threads=4）**：只要还有一个 worker 被调度，驱动就前进；全套并发时线程数可控。
2. **Flaky 转发挪到专用线程自己的 runtime**：转发精度不再依赖测试 runtime 的忙碌程度（往返被拉长的最大一项）；Drop 用哑包唤醒 + 停止标记干净退出。
3. **revalidate 的窗口决策改用钩子断言，端到端行为在撤掉假延迟后验证**：
   - client.rs 加 `#[doc(hidden)] pub fn next_revalidate_window()`：若现在 revalidate 会用多长的确认窗口。它只取决于连接、路径和探测证据，与机器负载无关——「慢路径的证据是否改变了决策」由此变成确定性断言。
   - 两条慢路径测试改为：900ms 假延迟只负责让两次 probe 记下慢证据（每趟 ≥1.8 秒，只会更慢不会更快）→ 断言 `next_revalidate_window() == 8s`（#405 的决策）→ 撤掉假延迟 → revalidate / GET 的行为断言（连接数 1、请求走完、首包丢了靠重传走完且不换连接）在毫秒级往返下进行，不受调度饥饿影响。
   - 不撤假延迟直接断言 num_conns==1 的话，8 秒窗本身可能被极端饥饿拖爆（实测宿主负载 118、nice-0 编译抢核时约 1%），端到端断言就还是靠墙钟。决策断言与行为断言分开后，两者都确定性成立。

## 验证（命令和结果）

cargo 全部经 `~/wonton-dev/cargo-slot.sh`；复现与 500 次验证钉 24-27 号核、nice -n 19、同核 4 个空转负载（宿主当时负载 70-118，含其他 agent 的 nice-0 编译进程，比 CI 条件更狠）。

- 复现：改前 exact 模式 12 跑 1 败；同款 support.rs:260 在 37 跑时出现（trace 已定位到包级证据）。中间形态（只改多线程 runtime）150 跑 15 败、再加专用转发线程 500 跑 6 败——据此继续把决策断言与墙钟脱钩。
- 改后全套：`cargo test -p wonton-tunnel --test tunnel` 34/34 通过（多轮）。
- 回归测试 500 次：**500 过 0 败**（钉核 + 空转负载）。
- 既有慢路径保持连接测试 500 次：[待填]。
- #405 行为回退（revalidation_window 临时固定回 1.5 秒，补丁不入库）500 次：[待填]（窗口断言应 500/500 失败）。
- fmt --check、clippy -D warnings、check-file-size：通过（2026-11-06 23:35 后用 CARGO_SLOT_LOCAL=1 在本机复核，见下）。

### 关于 2026-11-06 21:55–23:05 cargo-slot 远端分发的说明

- 本任务的测试执行循环（复现 repro-stall.sh、500 次验证 verify-500.sh、全套 suite）**全部是直接在本机 taskset+nice 跑测试二进制**，不经 cargo-slot，不受远端分发影响。
- 经 cargo-slot 跑的只有构建与门禁：`cargo build`（非 test|clippy|check 动词，不分发）、`cargo test --no-run`（明确不分发）、`cargo fmt`（不分发）、`cargo clippy`。cargo-slot-remote.log 里本任务目录零记录；且远端分发功能 23:30 才合入，我的 clippy 在 23:11 就跑完了。为稳妥，所有门禁已用 `CARGO_SLOT_LOCAL=1` 在本机重跑，结果见上。

## 没做的和原因

- 没改产品：两个失败表现里产品行为都符合 ADR-0032。不加长超时、不加请求时限、不加重试。
- `next_revalidate_window` 是 `#[doc(hidden)]` 的测试钩子：把 ADR-0032 的窗口决策暴露给测试做确定性断言。若审查不收这个口子，替代方案是测试里复刻决策表，但那会和实现漂移，不如钩子诚实。
- 同套 `forwarding::client_recovers_after_server_crash` 在本地高负载下也偶发同款 15 秒超时（不经过 stall rig，本次改动不涉及它）。没有顺带修：超出本任务范围，先报告。
- 未在真实 CI runner 重放原始 job：本地以同宿主 + 钉核 + 空转负载近似，两种失败表现均已复现并定位到包级证据。

## 底座的问题

- 无插件底座问题（纯主仓库测试）。两条备查：
  - CI 与开发共享宿主、负载常态 70-118：靠墙钟时限的集成测试天然受邻居影响。stall 组这类建议以后默认多线程 runtime；`forwarding::client_recovers_after_server_crash` 有同款隐患。
  - 本机多次出现 cargo 输出的 crate 路径是 `<别的工作副本路径>`（该目录在本机不存在），而实际工作目录是 `<本机路径>`（inode 已核对一致）。发生时间 23:11，早于远端分发功能合入（23:30），与本次远端分发无关。怀疑共享机上某个会话/临时 shell 的 cwd 指向别的 agent 的同名任务目录。已用 `--manifest-path` 显式钉住重跑确认。建议查来源——别的 agent 踩到会把验证结论安到错误的工作副本上。
