# ADR-0114 实现卡拆分

依赖 `docs/adr/0114-plugin-agent-platform.md` 和 `notes/adr0114-interfaces.md`（路径简写成文件名）。
每张一个 PR，先红后绿；协议卡只改类型、校验和文档，不带行为。

## 0. 实验：工具 `_meta` 能不能原样到插件（先做，决定 7、9、12 的路）

- 目标：跑一个假插件，验 `tools/list` 与 `tools/call` 的 `_meta` 有没有被 rmcp 丢掉。
- 改哪：只用现有代码和一个测试插件，不改产品。
- 依赖：无；**7、9、12 的前置**。
- 大小：S。
- 先红：一条断言“`app.wonton/card` 到了插件进程”的测试。
- 后绿：打印实际到手的键。过不了就换路（`tools/list` 的 `description` 前缀约定，或核心侧工具能力表），结论写进 ADR 待验证第 4 条。
- 并行：最先。

## 1. 协议：`member_accounts` 清单字段与校验

- 目标：清单能声明四条建号命令，坏字段在校验期报错。
- 改哪：新文件 `member_accounts.rs`；`manifest/mod.rs` 加字段；
  `plugin-spec.md` 新节，`wonton-docs` 同批；四条命令都是绝对路径，`status` 可选。
- 依赖：无。
- 大小：M。
- 先红：`manifest_member_accounts_service_must_exist`、`commands_must_be_absolute` 两个测试，各自断言 `Manifest::validate` 的具体错误码。
- 后绿：两处校验都实现；字段序列化往返不丢；`status` 可选、缺省不影响通过。
- 并行：能，和 2、3 之外的任何协议卡并行；2 等它。

## 2. 核心：按人建号的时机、重放和回收

- 目标：安装、授权、移除时正确调插件命令，失败可重试，不吞错。
- 改哪：插件安装完成、成员授权、角色改动、移除成员的流程；`MemberRunner`（新）封装 Docker exec、stdin、退出码；
  容器重建后对缺账号的人重放 `add`。
- 依赖：1（协议）、ADR-0110 的 exec 通道。
- 大小：L。
- 先红：`install_calls_add_for_each_member_with_access`、`revoke_calls_remove`、
  `add_failure_marks_pending_and_retries`、`credential_exit_3_marks_reset`。
- 后绿：按上述行为实现；`add` 幂等，重复安装不重复建；对账只三处触发（安装、容器重建、每天），补建前看成员还在不在。
- 并行：不能，等 1 合入；和其它核心卡（6、11、12）可并行，但要分配不同的文件。

## 3. 客户端账号按人（破坏性）

- 目标：`lan.client` 的详情只回请求者自己那条，口令按（插件、标签、人）存。
- 改哪：`clients/passwords.rs` 的键（加人）、生成时机（有人被授权时）、`GET` 详情接口；**`gateway/clients_entry.rs` 的认证**——现在只比主人用户名
  （`parse_authorization(headers, &owner_username())`）并无条件注入主人的身份头，必须改成先认出是哪个家人、再按那人注入；
  `clients_passwords.rs` 的详情鉴权；升级迁移。
- 依赖：1、2 的命令。核心自己的 `clients/passwords.json` 存的是*本机客户端口令*；`credential` 取的是*上游应用*口令、在插件数据卷，两者别混。
- 大小：M。
- 先红：`detail_lists_only_callers_account`、`migration_copies_owner_password_and_marks_reset`、`non_member_gets_404`。
- 后绿：迁移跑一次；两人同时索要互不串；旧共享口令只复制给主人时，家人继续用旧口令**会按主人身份进来**，说明里写明。
- 并行：能，和 1、4 以外无关；要动 `passwords.rs` 就和 3 以外的人说好。

## 4. 核心：`_meta` 的 `speakers` 与 `room_people`

- 目标：共享 Bot 单人时带说话人，多人时不猜；来源房间核对过才带房间成员。
- 改哪：`mcp` 的 `server.rs` 里 `call_tool`、`mcp/mod.rs` 的 `tool_call_identity`、`family.rs` 常量。
- 依赖：无（协议常量可以先落在同 PR）。
- 大小：M。
- 先红：`shared_bot_single_speker_sets_user`、`shared_bot_two_speakers_omits_user`、`unverified_origin_omits_room_people`、
  `speakers_dedup_sorted_cap_8`。
- 后绿：上述行为；私人 Bot 的 `user` 不变。
- 并行：能。

## 5. 核心：事件入口与队列

- 目标：`events` 权限可用，事件校验、限流、去重、入队。
- 改哪：`gateway/events.rs`（新）、`gateway/mod.rs` 路由、`manifest/permission.rs` 的 `Events`、安装注入 `WONTON_EVENTS_URL`。
- 依赖：协议事件类型（可并到本卡）。
- 大小：L。
- 先红：`without_permission_403`、`bad_kind_400`、`eleven_per_minute_429`、`same_dedupe_key_merges_count`、
  `queue_caps_at_200_and_marks_truncated`。
- 后绿：按 ADR 第 3 节；队列落盘，重启不丢。
- 并行：能，和 6 以外的卡无关；6 等它。

## 6. 核心：事件投递、叫醒与两个工具

- 目标：按人私 Bot→共享 Bot→通知的顺序投递，能叫醒，工具只给收件人。
- 改哪：`gateway/` 的投递、`plugin/contacts.rs` 的 `MATRIX_ALLOWED_USERS`、MCP 的 `wonton__events` 和
  `wonton__events_ack`。
- 依赖：5、ADR-0111 的 503 重试形状；前提是先实测 Hermes 认核心账号（ADR“待验证”第 1 条）。
- 大小：L。
- 先红：`prefers_private_bot_then_shared_then_notice`、`wake_message_has_event_meta`、
  `non_recipient_bot_gets_error`、`ack_removes_from_queue`。
- 后绿：按 ADR 第 3 节；投递失败保留并 5 分钟重试。
- 并行：不能，等 5；和 7 可以并行（不同文件）。

## 7. 协议与核心：通知 `to`、`link`、按人限流

- 目标：通知能发给指定家人，按（插件, 人）限流，非法收件人 400。
- 改哪：`plugin_notice.rs` 两个字段、`gateway/notify.rs` 分发、`notify_limit.rs` 的键。
- 依赖：无；`link` 的校验复用 9。
- 大小：M。
- 先红：`to_two_people_sends_two_notices`、`unknown_person_400`、`limit_counts_per_person`、`link_survives_into_notice`。
- 后绿：按 ADR 第 4 节；不写 `to` 仍只发主人。
- 并行：能。

## 8. 客户端：通知里的深链与“给 X”

- 目标：通知点开走深链，收件不是自己时标“给妈妈”。
- 改哪：`wonton-client` 的通知列表、`chat/links.rs` 的 `wonton://` 点击、通知数据的展示。
- 依赖：7。
- 大小：S。
- 先红：`tapping_notice_link_opens_plugin_path`、`foreign_recipient_badge`。
- 后绿：按 ADR 第 6 节；没装插件时提示不可用。
- 并行：能，和 9 及后台卡并行。

## 9. 协议与核心：卡片模板

- 目标：确认卡按插件模板渲染，结果卡用 `summary`，坏模板回原始 JSON。
- 改哪：协议卡片类型与校验、`mcp/cards/tool.rs` 与 `cards.rs` 的渲染、结果 `_meta` 的 `card_result`。
- 依赖：无。
- 大小：M。
- 先红：`template_renders_scalar_and_count`、`missing_title_arg_falls_back_to_json`、`missing_line_arg_drops_line`、
  `over_limit_truncates`、`bad_template_warns_not_rejects`。
- 后绿：按 ADR 第 5 节；确认按钮和风险行仍在。
- 并行：能；10 依赖它的结果位置。

## 10. 协议与核心：深链校验和结果链接

- 目标：只放行本插件的 `plugin-path` 或本 origin URL，`path` 自动转 `link`。
- 改哪：`link.rs` 的校验函数（新）、`gateway/{notify,events}.rs` 收 `link`、`mcp/cards` 把 `deep_link` 变一行链接。
- 依赖：9（结果卡行）。
- 大小：S。
- 先红：`foreign_plugin_link_400`、`foreign_origin_400`、`path_becomes_plugin_path_link`、`deep_link_renders_clickable`。
- 后绿：按 ADR 第 6 节。
- 并行：能（和 7 的文件有交，先说好）。

## 11. 核心：按房间可见的文件

- 目标：声明了 `room_results` 且房间核对过的结果对在房成员可读；没声明一律 `keeper`。
- 改哪：`files/record.rs` 的 `Accessor::Room`、`files/bot.rs` 的登记与 `for_bot`、`gateway/files.rs` 的鉴权、结果 `result_scope`、
  `audience::verify_room`（含 60 秒缓存）。
- 依赖：无。
- 大小：L。
- 先红：`undeclared_defaults_to_keeper`、`declared_and_verified_gets_room`、`room_member_reads_room_result`、`outsider_403`、
  `left_member_loses_access_after_cache`。
- 附：一条升/降级脚本与用例——`room:` 记录被降级后改成 `bot:`，改不了就删并记日志。
- 后绿：按 ADR 第 7 节；`keeper` 保持旧行为。
- 并行：能，和协议卡不同文件。

## 11b. 协议与核心：`inbox.scan` 命令

- 目标：放完字节调插件声明的 `scan`，30 秒内合并，失败只记日志下次重试。
- 改哪：`inbox` 校验（`scan` 绝对路径）、放完之后的调用、合并计时。
- 依赖：12，和 0（`_meta`）。
- 大小：S。
- 先红：`scan_called_once_per_30s_window`、`scan_failure_does_not_fail_share`、`no_scan_no_call`。
- 后绿：按 ADR 第 8 节；幂等由“同一目录一次”保证。
- 并行：靠 12。

## 12. 协议与核心：收件箱与 `wonton__give_file`

- 目标：`inbox` 收字节，参数换成 `inbox:` 路径，24 小时清。
- 改哪：`inbox.rs`（新类型）、工具 `_meta` 校验、MCP 调用前的参数改写、`files/` 的拷贝与清理、新工具。
- 依赖：1 之后的清单校验框架。
- 大小：L。
- 先红：`declared_arg_is_replaced_with_inbox_path`、`traversal_name_gets_sanitized`、
  `no_inbox_returns_source_unavailable`、`cleanup_after_24h`。
- 后绿：按 ADR 第 8 节；一件 1 MiB、一轮 48 MiB、100 个上限照旧。
- 并行：能，和 11 有 `files/` 交叠，分配文件。

## 13. 客户端：分享页“存进共享文件夹”

- 目标：分享进来的东西能直接放进主人的共享文件夹，并触发 `scan_start`。
- 改哪：分享页去向、文件写入、发 `scan_start` 的那一步。
- 依赖：ADR-0097 的分享页。
- 大小：M。
- 先红：`pick_folder_puts_file_there`、`root_folder_only`、`family_recipient_stays_in_chat`。
- 后绿：按 ADR 第 8 节。
- 并行：能，纯壳。

## 14. `wonton-docs` 规范更新

- 目标：把清单、权限、`_meta`、通知、收件箱写进公开规范。
- 改哪：`plugin-spec.md` 对应各节、`plugin check --official`、`spec_floor.rs` 登记、规范版本号；`events`/`files` 两个权限要写进权限表、安装预览、
  商店元数据和四种语言说明。
- 依赖：1、2、4、5、7、9、10、11b、12 的字段定稿（可先行，随字段改）。
- 大小：M。
- 先红：`plugin check` 对坏 `member_accounts`/`inbox` 的用例；规范里的示例能通过校验。
- 后绿：全部小节；版本号升一小版。
- 并行：最后收口，不和实现同 PR。

## 排期

0. 第零步：0（`_meta` 实验）。过不了先定替代，再开 9、12 与 4 的 `_meta` 半边。
1. 第一波并行：1、4、5、9、11 的协议半边、12 的协议半边、14 的草稿。
2. 第二波：2（等 1）、6（等 5）、10（等 9）、12 的行为、13。
3. 第三波：3（等 1、2）、7（等 10 的校验）、8（等 7）、11 的鉴权（等记录类型）。
4. `files/` 是争用点：11 和 12 各占 `files/bot.rs`、`files/host/*` 的不同函数，先约好再加。
5. `_meta` 的键集中在 4、5、7、9、11、12 六张，第一波合并前先在一处把常量统一，避免各卡各加一份。

## 不拆成卡的

- ADR 本身的修改和“待验证”实验另开卡。
