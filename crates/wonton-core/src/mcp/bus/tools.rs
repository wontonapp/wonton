//! 工具调用与审计：列工具、调插件、确认决定、写审计。

use super::*;

impl Bus {
    /// 调用方能看到的工具：核心自己的，加上所有在运行、对它放行（[`Bus::bot_access`]）的插件的。并发地问，一个插件卡住
    /// 挡不住别的（每个插件有期限，见 [`Upstreams::tools`]）。列不出工具的插件记一条警告后跳过，它的工具这时本来也调不通。
    ///
    /// 封顶“只读”的（ADR-0088 第 5 节）只列只读的插件工具；核心安装等工具各按自己的权限项管，`save_file` 另按 AI 级别管。
    pub(super) async fn tools(&self, caller: &Caller) -> Vec<Tool> {
        if self.remote_contact_blocked(caller) {
            return contacts_gate::tools();
        }
        let cap = self.ai_cap(caller);
        let mut servers = self.plugins.mcp_servers();
        servers.retain(|server| self.bot_access(caller, &server.id).is_ok());
        let listed = servers.iter().map(|server| async {
            match self.unconfigured(&server.id) {
                Some(notice) => {
                    self.upstreams.note_problem(server, notice.clone());
                    Err(notice)
                }
                None => self.upstreams.tools(server).await,
            }
        });
        let listed = futures_util::future::join_all(listed).await;
        let mut tools = core_tools::definitions();
        for (server, list) in servers.iter().zip(listed) {
            match list {
                Ok(list) => tools.extend(list.into_iter().map(|mut tool| {
                    tool.name = naming::join(&server.id.slug(), &tool.name).into();
                    tool
                })),
                Err(e) => tracing::warn!(caller = %caller.id, error = %e, "插件的工具暂时不可用"),
            }
        }
        if cap == AiLevel::ReadOnly {
            let save = naming::join(naming::CORE, save_tools::SAVE);
            tools.retain(|tool| {
                tool.name != save
                    && (naming::split(&tool.name).is_some_and(|(owner, _)| owner == naming::CORE)
                        || risky::classify(tool) == AiLevel::ReadOnly)
            });
        }
        tools
    }

    /// 插件必填的密钥还没填时的说明，填齐后为空：它的 MCP 工具这时连上也会被对方拒，调用直接按未就绪回绝并
    /// 点名缺哪项（ADR-0064）。
    fn unconfigured(&self, id: &PluginId) -> Option<String> {
        let missing = self.plugins.unfilled_secrets(id, None);
        (!missing.is_empty()).then(|| {
            format!(
                "插件 {id} 还没填好密钥，先填 {} 再用它的工具",
                missing.join("、")
            )
        })
    }

    /// `cancelled` 完成时调用方已经不要结果了（取消了，或者断开了）。
    pub(super) async fn call(
        self: &Arc<Self>,
        caller: &Caller,
        request: CallToolRequestParams,
        cancelled: impl Future<Output = ()> + Send,
    ) -> CallToolResult {
        let activity = self.plugins.activity.start_tool(&caller.id);
        let name = request.name.to_string();
        let arguments = request.arguments.unwrap_or_default();
        let args = Value::Object(arguments.clone());
        let Some((owner, tool)) = naming::split(&name) else {
            return self.deny(
                caller,
                &name,
                &args,
                &ToolFailure::new("invalid_input", format!("没有工具 {name}")),
            );
        };
        if let Some(failure) = self.remote_contact_gate(caller, owner, tool) {
            return self.deny(caller, &name, &args, &failure);
        }
        if owner == naming::CORE {
            return core_tools::call(self, caller, tool, arguments, activity).await;
        }
        let Some(server) = self
            .plugins
            .mcp_servers()
            .into_iter()
            .find(|s| s.id.slug() == owner)
        else {
            return self.deny(
                caller,
                &name,
                &args,
                &ToolFailure::new("not_found", format!("没有在运行的插件提供 {name}")),
            );
        };
        if let Err(failure) = self.bot_access(caller, &server.id) {
            return self.deny(caller, &name, &args, &failure);
        }
        if let Some(notice) = self.unconfigured(&server.id) {
            return self.deny(caller, &name, &args, &ToolFailure::new("not_ready", notice));
        }
        self.call_plugin(
            caller,
            &server,
            (&name, tool, activity),
            arguments,
            cancelled,
        )
        .await
    }

    /// 插件工具的声明：清单这会儿取不到、或插件没有这个工具时，给 Agent 的失败。
    /// 错误装箱：serde_json 开 `preserve_order` 时 `CallToolResult` 撑过 clippy `result_large_err` 的 128 字节阈值。
    async fn definition_of(
        &self,
        caller: &Caller,
        name: &str,
        args: &Value,
        server: &McpServer,
        tool: &str,
    ) -> Result<rmcp::model::Tool, Box<CallToolResult>> {
        let listed = match self.upstreams.tools(server).await {
            Ok(tools) => tools,
            // 插件的工具清单这会儿取不到：多半是插件没就绪或连接断了，Agent 该晚点再试。
            Err(e) => {
                return Err(Box::new(self.deny(
                    caller,
                    name,
                    args,
                    &ToolFailure::new("source_unavailable", e),
                )));
            }
        };
        listed.into_iter().find(|t| t.name == tool).ok_or_else(|| {
            Box::new(self.deny(
                caller,
                name,
                args,
                &ToolFailure::new("not_found", format!("插件 {} 没有工具 {tool}", server.id)),
            ))
        })
    }

    /// 一次插件工具调用的确认决定：封顶不够直接拒（失败即停），会删、会覆盖的调用在“删除要管理员批”的封顶下
    /// 把卡片交给管理员（ADR-0088 第 5 节），其余按 ADR-0024。
    fn plugin_decision(
        &self,
        caller: &Caller,
        server: &McpServer,
        definition: &Tool,
    ) -> Result<risky::ToolDecision, ToolFailure> {
        let cap = self.ai_cap(caller);
        let tool_level = risky::classify(definition);
        if risky::capped(cap, tool_level) {
            return Err(self.read_only_refusal(caller, "这个工具会改或删数据"));
        }
        let highest = self.plugins.requires_root_confirmation(&server.id);
        let subject = if cap == AiLevel::AdminApprovesDeletes && tool_level == AiLevel::SelfConfirms
        {
            Subject::ToolDelete
        } else {
            Subject::Tool
        };
        let relaxed = cap == AiLevel::SelfConfirms;
        Ok(risky::ToolDecision::new(
            definition, cap, highest, subject, relaxed,
        ))
    }

    pub(super) async fn call_plugin(
        self: &Arc<Self>,
        caller: &Caller,
        server: &McpServer,
        (name, tool, activity): (&str, &str, Guard),
        arguments: JsonObject,
        cancelled: impl Future<Output = ()> + Send,
    ) -> CallToolResult {
        let args = Value::Object(arguments.clone());
        let definition = match self.definition_of(caller, name, &args, server, tool).await {
            Ok(definition) => definition,
            Err(result) => return *result,
        };
        let decision = match self.plugin_decision(caller, server, &definition) {
            Ok(decision) => decision,
            Err(failure) => return self.deny(caller, name, &args, &failure),
        };
        let policy = self.plugins.tool_policy();
        let clearance = || {
            decision
                .relaxed
                .then(|| policy.clearance(caller.id.as_str(), server.id.as_str(), tool))
                .flatten()
        };
        let not_sensitive = if decision.every_time {
            None
        } else if decision.read_only {
            Some(Basis::PluginReadOnly)
        } else {
            clearance()
                .filter(|c| *c == Clearance::OwnerReadOnly)
                .map(Basis::from)
        };
        let mut cancelled = std::pin::pin!(cancelled);
        let (basis, held_ms, activity) = if not_sensitive.is_some() {
            (not_sensitive, None, activity)
        } else {
            drop(activity);
            let held_ms = match self
                .hold(caller, name, &args, &server.id, cancelled.as_mut())
                .await
            {
                Ok(held_ms) => held_ms,
                Err(result) => return *result,
            };
            // 停住期间主人可能改了放宽，时段也可能到期，所以放在停住之后判定。每次都问的工具全部不放宽。
            let cleared = if decision.every_time || !decision.relaxed {
                None
            } else {
                clearance().map(Basis::from).or_else(|| {
                    self.confirmations
                        .allow_in_window(
                            &caller.id,
                            &risky::window_scope(caller, server.id.as_str()),
                        )
                        .map(Basis::Window)
                })
            };
            let activity = self.plugins.activity.start_tool(&caller.id);
            (cleared, held_ms, activity)
        };
        if let Some(basis) = basis {
            let outcome = forward(self, caller, server, tool, arguments, cancelled).await;
            self.record(
                caller,
                name,
                &args,
                Decision::Allowed,
                Some(Grounds {
                    basis: basis.clone(),
                    held_ms,
                }),
                &outcome,
            );
            // 放行时段里没发卡片，但这段是主人先前批准的：照实告诉 Agent。
            return match basis {
                Basis::Window(_) => with_confirmation(outcome.result, "window"),
                _ => outcome.result,
            };
        }
        let draft = self.tool_card(caller, server, &definition, &args, decision);
        // 不传 `cancelled`：确认过的调用不跟着 Agent 走，它断开了照样执行，结果在卡片上（ADR-0016）。
        let (agent, server, tool) = (caller.clone(), server.clone(), tool.to_owned());
        let run = move |bus: Arc<Bus>,
                        _secrets: SecretValues,
                        _settings: wonton_protocol::SettingValues,
                        _devices: Vec<String>| async move {
            forward(
                &bus,
                &agent,
                &server,
                &tool,
                arguments,
                std::future::pending(),
            )
            .await
        };
        self.confirmed_after_hold(
            caller,
            name,
            args,
            (draft, activity),
            run,
            (held_ms, decision.subject),
        )
        .await
    }

    /// 主人没同意：拒绝了、过期了、链被停了抢在投递前撤回卡片，或者卡片发不出去。
    pub(super) fn refused(
        &self,
        caller: &Caller,
        name: &str,
        args: &Value,
        refusal: &confirm::Refusal,
        held_ms: Option<u64>,
    ) -> CallToolResult {
        let decision = match refusal {
            confirm::Refusal::Rejected => Decision::Rejected,
            confirm::Refusal::Expired => Decision::Expired,
            confirm::Refusal::Cancelled => Decision::Withdrawn,
            _ => Decision::Unconfirmed,
        };
        let message = refusal.message();
        self.audit.record(&Record {
            caller: caller.id.as_str(),
            tool: name,
            arguments: args,
            decision,
            basis: Some(Grounds {
                basis: Basis::Card,
                held_ms,
            }),
            outcome: Some(&message),
            files: &[],
        });
        with_failure(&message, refusal.kind())
    }

    /// 总线拒绝：工具不在、参数不对、不让这个 Bot 做的事。类别进结果的 `_meta`，Agent 不用读文案。
    pub(super) fn deny(
        &self,
        caller: &Caller,
        name: &str,
        args: &Value,
        failure: &ToolFailure,
    ) -> CallToolResult {
        self.audit.record(&Record {
            caller: caller.id.as_str(),
            tool: name,
            arguments: args,
            decision: Decision::Denied,
            basis: None,
            outcome: Some(&failure.message),
            files: &[],
        });
        with_failure(&failure.message, failure.kind)
    }

    pub(super) fn record(
        &self,
        caller: &Caller,
        name: &str,
        args: &Value,
        decision: Decision,
        basis: Option<Grounds>,
        outcome: &Outcome,
    ) {
        let files = &outcome.files;
        let outcome = match &outcome.state {
            ConfirmationState::Failed { message } | ConfirmationState::Uncertain { message } => {
                message.as_str()
            }
            _ => "ok",
        };
        self.audit.record(&Record {
            caller: caller.id.as_str(),
            tool: name,
            arguments: args,
            decision,
            basis,
            outcome: Some(outcome),
            files,
        });
    }
}
