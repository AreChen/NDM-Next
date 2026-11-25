use super::*;
use gpui_kit::assets::IconName as AssetIcon;
use gpui_motion_kit::Frame;
use ndm_host::harness::{Harness, McpStatus, Status, Target};
use std::io;

#[derive(Default)]
pub(super) struct Clients {
    enabled: bool,
    status: HashMap<Harness, ClientStatus>,
}

impl Clients {
    pub(super) fn new(store: &Store) -> Self {
        Self {
            enabled: store.ai_enabled().unwrap_or(false),
            ..Self::default()
        }
    }
}

#[derive(Clone, Copy, Default)]
struct ClientStatus {
    busy: bool,
    result: Option<Result<Status, Issue>>,
}

#[derive(Clone, Copy)]
enum Action {
    Inspect,
    Install,
    Remove,
}

#[derive(Clone, Copy)]
enum Issue {
    Conflict,
    InvalidConfig,
    Access,
    MissingHost,
    Changed,
    Other,
}

impl From<io::Error> for Issue {
    fn from(error: io::Error) -> Self {
        match error.kind() {
            io::ErrorKind::AlreadyExists => Self::Conflict,
            io::ErrorKind::InvalidData => Self::InvalidConfig,
            io::ErrorKind::PermissionDenied => Self::Access,
            io::ErrorKind::NotFound => Self::MissingHost,
            io::ErrorKind::WouldBlock => Self::Changed,
            _ => Self::Other,
        }
    }
}

impl Desktop {
    pub(in crate::view) fn refresh_ai_client_status(&mut self, cx: &mut Context<Self>) {
        for client in Harness::ALL {
            self.ai_client_action(client, Action::Inspect, cx);
        }
    }

    pub(crate) fn refresh_managed_skill(&self, cx: &mut Context<Self>) {
        let profile = self.store.profile_directory().to_owned();
        let task = self.runtime.spawn_blocking(move || sync_skills(profile));
        cx.spawn(async move |view, cx| {
            if !matches!(task.await, Ok(Ok(()))) {
                let _ = view.update(cx, |this, cx| {
                    this.set_notice(this.text(
                        "部分 AI Skill 刷新失败，请检查客户端配置。",
                        "Some AI Skills could not be refreshed. Check your client configuration.",
                    ).into(), cx);
                    cx.notify();
                });
            }
        })
        .detach();
    }

    fn ai_client_action(&mut self, client: Harness, action: Action, cx: &mut Context<Self>) {
        let state = self.integration.ai.status.entry(client).or_default();
        if state.busy {
            return;
        }
        state.busy = true;
        let profile = self.store.profile_directory().to_owned();
        let work = self.runtime.spawn_blocking(move || {
            let target = Target::for_desktop(profile)?;
            let operation = match action {
                Action::Inspect => Ok(()),
                Action::Install => client.install(&target),
                Action::Remove => client.remove(&target),
            };
            Ok::<_, io::Error>((
                client.status(&target).map_err(Issue::from),
                operation.err().map(Issue::from),
            ))
        });
        cx.spawn(async move |view, cx| {
            let (result, operation_error) = match work.await {
                Ok(Ok(done)) => done,
                Ok(Err(error)) => {
                    let issue = Issue::from(error);
                    (Err(issue), Some(issue))
                }
                Err(_) => (Err(Issue::Other), Some(Issue::Other)),
            };
            let _ = view.update(cx, |this, cx| {
                this.integration.ai.status.insert(
                    client,
                    ClientStatus {
                        busy: false,
                        result: Some(result),
                    },
                );
                if !matches!(action, Action::Inspect)
                    && let Some(issue) = operation_error
                {
                    this.set_notice(
                        format!("{} · {}", client.name(), this.ai_issue_text(issue)),
                        cx,
                    );
                }
                cx.notify();
            });
        })
        .detach();
        cx.notify();
    }

    fn ai_issue_text(&self, issue: Issue) -> &'static str {
        match issue {
            Issue::Conflict => self.text(
                "已有同名配置或 Skill",
                "A same-name configuration or Skill already exists",
            ),
            Issue::InvalidConfig => {
                self.text("客户端配置格式无效", "Client configuration is invalid")
            }
            Issue::Access => self.text(
                "客户端配置权限不足",
                "Permission denied for client configuration",
            ),
            Issue::MissingHost => {
                self.text("无法找到 NDM Next 宿主", "NDM Next host is unavailable")
            }
            Issue::Changed => self.text(
                "客户端配置刚被修改，请重试",
                "Client configuration changed. Try again.",
            ),
            Issue::Other => self.text(
                "无法读写客户端配置",
                "Could not read or write client configuration",
            ),
        }
    }

    pub(in crate::view) fn ai_panel(&self, frame: Frame, cx: &mut Context<Self>) -> AnyElement {
        let tone = crate::motion::ThemeTone::current(cx);
        let open = self.settings_open.contains(&AI_CLIENTS_GROUP);
        let view = cx.entity().downgrade();
        settings::card_surface(AI_CLIENTS_GROUP, tone)
            .flex_none()
            .flex()
            .flex_col()
            .p_3()
            .border_color(rgb(tone.rgb(0x554681, 0xc9bce9)))
            .bg(linear_gradient(
                112.,
                linear_color_stop(rgb(tone.rgb(0x2b2150, 0xf2eaff)), 0.),
                linear_color_stop(rgb(tone.rgb(0x123542, 0xe7f8fc)), 1.),
            ))
            .child(
                div()
                    .flex()
                    .items_center()
                    .gap_3()
                    .child(
                        div()
                            .size(px(40.))
                            .flex()
                            .flex_shrink_0()
                            .items_center()
                            .justify_center()
                            .rounded_lg()
                            .border_1()
                            .border_color(rgba(tone.rgba(0xb59aff70, 0x885dff48)))
                            .bg(rgba(tone.rgba(0x885dff55, 0x885dff20)))
                            .text_color(rgb(tone.rgb(0xebddff, 0x5936a5)))
                            .child(Icon::new(AssetIcon::Sparkles).with_size(px(20.))),
                    )
                    .child(
                        div()
                            .flex_1()
                            .min_w_0()
                            .flex()
                            .flex_col()
                            .gap_1()
                            .child(
                                div().text_base().font_weight(FontWeight::SEMIBOLD).child(
                                    self.display_text(self.text("AI 集成", "AI integration")),
                                ),
                            )
                            .child(
                                div()
                                    .text_xs()
                                    .text_color(rgb(tone.rgb(0xc2c8e3, 0x455272)))
                                    .child(self.text(
                                        "管理客户端的 MCP 和 Skill",
                                        "Manage MCP and Skills for AI clients",
                                    )),
                            ),
                    )
                    .child(
                        crate::motion::Switch::new("enable-ai-integration")
                            .checked(self.integration.ai.enabled)
                            .tooltip(self.text("AI 接入", "AI access"))
                            .on_click(move |enabled, _, cx| {
                                let _ = view.update(cx, |this, cx| {
                                    if this.store.set_ai_enabled(*enabled).is_ok() {
                                        this.integration.ai.enabled = *enabled;
                                        this.refresh_managed_skill(cx);
                                    } else {
                                        this.set_notice(
                                            this.text(
                                                "无法保存 AI 设置。",
                                                "Could not save AI settings.",
                                            )
                                            .into(),
                                            cx,
                                        );
                                    }
                                    cx.notify();
                                });
                            }),
                    )
                    .child(
                        crate::chrome::icon_button(
                            "ai-client-disclosure",
                            if open {
                                IconName::ChevronUp
                            } else {
                                IconName::ChevronDown
                            },
                            if open {
                                self.text("收起客户端", "Hide clients")
                            } else {
                                self.text("展开客户端", "Show clients")
                            },
                        )
                        .on_click(cx.listener(|this, _, window, cx| {
                            this.toggle_settings_group(AI_CLIENTS_GROUP, window, cx)
                        })),
                    ),
            )
            .when(frame.is_mounted(), |card| {
                let rows = Harness::ALL
                    .into_iter()
                    .map(|client| self.ai_client_row(client, tone, cx));
                card.child(settings::settings_detail(
                    AI_CLIENTS_GROUP,
                    frame,
                    open,
                    div().flex().flex_col().children(rows).into_any_element(),
                ))
            })
            .into_any_element()
    }

    fn ai_client_row(
        &self,
        client: Harness,
        tone: crate::motion::ThemeTone,
        cx: &mut Context<Self>,
    ) -> Div {
        let state = self
            .integration
            .ai
            .status
            .get(&client)
            .copied()
            .unwrap_or_default();
        let status = state.result.and_then(Result::ok);
        let installed = status.is_some_and(Status::installed);
        let partial = !installed && status.is_some_and(Status::has_integration);
        let description = if state.busy {
            self.text("处理中…", "Working…")
        } else {
            match state.result {
                Some(Err(issue)) => self.ai_issue_text(issue),
                Some(Ok(_)) if installed => {
                    self.text("MCP 和 Skill 已安装", "MCP and Skill installed")
                }
                Some(Ok(Status {
                    mcp: McpStatus::NeedsRepair,
                    ..
                })) => self.text("MCP 需要修复", "MCP needs repair"),
                Some(Ok(Status {
                    mcp: McpStatus::Configured,
                    ..
                })) => self.text("MCP 已配置，缺少 Skill", "MCP configured. Skill missing."),
                Some(Ok(Status {
                    skill_installed: true,
                    ..
                })) => self.text("Skill 已安装，缺少 MCP", "Skill installed. MCP missing."),
                _ => self.text("待安装", "Available to install"),
            }
        };
        let name = if client == Harness::Codex {
            self.text(
                "OpenAI Codex · CLI 与桌面版",
                "OpenAI Codex · CLI and desktop",
            )
        } else {
            client.name()
        };
        div()
            .flex()
            .items_center()
            .gap_2()
            .py_2()
            .border_t_1()
            .border_color(rgba(tone.rgba(0xb5acdf40, 0x76689932)))
            .child(
                Icon::new(AssetIcon::Terminal)
                    .with_size(px(16.))
                    .text_color(rgb(tone.rgb(0x96e4f1, 0x26687b))),
            )
            .child(
                div()
                    .flex_1()
                    .min_w_0()
                    .flex()
                    .flex_wrap()
                    .items_center()
                    .gap_2()
                    .child(
                        div()
                            .text_sm()
                            .font_weight(FontWeight::SEMIBOLD)
                            .child(name),
                    )
                    .when(partial || matches!(state.result, Some(Err(_))), |row| {
                        row.child(div().text_xs().child(description))
                    }),
            )
            .when(partial, |row| {
                row.child(
                    crate::chrome::icon_button(
                        ("remove-ai-client", client as usize),
                        AssetIcon::Trash,
                        self.text("移除部分安装的集成", "Remove partial integration"),
                    )
                    .disabled(state.busy)
                    .on_click(cx.listener(move |this, _, _, cx| {
                        this.ai_client_action(client, Action::Remove, cx)
                    })),
                )
            })
            .child(
                crate::motion::Switch::new(client.name())
                    .checked(installed)
                    .disabled(state.busy)
                    .tooltip(format!("{} · {}", name, description))
                    .on_click(cx.listener(move |this, checked, _, cx| {
                        let action = if status.is_none() {
                            Action::Inspect
                        } else if *checked {
                            Action::Install
                        } else {
                            Action::Remove
                        };
                        this.ai_client_action(client, action, cx)
                    })),
            )
    }
}

pub(super) fn sync_skills(profile: PathBuf) -> io::Result<()> {
    ndm_host::harness::sync_skills(&Target::for_desktop(profile)?)
}
