use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use gpui::{
    AnyElement, ClickEvent, Context, CursorStyle, Entity, Hsla, InteractiveElement, IntoElement,
    ParentElement, PathPromptOptions, SharedString, StatefulInteractiveElement, Styled, div,
    prelude::*, px, rgb, svg,
};
use paneflow_config::schema::AgentProfileConfig;
use paneflow_mcp_install::IntegrationState;

use crate::PaneFlowApp;
use crate::SidebarWidthAnimation;
use crate::agent_launcher::{AgentProfile, ORCHESTRATOR_MODEL, TerminalAgent};
use crate::agent_updates::{self, AgentUpdateState, UpdateMethod, UpdatePhase};
use crate::settings::components::{
    deferred_select_menu, destructive_color, ghost_icon_button, ghost_skin, hairline,
    hairline_inset, raised_button, section_header_with_action, section_title, select_chevron,
    select_menu, select_option, select_trigger, setting_card, setting_text, soft_button,
    toggle_row, toggle_switch, with_alpha,
};
use crate::settings::search::{self, Block, SearchCard};
use crate::ui_primitives::{
    AccessibleControlExt, BODY, BODY_EMPHASIS, LABEL_SM, LABEL_XS, TooltipDelayExt, text_tooltip,
};
use crate::widgets::text_input::TextInput;

const ROW_ICON: f32 = 18.;
const CONTROL_WIDTH: f32 = 300.;
const ENV_NAME_WIDTH: f32 = 200.;
const INPUT_HEIGHT: f32 = 28.;
const ICON_BUTTON_SIZE: f32 = 26.;
const AGENT_ROW_HEIGHT: f32 = 50.;
const CARD_PADDING_X: f32 = crate::settings::components::CARD_PADDING_X;
const CARD_PADDING_Y: f32 = crate::settings::components::CARD_PADDING_Y;
const HAIRLINE_HEIGHT: f32 = 1.;
const MISSING_PREVIEW_ROWS: usize = 3;
const UPDATE_METHOD_WIDTH: f32 = 220.;

fn installed_agents() -> impl Iterator<Item = TerminalAgent> {
    TerminalAgent::for_this_platform().filter(|agent| agent.is_installed())
}

fn missing_agents() -> impl Iterator<Item = TerminalAgent> {
    TerminalAgent::for_this_platform().filter(|agent| !agent.is_installed())
}

fn folded_missing_count() -> usize {
    missing_agents()
        .count()
        .saturating_sub(MISSING_PREVIEW_ROWS)
}

fn folded_missing_height() -> f32 {
    folded_missing_count() as f32 * (AGENT_ROW_HEIGHT + HAIRLINE_HEIGHT)
}

pub(crate) struct EnvRowInputs {
    pub(crate) key: Entity<TextInput>,
    pub(crate) value: Entity<TextInput>,
}

pub(crate) struct AgentProfileEditor {
    pub(crate) index: Option<usize>,
    pub(crate) agent: TerminalAgent,
    pub(crate) agent_menu_open: bool,
    pub(crate) error: Option<String>,
    pub(crate) env_rows: Vec<EnvRowInputs>,
    pub(crate) model_input: Entity<TextInput>,
    pub(crate) models: Vec<crate::agent_profile_context::ModelChoice>,
    pub(crate) model_menu_open: bool,
    pub(crate) model_catalog_generation: u64,
    pub(crate) custom_model: bool,
    pub(crate) effort: Option<String>,
    pub(crate) effort_menu_open: bool,
    pub(crate) project: Option<PathBuf>,
    pub(crate) default_profile: Option<String>,
    pub(crate) skills: Vec<PathBuf>,
    pub(crate) skill_input: Entity<TextInput>,
    pub(crate) available_skills: Vec<crate::agent_profile_context::SkillChoice>,
    pub(crate) skills_loading: bool,
    pub(crate) skills_generation: u64,
    pub(crate) suggested_name: String,
    pub(crate) role_search: Entity<TextInput>,
    pub(crate) role_menu_open: bool,
    pub(crate) agency_role: Option<String>,
}

fn matching_agency_roles(query: &str) -> Vec<&'static paneflow_mcp_install::agency::AgencyRole> {
    let query = query.trim().to_lowercase();
    paneflow_mcp_install::agency::ROLES
        .iter()
        .filter(|role| {
            query.is_empty()
                || format!(
                    "{} {} {} {}",
                    role.id, role.name, role.category, role.description
                )
                .to_lowercase()
                .contains(&query)
        })
        .collect()
}

fn profile_project_label(project: &Path) -> String {
    project
        .file_name()
        .unwrap_or(project.as_os_str())
        .to_string_lossy()
        .into_owned()
}

fn suggested_profile_name(
    project: Option<&Path>,
    role: Option<&str>,
    agent: TerminalAgent,
    profiles: &[AgentProfileConfig],
    editing: Option<usize>,
) -> String {
    let role_name = role
        .and_then(paneflow_mcp_install::agency::role)
        .map(|role| role.name);
    let base = match (project, role_name) {
        (Some(project), role) => format!(
            "{} {}",
            profile_project_label(project),
            role.unwrap_or(agent.display_name())
        ),
        (None, Some(role)) => format!("Agency {role}"),
        (None, None) => agent.display_name().to_string(),
    };
    let mut name = base.clone();
    let mut suffix = 2;
    while profiles.iter().enumerate().any(|(index, entry)| {
        Some(index) != editing
            && entry.applies_to_project(project)
            && entry.name.eq_ignore_ascii_case(&name)
    }) {
        name = format!("{base} {suffix}");
        suffix += 1;
    }
    name
}

impl PaneFlowApp {
    pub(crate) fn open_project_agent_settings(
        &mut self,
        index: usize,
        window: &mut gpui::Window,
        cx: &mut Context<Self>,
    ) {
        let Some(workspace) = self.workspaces.get(index) else {
            return;
        };
        let project = workspace
            .repo_root
            .clone()
            .unwrap_or_else(|| PathBuf::from(&workspace.cwd));
        if !project.is_absolute() {
            return;
        }
        self.open_settings_at(crate::SettingsSection::Agents, window, cx);
        self.agent_profiles_project = Some(project);
        cx.notify();
    }

    fn active_profile_project(&self) -> Option<PathBuf> {
        let workspace = self.active_workspace()?;
        let project = workspace
            .repo_root
            .as_deref()
            .unwrap_or_else(|| Path::new(&workspace.cwd));
        project.is_absolute().then(|| project.to_path_buf())
    }

    fn update_agent_profile_suggested_name(&mut self, cx: &mut Context<Self>) {
        let Some(editor) = self.agent_profile_editor.as_mut() else {
            return;
        };
        if editor.agency_role.as_deref() == Some("orchestrator")
            && editor.model_input.read(cx).value() == ORCHESTRATOR_MODEL
        {
            editor
                .model_input
                .update(cx, |input, cx| input.set_value(SharedString::from(""), cx));
            editor.custom_model = false;
        }
        let next = suggested_profile_name(
            editor.project.as_deref(),
            editor.agency_role.as_deref(),
            editor.agent,
            &self.cached_config.agent_profiles,
            editor.index,
        );
        let current = self.agent_profile_name_input.read(cx).value();
        if current.trim().is_empty() || current == editor.suggested_name {
            self.agent_profile_name_input.update(cx, |input, cx| {
                input.set_value(SharedString::from(next.clone()), cx)
            });
        }
        editor.suggested_name = next;
    }

    pub(crate) fn render_agents_content(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let ui = crate::theme::ui_colors();
        let project_view = self.agent_profiles_project.is_some();
        div()
            .flex()
            .flex_col()
            .child(div().pb(px(12.)).child(if project_view {
                soft_button(
                    "agent-profiles-defaults",
                    "Edit default profiles",
                    ui,
                    cx.listener(|this, _, window, cx| {
                        this.open_settings_at(crate::SettingsSection::Agents, window, cx)
                    }),
                )
                .into_any_element()
            } else if self.active_profile_project().is_some() {
                let index = self.active_idx;
                soft_button(
                    "agent-profiles-project-settings",
                    "Project agent settings",
                    ui,
                    cx.listener(move |this, _, window, cx| {
                        this.open_project_agent_settings(index, window, cx)
                    }),
                )
                .into_any_element()
            } else {
                div().into_any_element()
            }))
            .when(!project_view, |column| {
                column
                    .child(self.render_installed_agents_section(ui, cx))
                    .child(self.render_missing_agents_section(ui, cx))
            })
            .child(self.render_agent_profiles_section(ui, cx))
            .when(!project_view, |column| {
                column.child(self.render_agent_permissions_section(ui, cx))
            })
            .child(div().h(px(180.)).flex_none())
    }

    pub(crate) fn tick_agents_list_animation(&mut self, window: &mut gpui::Window) {
        let Some(animation) = self.agents_list_animation else {
            return;
        };
        if animation.is_finished(std::time::Instant::now()) {
            self.agents_list_animation = None;
        } else {
            window.request_animation_frame();
        }
    }

    fn agents_list_height_now(&self) -> f32 {
        match self.agents_list_animation {
            Some(animation) => animation.width_at(std::time::Instant::now()),
            None if self.agents_list_expanded => folded_missing_height(),
            None => 0.,
        }
    }

    fn toggle_agents_list(&mut self, cx: &mut Context<Self>) {
        let now = std::time::Instant::now();
        let from_width = self.agents_list_height_now();
        self.agents_list_expanded = !self.agents_list_expanded;
        let to_width = if self.agents_list_expanded {
            folded_missing_height()
        } else {
            0.
        };
        self.agents_list_animation =
            if !crate::ui_primitives::reduce_motion() && (from_width - to_width).abs() > 0.5 {
                Some(SidebarWidthAnimation {
                    from_width,
                    to_width,
                    started_at: now,
                })
            } else {
                None
            };
        cx.notify();
    }

    pub(crate) fn probe_agent_versions(&mut self, cx: &mut Context<Self>) {
        cx.spawn(async move |this, cx| {
            smol::unblock(TerminalAgent::probe_missing_versions).await;
            let _ = this.update(cx, |this, cx| {
                cx.notify();
                this.inspect_agent_updates(cx);
            });
        })
        .detach();
    }

    pub(crate) fn refresh_integration_status(&self, cx: &mut Context<Self>) {
        cx.spawn(async move |this, cx| {
            let statuses = smol::unblock(paneflow_mcp_install::list_integrations).await;
            let _ = this.update(cx, |this, cx| {
                this.integration_status = Some(statuses);
                cx.notify();
            });
        })
        .detach();
    }

    fn change_integration(&mut self, slug: String, install: bool, cx: &mut Context<Self>) {
        if self.integration_busy.is_some() {
            return;
        }
        self.integration_busy = Some(slug.clone());
        self.integration_errors.remove(&slug);
        cx.notify();
        cx.spawn(async move |this, cx| {
            let action_slug = slug.clone();
            let result = smol::unblock(move || {
                if install {
                    let hook_binary = crate::ai_hooks::extract::ensure_ai_hook_extracted()
                        .map_err(|error| format!("hook extraction failed: {error:#}"))?;
                    let bridge_binary = crate::ai_hooks::extract::ensure_bridge_extracted()
                        .map_err(|error| format!("bridge extraction failed: {error:#}"))?;
                    paneflow_mcp_install::install_integration(
                        &action_slug,
                        &paneflow_mcp_install::IntegrationBinaries {
                            hook_binary,
                            bridge_binary,
                        },
                        false,
                    )
                } else {
                    paneflow_mcp_install::remove_integration(&action_slug)
                }
            })
            .await;
            let statuses = smol::unblock(paneflow_mcp_install::list_integrations).await;
            let _ = this.update(cx, |this, cx| {
                this.integration_busy = None;
                this.integration_status = Some(statuses);
                match result {
                    Err(error) => {
                        this.integration_errors.insert(slug, error);
                    }
                    Ok(status) if !status.notes.is_empty() => {
                        this.integration_errors
                            .insert(slug, status.notes.join("\n"));
                    }
                    Ok(_) => {}
                }
                cx.notify();
            });
        })
        .detach();
    }

    fn render_installed_agents_section(
        &self,
        ui: crate::theme::UiColors,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let mut card = setting_card(ui).py(px(CARD_PADDING_Y));
        let mut any = false;
        for agent in installed_agents() {
            if any {
                card = card.child(hairline_inset(ui));
            }
            any = true;
            card = card.child(self.render_installed_agent_row(agent, ui, cx));
        }
        if !any {
            card = card.child(
                div()
                    .px(px(CARD_PADDING_X))
                    .py(px(14.))
                    .text_size(LABEL_SM)
                    .text_color(ui.muted)
                    .child("No agent found on PATH."),
            );
        }
        Block::new("Agents").child(card).finish()
    }

    fn render_missing_agents_section(
        &self,
        ui: crate::theme::UiColors,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let missing: Vec<TerminalAgent> = missing_agents().collect();
        if missing.is_empty() {
            return div().into_any_element();
        }
        let expanded = self.agents_list_expanded;
        let folded = folded_missing_count();
        let counter: SharedString = missing.len().to_string().into();

        let mut card = setting_card(ui).py(px(CARD_PADDING_Y));
        for (idx, agent) in missing.iter().take(MISSING_PREVIEW_ROWS).enumerate() {
            if idx > 0 {
                card = card.child(hairline_inset(ui));
            }
            card = card.child(missing_agent_row(*agent, ui));
        }
        let folded_height = self.agents_list_height_now();
        if folded_height > 0. {
            let mut rest = div()
                .flex()
                .flex_col()
                .flex_none()
                .h(px(folded_height))
                .overflow_hidden();
            for agent in missing.iter().skip(MISSING_PREVIEW_ROWS) {
                rest = rest
                    .child(hairline_inset(ui))
                    .child(missing_agent_row(*agent, ui));
            }
            card = card.child(rest);
        }
        if folded > 0 {
            let more_label: SharedString = if expanded {
                "Show less".into()
            } else {
                format!("Show {folded} more").into()
            };
            let more_button = ghost_skin(
                div()
                    .id("agents-show-more")
                    .h(px(28.))
                    .px(px(10.))
                    .flex()
                    .flex_row()
                    .items_center()
                    .gap(px(6.))
                    .text_size(LABEL_SM)
                    .font_weight(gpui::FontWeight::MEDIUM)
                    .text_color(ui.text),
                "agents-show-more-squircle",
                px(14.),
                ui,
            )
            .on_click(cx.listener(|this, _: &ClickEvent, _w, cx| {
                this.toggle_agents_list(cx);
            }))
            .child(more_label)
            .child(
                svg()
                    .size(px(10.))
                    .flex_none()
                    .path(if expanded {
                        "icons/settings/arrow-up-01.svg"
                    } else {
                        "icons/settings/arrow-down-01.svg"
                    })
                    .text_color(with_alpha(ui.text, 0.8)),
            );
            card = card.child(hairline(ui)).child(
                div()
                    .py(px(6.))
                    .flex()
                    .flex_row()
                    .justify_center()
                    .child(more_button),
            );
        }

        Block::new("Not installed")
            .top_gap(24.)
            .child(section_header_with_action(
                ui,
                "Not installed",
                div()
                    .text_size(LABEL_SM)
                    .text_color(ui.muted)
                    .child(counter),
            ))
            .child(card)
            .finish()
    }

    fn toggle_agent_details(&mut self, agent: TerminalAgent, cx: &mut Context<Self>) {
        self.expanded_agent = if self.expanded_agent == Some(agent) {
            None
        } else {
            Some(agent)
        };
        cx.notify();
    }

    fn inspect_agent_updates(&mut self, cx: &mut Context<Self>) {
        for agent in installed_agents() {
            if self
                .agent_updates
                .get(&agent)
                .is_some_and(AgentUpdateState::is_running)
            {
                continue;
            }
            cx.spawn(async move |this, cx| {
                let inspection = smol::unblock(move || agent_updates::inspect(agent)).await;
                let _ = this.update(cx, |this, cx| {
                    let state = this.agent_updates.entry(agent).or_default();
                    if !state.is_running() {
                        state.inspection = Some(inspection);
                        cx.notify();
                    }
                });
            })
            .detach();
        }
    }

    fn selected_update_method(&self, agent: TerminalAgent) -> UpdateMethod {
        UpdateMethod::from_id(
            self.cached_config
                .agents
                .as_ref()
                .and_then(|agents| agents.update_methods.get(agent.runtime().slug))
                .map(String::as_str),
        )
    }

    fn set_agent_update_method(
        &mut self,
        agent: TerminalAgent,
        method: UpdateMethod,
        cx: &mut Context<Self>,
    ) {
        self.agent_update_menu_open = None;
        let mut agents = self.cached_config.agents.clone().unwrap_or_default();
        let slug = agent.runtime().slug.to_string();
        if method == UpdateMethod::Auto {
            agents.update_methods.remove(&slug);
        } else {
            agents.update_methods.insert(slug, method.id().to_string());
        }
        if let Some(state) = self.agent_updates.get_mut(&agent)
            && !state.is_running()
        {
            state.phase = None;
        }
        match serde_json::to_value(agents) {
            Ok(value) => self.persist_setting(false, "agents", value, cx),
            Err(_) => cx.notify(),
        }
    }

    fn start_agent_update(&mut self, agent: TerminalAgent, cx: &mut Context<Self>) {
        let method = self.selected_update_method(agent);
        let Some(state) = self.agent_updates.get_mut(&agent) else {
            return;
        };
        if state.is_running() {
            return;
        }
        let Some(plan) = state.runnable_plan(method).cloned() else {
            return;
        };
        state.phase = Some(UpdatePhase::Running { progress: None });
        cx.notify();
        let before = agent.cached_version();
        cx.spawn(async move |this, cx| {
            let (sender, receiver) = smol::channel::unbounded::<String>();
            let job = smol::unblock(move || {
                agent_updates::run_update(&plan, move |line| {
                    let _ = sender.try_send(line);
                })
            });
            while let Ok(line) = receiver.recv().await {
                let _ = this.update(cx, |this, cx| {
                    if let Some(state) = this.agent_updates.get_mut(&agent)
                        && state.is_running()
                    {
                        state.phase = Some(UpdatePhase::Running {
                            progress: Some(line),
                        });
                        cx.notify();
                    }
                });
            }
            let result = job.await;
            let after = smol::unblock(move || agent.refresh_version()).await;
            let phase = match result {
                Err(message) => UpdatePhase::Failed { message },
                Ok(()) if after.is_some() && after != before => {
                    UpdatePhase::Updated { version: after }
                }
                Ok(()) => UpdatePhase::Unchanged,
            };
            let _ = this.update(cx, |this, cx| {
                this.agent_updates.entry(agent).or_default().phase = Some(phase);
                cx.notify();
            });
        })
        .detach();
    }

    fn open_agent_sessions(&self, agent: TerminalAgent) -> usize {
        let mut surfaces = std::collections::HashSet::new();
        let mut unbound = 0;
        for session in self
            .workspaces
            .iter()
            .flat_map(|workspace| workspace.agent_sessions.values())
            .filter(|session| session.tool == agent)
        {
            match session.surface_id {
                Some(surface) => {
                    surfaces.insert(surface);
                }
                None => unbound += 1,
            }
        }
        surfaces.len() + unbound
    }

    fn render_installed_agent_row(
        &self,
        agent: TerminalAgent,
        ui: crate::theme::UiColors,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let visible = agent.is_visible(&self.cached_config);
        let expanded = self.expanded_agent == Some(agent);
        let installed = agent.cached_version();
        let update = self.agent_updates.get(&agent);
        let running = update.is_some_and(AgentUpdateState::is_running);
        let newer = update
            .and_then(|update| update.newer_version(installed.as_deref()))
            .map(str::to_string);
        let method = self.selected_update_method(agent);
        let can_update = update.is_some_and(|update| update.runnable_plan(method).is_some());

        let subtitle: AnyElement = if running {
            div()
                .text_size(LABEL_SM)
                .text_color(ui.muted)
                .child("Updating…")
                .into_any_element()
        } else {
            let mut label = agent.binary().to_string();
            if let Some(version) = installed.as_deref() {
                label.push(' ');
                label.push_str(version);
            }
            div()
                .flex()
                .flex_row()
                .items_center()
                .gap(px(4.))
                .child(mono_text(label.into(), ui.muted))
                .when_some(newer.clone(), |line, newer| {
                    line.child(mono_text("→".into(), ui.muted))
                        .child(mono_text(newer.into(), ui.text))
                })
                .into_any_element()
        };

        let tag = agent.tag();
        let key = agent.visibility_config_key();
        let target = !visible;
        let toggle = toggle_switch(
            SharedString::from(format!("agent-visible-{tag}")),
            format!("Show {} in the agent picker", agent.display_name()),
            visible,
            ui,
        )
        .cursor(CursorStyle::PointingHand)
        .on_click(cx.listener(move |this, _: &ClickEvent, _w, cx| {
            cx.stop_propagation();
            this.persist_setting(false, key, serde_json::Value::Bool(target), cx);
        }));

        let action: Option<AnyElement> = if running {
            Some(crate::ui_primitives::comet_spinner(
                SharedString::from(format!("agent-update-spinner-{tag}")),
                px(14.),
                ui.muted,
            ))
        } else if newer.is_some() && can_update {
            Some(
                soft_button(
                    SharedString::from(format!("agent-update-header-{tag}")),
                    "Update",
                    ui,
                    cx.listener(move |this, _: &ClickEvent, _w, cx| {
                        cx.stop_propagation();
                        this.start_agent_update(agent, cx);
                    }),
                )
                .into_any_element(),
            )
        } else {
            None
        };

        let group = SharedString::from(format!("agent-row-{tag}"));
        let header = div()
            .id(group.clone())
            .group(group.clone())
            .flex()
            .flex_row()
            .items_center()
            .flex_none()
            .h(px(AGENT_ROW_HEIGHT))
            .gap(px(12.))
            .px(px(CARD_PADDING_X))
            .cursor(CursorStyle::PointingHand)
            .role(gpui::accesskit::Role::Button)
            .aria_label(format!("{} settings", agent.display_name()))
            .aria_expanded(expanded)
            .on_click(cx.listener(move |this, _: &ClickEvent, _w, cx| {
                this.toggle_agent_details(agent, cx);
            }))
            .child(agent_icon_el(agent, ui))
            .child(
                div()
                    .flex_1()
                    .min_w_0()
                    .flex()
                    .flex_col()
                    .gap(px(1.))
                    .child(
                        div()
                            .text_size(BODY_EMPHASIS)
                            .font_weight(gpui::FontWeight::MEDIUM)
                            .text_color(ui.text)
                            .child(agent.display_name()),
                    )
                    .child(subtitle),
            )
            .child(
                div()
                    .id(SharedString::from(format!("agent-row-controls-{tag}")))
                    .flex()
                    .flex_row()
                    .items_center()
                    .gap(px(12.))
                    .flex_none()
                    .on_mouse_down(gpui::MouseButton::Left, |_, _, cx| cx.stop_propagation())
                    .children(action)
                    .child(
                        svg()
                            .size(px(12.))
                            .flex_none()
                            .path(if expanded {
                                "icons/settings/arrow-down-01.svg"
                            } else {
                                "icons/settings/arrow-right-01.svg"
                            })
                            .text_color(ui.muted)
                            .group_hover(group, move |style| style.text_color(ui.text)),
                    )
                    .child(toggle),
            );

        div()
            .flex()
            .flex_col()
            .child(header)
            .when(expanded, |row| {
                row.child(self.render_agent_details(agent, ui, cx))
            })
            .into_any_element()
    }

    fn render_agent_details(
        &self,
        agent: TerminalAgent,
        ui: crate::theme::UiColors,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let mut rows: Vec<AnyElement> = vec![self.render_agent_update_row(agent, ui, cx)];
        if let Some(method) = self.render_agent_update_method_row(agent, ui, cx) {
            rows.push(method);
        }
        if let Some(location) = self.render_agent_location_row(agent, ui, cx) {
            rows.push(location);
        }
        if let Some(hooks) = self.render_agent_hooks_row(agent, ui, cx) {
            rows.push(hooks);
        }
        let mut details = div()
            .flex()
            .flex_col()
            .pl(px(crate::settings::components::ROW_HAIRLINE_INSET))
            .pr(px(CARD_PADDING_X))
            .pb(px(4.));
        for (idx, row) in rows.into_iter().enumerate() {
            if idx > 0 {
                details = details.child(hairline(ui));
            }
            details = details.child(row);
        }
        details.into_any_element()
    }

    fn render_agent_update_row(
        &self,
        agent: TerminalAgent,
        ui: crate::theme::UiColors,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let tag = agent.tag();
        let installed = agent.cached_version();
        let method = self.selected_update_method(agent);
        let state = self.agent_updates.get(&agent);
        let inspection = state.and_then(|state| state.inspection.as_ref());
        let plan = inspection.and_then(|inspection| inspection.plan(method));
        let latest = inspection.and_then(|inspection| inspection.latest.as_deref());
        let newer = state.and_then(|state| state.newer_version(installed.as_deref()));
        let up_to_date = state.is_some_and(|state| state.is_up_to_date(installed.as_deref()));

        let (text, color): (String, Hsla) =
            match (state.and_then(|state| state.phase.as_ref()), plan) {
                (Some(UpdatePhase::Running { progress }), _) => (
                    progress.clone().unwrap_or_else(|| "Updating…".to_string()),
                    ui.muted,
                ),
                (Some(UpdatePhase::Failed { message }), _) => {
                    (format!("Update failed: {message}"), destructive_color())
                }
                (Some(UpdatePhase::Updated { version }), _) => (
                    match version {
                        Some(version) => format!("Updated to {version}."),
                        None => "Updated.".to_string(),
                    },
                    ui.muted,
                ),
                (Some(UpdatePhase::Unchanged), _) => ("Already up to date.".to_string(), ui.muted),
                (None, None) => ("Checking for updates…".to_string(), ui.muted),
                (None, Some(Err(reason))) => (reason.clone(), ui.muted),
                (None, Some(Ok(plan))) => (
                    match (newer, latest) {
                        (Some(newer), _) => {
                            format!("Version {newer} is available through {}.", plan.via)
                        }
                        (None, Some(_)) => "Up to date.".to_string(),
                        (None, None) => format!("Updates through {}.", plan.via),
                    },
                    ui.muted,
                ),
            };

        let running = state.is_some_and(AgentUpdateState::is_running);
        let sessions = self.open_agent_sessions(agent);
        let mut description = div()
            .flex()
            .flex_col()
            .gap(px(2.))
            .child(detail_text(text, color));
        let offers_update = plan.is_some_and(Result::is_ok) && !up_to_date;
        if sessions > 0 && offers_update && !running {
            description = description.child(detail_text(
                if sessions == 1 {
                    "1 open session keeps the current version until it restarts.".to_string()
                } else {
                    format!("{sessions} open sessions keep the current version until they restart.")
                },
                ui.muted,
            ));
        }

        let control: Option<AnyElement> = if running {
            Some(crate::ui_primitives::comet_spinner(
                SharedString::from(format!("agent-update-row-spinner-{tag}")),
                px(14.),
                ui.muted,
            ))
        } else if offers_update {
            Some(
                soft_button(
                    SharedString::from(format!("agent-update-{tag}")),
                    "Update",
                    ui,
                    cx.listener(move |this, _: &ClickEvent, _w, cx| {
                        this.start_agent_update(agent, cx);
                    }),
                )
                .into_any_element(),
            )
        } else {
            None
        };
        detail_row("Update", description.into_any_element(), control, ui)
    }

    fn render_agent_update_method_row(
        &self,
        agent: TerminalAgent,
        ui: crate::theme::UiColors,
        cx: &mut Context<Self>,
    ) -> Option<AnyElement> {
        let inspection = self.agent_updates.get(&agent)?.inspection.as_ref()?;
        let selected = self.selected_update_method(agent);
        let offered = inspection.offered_methods(selected);
        if offered.len() <= 1 {
            return None;
        }
        let tag = agent.tag();
        let auto_via = inspection
            .plan(UpdateMethod::Auto)
            .and_then(|plan| plan.as_ref().ok())
            .map(|plan| plan.via);
        let option_label = |method: UpdateMethod| -> String {
            match (method, auto_via) {
                (UpdateMethod::Auto, Some(via)) => format!("Automatic ({via})"),
                _ => method.label().to_string(),
            }
        };

        let is_open = self.agent_update_menu_open == Some(agent);
        let mut trigger = select_trigger(
            SharedString::from(format!("agent-update-method-{tag}")),
            "Update method",
            is_open,
            ui,
        )
        .w_full()
        .h(px(INPUT_HEIGHT))
        .py(px(0.))
        .on_press(cx.listener(move |this, _, window, cx| {
            cx.stop_propagation();
            this.agent_update_menu_open = if is_open { None } else { Some(agent) };
            this.settings_focus.focus(window, cx);
            cx.notify();
        }))
        .child(
            div()
                .flex_1()
                .min_w_0()
                .text_size(BODY)
                .text_color(ui.text)
                .truncate()
                .child(option_label(selected)),
        )
        .child(select_chevron(ui));
        if is_open {
            let mut menu = select_menu(
                SharedString::from(format!("agent-update-method-list-{tag}")),
                ui,
            )
            .on_mouse_down_out(cx.listener(|this, _, _w, cx| {
                this.agent_update_menu_open = None;
                cx.notify();
            }));
            for method in offered {
                menu = menu.child(
                    select_option(
                        SharedString::from(format!("agent-update-method-{tag}-{}", method.id())),
                        method == selected,
                        ui,
                    )
                    .cursor(CursorStyle::Arrow)
                    .on_click(cx.listener(move |this, _: &ClickEvent, _w, cx| {
                        this.set_agent_update_method(agent, method, cx);
                    }))
                    .child(
                        div()
                            .flex_1()
                            .min_w_0()
                            .truncate()
                            .text_color(ui.text)
                            .child(option_label(method)),
                    ),
                );
            }
            trigger = trigger.child(deferred_select_menu(menu));
        }

        let mut description = div().flex().flex_col().gap(px(2.)).min_w_0();
        match inspection.plan(selected) {
            Some(Ok(plan)) => {
                description = description.child(
                    div()
                        .min_w_0()
                        .truncate()
                        .child(mono_text(plan.display_command().into(), ui.muted)),
                );
            }
            Some(Err(reason)) => {
                description = description.child(detail_text(reason.clone(), destructive_color()));
            }
            None => {}
        }
        if let Some(via) = auto_via.filter(|_| inspection.installs_second_copy(selected)) {
            description = description.child(detail_text(
                format!(
                    "{} was installed through {via}: this installs a second copy, and the one \
                     first on PATH runs.",
                    agent.display_name()
                ),
                ui.vc_modified,
            ));
        }

        Some(detail_row(
            "Update with",
            description.into_any_element(),
            Some(
                div()
                    .relative()
                    .flex_none()
                    .w(px(UPDATE_METHOD_WIDTH))
                    .child(trigger)
                    .into_any_element(),
            ),
            ui,
        ))
    }

    fn render_agent_location_row(
        &self,
        agent: TerminalAgent,
        ui: crate::theme::UiColors,
        cx: &mut Context<Self>,
    ) -> Option<AnyElement> {
        let inspection = self.agent_updates.get(&agent)?.inspection.as_ref()?;
        let executable = inspection.executable.as_ref()?;
        let command = inspection
            .plan(self.selected_update_method(agent))
            .and_then(|plan| plan.as_ref().ok())
            .map(agent_updates::UpdatePlan::display_command);
        let copy = command.map(|command| {
            icon_button(
                SharedString::from(format!("agent-update-copy-{}", agent.tag())),
                "icons/settings/copy-01.svg",
                with_alpha(ui.text, 0.8),
                ui.text,
                ui,
            )
            .delayed_tooltip(text_tooltip(format!("Copy update command: {command}")))
            .on_click(cx.listener(move |_, _: &ClickEvent, _w, cx| {
                cx.write_to_clipboard(gpui::ClipboardItem::new_string(command.clone()));
            }))
            .into_any_element()
        });
        Some(detail_row(
            "Location",
            div()
                .min_w_0()
                .truncate()
                .child(mono_text(
                    executable.to_string_lossy().into_owned().into(),
                    ui.muted,
                ))
                .into_any_element(),
            copy,
            ui,
        ))
    }

    fn render_agent_hooks_row(
        &self,
        agent: TerminalAgent,
        ui: crate::theme::UiColors,
        cx: &mut Context<Self>,
    ) -> Option<AnyElement> {
        let slug = agent.runtime().slug;
        let status = self
            .integration_status
            .as_ref()?
            .iter()
            .find(|status| status.slug == slug)?;
        let installed = match status.state {
            IntegrationState::Installed => true,
            IntegrationState::NotInstalled => false,
            IntegrationState::UnsupportedPlatform | IntegrationState::DetectionOnly => {
                return None;
            }
        };
        let (text, color) = match self.integration_errors.get(slug) {
            Some(error) => (error.clone(), destructive_color()),
            None => match status.post_install_step.filter(|_| installed) {
                Some(step) => (step.to_string(), ui.muted),
                None => (
                    "Lets Paneflow resume its conversations and registers the Paneflow MCP server."
                        .to_string(),
                    ui.muted,
                ),
            },
        };
        let control = if self.integration_busy.as_deref() == Some(slug) {
            crate::ui_primitives::comet_spinner(
                SharedString::from(format!("integration-spinner-{slug}")),
                px(14.),
                ui.muted,
            )
        } else {
            let owned = slug.to_string();
            toggle_switch(
                SharedString::from(format!("integration-toggle-{slug}")),
                format!("Paneflow hooks for {}", agent.display_name()),
                installed,
                ui,
            )
            .cursor(CursorStyle::PointingHand)
            .on_click(cx.listener(move |this, _: &ClickEvent, _w, cx| {
                this.change_integration(owned.clone(), !installed, cx);
            }))
            .into_any_element()
        };
        Some(detail_row(
            "Paneflow hooks",
            detail_text(text, color),
            Some(control),
            ui,
        ))
    }

    fn render_agent_profiles_section(
        &self,
        ui: crate::theme::UiColors,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let entries: Vec<_> = self
            .cached_config
            .agent_profiles
            .iter()
            .enumerate()
            .filter(|(_, profile)| {
                self.cached_config
                    .agent_profile_applies(profile, self.agent_profiles_project.as_deref())
            })
            .collect();
        let editor = self.agent_profile_editor.as_ref();
        let creating = editor.is_some_and(|editor| editor.index.is_none());

        let mut card = setting_card(ui).py(px(CARD_PADDING_Y));
        for (position, &(idx, entry)) in entries.iter().enumerate() {
            if position > 0 {
                card = card.child(hairline_inset(ui));
            }
            let editing = editor.is_some_and(|editor| editor.index == Some(idx));
            card = card.child(self.render_agent_profile_row(idx, entry, editing, ui, cx));
            if editing && let Some(editor) = editor {
                card = card.child(self.render_agent_profile_editor(editor, ui, cx));
            }
        }
        if creating && let Some(editor) = editor {
            if !entries.is_empty() {
                card = card.child(hairline_inset(ui));
            }
            card = card
                .child(
                    div()
                        .flex()
                        .flex_row()
                        .items_center()
                        .gap(px(12.))
                        .px(px(CARD_PADDING_X))
                        .py(px(10.))
                        .child(agent_icon_el(editor.agent, ui))
                        .child(
                            div()
                                .flex_1()
                                .min_w_0()
                                .flex()
                                .flex_col()
                                .gap(px(1.))
                                .child(
                                    div()
                                        .text_size(BODY)
                                        .text_color(ui.muted)
                                        .child("New profile"),
                                )
                                .child(
                                    div()
                                        .text_size(LABEL_SM)
                                        .text_color(ui.muted)
                                        .child("Pick a base agent, then name it."),
                                ),
                        ),
                )
                .child(self.render_agent_profile_editor(editor, ui, cx));
        }

        if !creating {
            if !entries.is_empty() {
                card = card.child(hairline(ui));
            }
            card = card.child(
                div()
                    .id("agent-profile-new")
                    .h(px(36.))
                    .px(px(CARD_PADDING_X))
                    .flex()
                    .flex_row()
                    .items_center()
                    .cursor(CursorStyle::PointingHand)
                    .text_size(BODY)
                    .text_color(ui.muted)
                    .hover(move |style| style.text_color(ui.text).bg(with_alpha(ui.text, 0.05)))
                    .on_click(cx.listener(|this, _: &ClickEvent, _w, cx| {
                        this.open_agent_profile_editor(None, cx);
                    }))
                    .child("+ New profile"),
            );
            if self.agent_profiles_project.is_none() {
                card = card.child(
                    div()
                        .id("agent-profile-agency")
                        .accessible_control(gpui::accesskit::Role::Button, "Add Agency Agents team")
                        .h(px(36.))
                        .px(px(CARD_PADDING_X))
                        .flex()
                        .items_center()
                        .cursor(CursorStyle::PointingHand)
                        .text_size(BODY)
                        .text_color(ui.muted)
                        .hover(move |style| style.text_color(ui.text).bg(with_alpha(ui.text, 0.05)))
                        .on_press(cx.listener(|this, _, _window, cx| this.add_agency_profiles(cx)))
                        .child("+ Add Agency Agents team"),
                );
            }
        }

        let mut block = Block::new("Profiles")
            .top_gap(24.)
            .child(section_title(
                ui,
                if self.agent_profiles_project.is_some() { "Project agents" } else { "Default profiles" },
                Some(if self.agent_profiles_project.is_some() { "Customize a default for this project, or add another agent. Defaults stay unchanged." } else { "Default agents and skills, available to every project until customized." }),
            ))
            .child(card);
        if self.agent_profiles_project.is_none() {
            block = block.child(
                div().mt(px(12.)).flex().gap(px(8.))
                    .child(soft_button("agent-setup-export", "Export setup…", ui,
                        cx.listener(|this, _, _, cx| this.export_agent_setup(cx))))
                    .child(soft_button("agent-setup-import", "Import setup…", ui,
                        cx.listener(|this, _, _, cx| this.choose_agent_setup(cx))))
            ).child(detail_text("JSON includes default and project profiles, environment values, arguments and skill paths. Skill files and agent sign-ins stay on this machine.".to_string(), ui.muted));
            if let Some(review) = &self.agent_setup_review {
                let mut rows = div()
                    .id("agent-setup-review-list")
                    .max_h(px(240.))
                    .overflow_y_scroll()
                    .flex()
                    .flex_col()
                    .gap(px(8.));
                for profile in &review.incoming {
                    rows = rows.child(div().flex().flex_col()
                        .child(div().text_size(BODY).text_color(ui.text).child(profile.name.clone()))
                        .child(detail_text(format!("{} · {} · {} effort · {} · {} skills · {} environment values · {} arguments",
                            profile.agent, profile.model.as_deref().unwrap_or("Agent default"),
                            profile.effort.as_deref().unwrap_or("default"),
                            profile.project.as_deref().map(|p| p.display().to_string()).unwrap_or_else(|| "All projects".into()),
                            profile.skills.len(), profile.env.len(), profile.args.len()), ui.muted))
                        .child(detail_text(serde_json::to_string(profile).unwrap_or_else(|error| format!("Cannot display profile: {error}")), ui.muted)));
                }
                block = block.child(setting_card(ui).p(px(16.)).flex().flex_col().gap(px(12.))
                    .child(div().text_size(BODY_EMPHASIS).text_color(ui.text).child(format!("Import review: {} added, {} updated", review.added, review.updated)))
                    .child(detail_text(review.source.display().to_string(), ui.muted))
                    .child(detail_text(format!("Matches use profile name and project. Other profiles stay unchanged. {} unavailable skill files will need their paths updated. A backup is saved before applying.", review.missing_skills), ui.muted))
                    .child(rows)
                    .child(div().flex().gap(px(8.))
                        .child(soft_button("agent-setup-apply", "Apply import", ui, cx.listener(|this, _, _, cx| this.apply_agent_setup(cx))))
                        .child(soft_button("agent-setup-cancel", "Cancel", ui, cx.listener(|this, _, _, cx| {
                            if !this.agent_profiles_busy { this.agent_setup_review = None; cx.notify(); }
                        }))))) ;
            }
        }
        block.finish()
    }

    fn agent_setup_available(&mut self, cx: &mut Context<Self>) -> bool {
        if self.agent_profiles_busy || self.pending_settings.contains_key("agent_profiles") {
            self.show_toast("Wait for agent profiles to finish saving", cx);
            return false;
        }
        if self.agent_profile_editor.is_some() {
            self.show_toast("Save or cancel the profile draft first", cx);
            return false;
        }
        true
    }

    fn export_agent_setup(&mut self, cx: &mut Context<Self>) {
        if !self.agent_setup_available(cx) {
            return;
        }
        let Some(directory) = dirs::download_dir().or_else(dirs::home_dir) else {
            self.show_toast("Cannot determine an export directory", cx);
            return;
        };
        let receiver = cx.prompt_for_new_path(&directory, Some("paneflow-agent-setup.json"));
        self.agent_profiles_busy = true;
        cx.spawn(async move |this, cx| {
            let result = match receiver.await {
                Ok(Ok(Some(path))) => {
                    smol::unblock(move || {
                        let current = crate::config_writer::agent_profiles_snapshot()?;
                        let (profiles, _, _) = crate::agent_setup::merge(&current, &[])?;
                        crate::agent_setup::export(&path, &profiles)?;
                        Ok(Some(path))
                    })
                    .await
                }
                Ok(Ok(None)) => Ok(None),
                Ok(Err(error)) => Err(error.to_string()),
                Err(error) => Err(error.to_string()),
            };
            let _ = this.update(cx, |this, cx| {
                this.agent_profiles_busy = false;
                match result {
                    Ok(Some(path)) => {
                        this.show_toast(format!("Agent setup exported to {}", path.display()), cx)
                    }
                    Ok(None) => cx.notify(),
                    Err(error) => this.show_toast(format!("Could not export setup: {error}"), cx),
                }
            });
        })
        .detach();
    }

    fn choose_agent_setup(&mut self, cx: &mut Context<Self>) {
        if !self.agent_setup_available(cx) {
            return;
        }
        let receiver = cx.prompt_for_paths(PathPromptOptions {
            files: true,
            directories: false,
            multiple: false,
            prompt: Some("Import agent setup JSON".into()),
        });
        self.agent_profiles_busy = true;
        self.agent_setup_review = None;
        cx.spawn(async move |this, cx| {
            let result = match receiver.await {
                Ok(Ok(Some(mut paths))) if paths.len() == 1 => {
                    let path = paths.remove(0);
                    smol::unblock(move || {
                        crate::agent_setup::read_review(
                            path,
                            crate::config_writer::agent_profiles_snapshot()?,
                        )
                        .map(Some)
                    })
                    .await
                }
                Ok(Ok(_)) => Ok(None),
                Ok(Err(error)) => Err(error.to_string()),
                Err(error) => Err(error.to_string()),
            };
            let _ = this.update(cx, |this, cx| {
                this.agent_profiles_busy = false;
                match result {
                    Ok(review) => {
                        if this.settings_section == Some(crate::SettingsSection::Agents)
                            && this.agent_profiles_project.is_none()
                        {
                            this.agent_setup_review = review;
                        }
                        cx.notify();
                    }
                    Err(error) => this.show_toast(format!("Could not import setup: {error}"), cx),
                }
            });
        })
        .detach();
    }

    fn apply_agent_setup(&mut self, cx: &mut Context<Self>) {
        if !self.agent_setup_available(cx) {
            return;
        }
        let Some(review) = self.agent_setup_review.take() else {
            return;
        };
        self.agent_profiles_busy = true;
        cx.spawn(async move |this, cx| {
            let (review, result) = smol::unblock(move || {
                let result = crate::config_writer::import_agent_setup(&review);
                (review, result)
            })
            .await;
            let _ = this.update(cx, |this, cx| {
                this.agent_profiles_busy = false;
                match result {
                    Ok((profiles, backup)) => {
                        this.cached_config.agent_profiles = profiles;
                        crate::config_snapshot::publish(&this.cached_config, cx);
                        this.refresh_usage_inventory(false, cx);
                        this.show_toast(
                            format!("Agent setup imported. Backup: {}", backup.display()),
                            cx,
                        );
                    }
                    Err(error) => {
                        if this.settings_section == Some(crate::SettingsSection::Agents)
                            && this.agent_profiles_project.is_none()
                        {
                            this.agent_setup_review = Some(review);
                        }
                        this.show_toast(format!("Could not apply setup: {error}"), cx);
                    }
                }
                cx.notify();
            });
        })
        .detach();
    }

    fn render_agent_profile_row(
        &self,
        idx: usize,
        entry: &AgentProfileConfig,
        editing: bool,
        ui: crate::theme::UiColors,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let group = SharedString::from(format!("agent-profile-row-{idx}"));
        let mut meta = div()
            .flex()
            .flex_row()
            .items_center()
            .gap(px(6.))
            .min_w_0()
            .overflow_hidden()
            .text_size(LABEL_SM)
            .text_color(ui.muted)
            .whitespace_nowrap();
        let icon = match AgentProfile::for_project(
            entry,
            &self.cached_config,
            self.agent_profiles_project.as_deref(),
        ) {
            Ok(profile) => {
                meta = meta.child(profile.agent.display_name());
                if let Some(role) = entry
                    .agency_role
                    .as_deref()
                    .and_then(paneflow_mcp_install::agency::role)
                {
                    meta = meta.child(chip(role.name.to_string(), ui));
                }
                if let Some(model) = &entry.model {
                    meta = meta.child(chip(
                        if model == ORCHESTRATOR_MODEL {
                            "Orchestrator model".to_string()
                        } else {
                            model.clone()
                        },
                        ui,
                    ));
                }
                if let Some(effort) = &entry.effort {
                    meta = meta.child(chip(format!("{effort} effort"), ui));
                }
                meta = meta.child(chip(
                    entry
                        .project
                        .as_deref()
                        .map(profile_project_label)
                        .unwrap_or_else(|| "Default".to_string()),
                    ui,
                ));
                if !profile.env.is_empty() || !entry.args.is_empty() {
                    meta = meta.child("·");
                }
                for (key, value) in &profile.env {
                    meta = meta.child(chip(format!("{key}={value}"), ui));
                }
                if !entry.args.is_empty() {
                    meta = meta.child(chip(entry.args.join(" "), ui));
                }
                agent_icon_el(profile.agent, ui)
            }
            Err(reason) => {
                meta = meta
                    .text_color(destructive_color())
                    .child(format!("Invalid profile: {reason}"));
                div().size(px(ROW_ICON)).flex_none().into_any_element()
            }
        };

        let actions = div()
            .flex()
            .flex_row()
            .items_center()
            .gap(px(2.))
            .flex_shrink_0()
            .when(!editing, |d| d.invisible())
            .group_hover(group.clone(), |style| style.visible())
            .child(
                icon_button(
                    SharedString::from(format!("agent-profile-edit-{idx}")),
                    "icons/settings/pencil-edit-02.svg",
                    with_alpha(ui.text, 0.8),
                    ui.text,
                    ui,
                )
                .on_click(cx.listener(move |this, _: &ClickEvent, _w, cx| {
                    this.open_agent_profile_editor(Some(idx), cx)
                })),
            )
            .when(
                !(self.agent_profiles_project.is_some() && entry.project.is_none()),
                |actions| {
                    actions.child(
                        icon_button(
                            SharedString::from(format!("agent-profile-delete-{idx}")),
                            "icons/settings/delete-02.svg",
                            with_alpha(ui.text, 0.8),
                            destructive_color(),
                            ui,
                        )
                        .on_click(cx.listener(
                            move |this, _: &ClickEvent, _w, cx| this.delete_agent_profile(idx, cx),
                        )),
                    )
                },
            );

        div()
            .group(group)
            .flex()
            .flex_row()
            .items_center()
            .gap(px(12.))
            .px(px(CARD_PADDING_X))
            .py(px(10.))
            .child(icon)
            .child(
                div()
                    .flex_1()
                    .min_w_0()
                    .flex()
                    .flex_col()
                    .gap(px(1.))
                    .child(
                        div()
                            .text_size(BODY)
                            .font_weight(gpui::FontWeight::MEDIUM)
                            .text_color(ui.text)
                            .truncate()
                            .child(entry.name.clone()),
                    )
                    .child(meta),
            )
            .child(actions)
            .into_any_element()
    }

    fn render_agent_profile_editor(
        &self,
        editor: &AgentProfileEditor,
        ui: crate::theme::UiColors,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let base_select = self.render_agent_profile_base_select(editor, ui, cx);
        let role_select = self.render_agency_role_select(editor, ui, cx);
        let effort_select = self.render_agent_profile_effort_select(editor, ui, cx);
        let model_select = self.render_agent_profile_model_select(editor, ui, cx);
        let mut skills = div().flex().flex_col().gap(px(6.));
        for (index, path) in editor.skills.iter().enumerate() {
            let name = crate::agent_profile_context::skill_label(path);
            skills = skills.child(
                div()
                    .flex()
                    .items_center()
                    .gap(px(6.))
                    .child(
                        div()
                            .flex_1()
                            .min_w_0()
                            .text_size(BODY)
                            .text_color(ui.text)
                            .truncate()
                            .id(SharedString::from(format!("agent-skill-path-{index}")))
                            .delayed_tooltip(text_tooltip(path.display().to_string()))
                            .child(name),
                    )
                    .child(
                        ghost_icon_button(
                            SharedString::from(format!("agent-skill-remove-{index}")),
                            "Remove skill",
                            "icons/settings/delete-02.svg",
                            ui,
                        )
                        .on_click(cx.listener(move |this, _, _, cx| {
                            if let Some(editor) = this.agent_profile_editor.as_mut()
                                && index < editor.skills.len()
                            {
                                editor.skills.remove(index);
                            }
                            cx.notify();
                        })),
                    ),
            );
        }
        skills = skills.child(
            div()
                .flex()
                .items_center()
                .gap(px(6.))
                .child(
                    div()
                        .flex_1()
                        .min_w_0()
                        .on_key_down(cx.listener(|this, event: &gpui::KeyDownEvent, _, cx| {
                            if event.keystroke.key == "enter" {
                                this.attach_agent_profile_skill_query(cx);
                                cx.stop_propagation();
                            }
                        }))
                        .child(input_box(editor.skill_input.clone(), false, ui)),
                )
                .child(soft_button(
                    "agent-skill-attach",
                    "Add",
                    ui,
                    cx.listener(|this, _, _, cx| this.attach_agent_profile_skill_query(cx)),
                )),
        );
        skills = skills.child(
            div()
                .flex()
                .items_center()
                .gap(px(8.))
                .child(
                    ghost_icon_button(
                        "agent-skill-browse",
                        "Browse…",
                        "icons/settings/folder-open.svg",
                        ui,
                    )
                    .on_click(cx.listener(|this, _, _, cx| this.choose_agent_profile_skills(cx))),
                )
                .child(
                    ghost_icon_button(
                        "agent-skill-refresh",
                        "Refresh local skills",
                        "icons/refresh.svg",
                        ui,
                    )
                    .on_click(cx.listener(|this, _, _, cx| this.refresh_agent_profile_skills(cx))),
                ),
        );
        let skill_query = editor.skill_input.read(cx).value();
        if !skill_query.trim().is_empty()
            && !crate::agent_profile_context::is_skill_path_query(&skill_query)
        {
            let matches = crate::agent_profile_context::matching_skills(
                &editor.available_skills,
                &skill_query,
            );
            for (index, choice) in matches.iter().take(8).enumerate() {
                let path = choice.path.clone();
                skills = skills.child(
                    select_option(
                        SharedString::from(format!("agent-local-skill-{index}")),
                        editor.skills.contains(&path),
                        ui,
                    )
                    .h(px(AGENT_ROW_HEIGHT))
                    .text_color(ui.text)
                    .accessible_control(
                        gpui::accesskit::Role::MenuItem,
                        SharedString::from(format!("{} {}", choice.name, path.display())),
                    )
                    .on_press(cx.listener(move |this, _, _, cx| {
                        this.attach_agent_profile_skill_paths(vec![path.clone()], cx)
                    }))
                    .child(
                        div()
                            .flex_col()
                            .min_w_0()
                            .child(div().truncate().child(choice.name.clone()))
                            .child(
                                div()
                                    .text_size(LABEL_SM)
                                    .text_color(ui.muted)
                                    .truncate()
                                    .child(choice.path.display().to_string()),
                            ),
                    ),
                );
            }
            if editor.skills_loading || matches.is_empty() || matches.len() > 8 {
                skills = skills.child(div().text_size(LABEL_SM).text_color(ui.muted).child(
                    if editor.skills_loading {
                        "Finding local skills…"
                    } else if matches.is_empty() {
                        "No matching local skill. Paste a Markdown path or use Browse."
                    } else {
                        "Showing the first 8 matches. Refine your search to see more."
                    },
                ));
            }
        }

        let home = dirs::home_dir();
        let row_values: Vec<String> = editor
            .env_rows
            .iter()
            .map(|row| row.value.read(cx).value())
            .collect();

        let mut env_column = div().flex().flex_col().gap(px(6.)).w_full();
        for (row_idx, row) in editor.env_rows.iter().enumerate() {
            let removable = editor.env_rows.len() > 1;
            let hint = row_values
                .get(row_idx)
                .and_then(|value| env_value_hint(value, home.as_deref(), ui));
            env_column = env_column.child(
                div()
                    .flex()
                    .flex_col()
                    .gap(px(3.))
                    .w_full()
                    .child(
                        div()
                            .flex()
                            .flex_row()
                            .items_center()
                            .gap(px(6.))
                            .w_full()
                            .child(div().w(px(ENV_NAME_WIDTH)).flex_none().child(input_box(
                                row.key.clone(),
                                true,
                                ui,
                            )))
                            .child(div().flex_1().min_w_0().child(input_box(
                                row.value.clone(),
                                true,
                                ui,
                            )))
                            .when(removable, |d| {
                                d.child(
                                    icon_button(
                                        SharedString::from(format!(
                                            "agent-profile-env-remove-{row_idx}"
                                        )),
                                        "icons/settings/delete-02.svg",
                                        with_alpha(ui.text, 0.8),
                                        destructive_color(),
                                        ui,
                                    )
                                    .on_click(cx.listener(
                                        move |this, _: &ClickEvent, _w, cx| {
                                            if let Some(editor) = this.agent_profile_editor.as_mut()
                                                && row_idx < editor.env_rows.len()
                                            {
                                                editor.env_rows.remove(row_idx);
                                                cx.notify();
                                            }
                                        },
                                    )),
                                )
                            }),
                    )
                    .when_some(hint, |d, hint| d.child(hint)),
            );
        }
        env_column = env_column.child(
            div().flex().flex_row().child(
                ghost_icon_button(
                    "agent-profile-env-add",
                    "Add variable",
                    "icons/settings/add-01.svg",
                    ui,
                )
                .on_click(cx.listener(|this, _: &ClickEvent, _w, cx| {
                    this.add_agent_profile_env_row(cx);
                })),
            ),
        );

        let footer_text: AnyElement = match editor.error.as_ref() {
            Some(error) => div()
                .flex_1()
                .min_w_0()
                .text_size(LABEL_SM)
                .text_color(destructive_color())
                .child(error.clone())
                .into_any_element(),
            None => {
                let preview = self.agent_profile_command_preview(editor, cx);
                div()
                    .flex_1()
                    .min_w_0()
                    .flex()
                    .flex_row()
                    .items_center()
                    .gap(px(4.))
                    .text_size(LABEL_SM)
                    .text_color(ui.muted)
                    .whitespace_nowrap()
                    .overflow_hidden()
                    .child("Launches")
                    .child(mono_text(SharedString::from(preview), ui.text))
                    .into_any_element()
            }
        };

        let save = raised_button("agent-profile-save", "Save", ui.text, ui.base).on_click(
            cx.listener(|this, _: &ClickEvent, _w, cx| {
                this.save_agent_profile(cx);
            }),
        );

        div()
            .flex()
            .flex_col()
            .child(hairline(ui))
            .child(editor_row(
                "Base agent",
                "Status, hooks, and sessions follow it.",
                div().w(px(CONTROL_WIDTH)).child(base_select),
                ui,
            ))
            .child(hairline(ui))
            .child(editor_row(
                "Name",
                "Shown in the launcher.",
                div().w(px(CONTROL_WIDTH)).child(input_box(
                    self.agent_profile_name_input.clone(),
                    false,
                    ui,
                )),
                ui,
            ))
            .child(hairline(ui))
            .when(matches!(editor.agent, TerminalAgent::ClaudeCode | TerminalAgent::Codex), |column| {
                column
                    .child(editor_row("Agency role", "Choose an expertise from Agency Agents, or leave it unset.", div().w(px(CONTROL_WIDTH)).child(role_select), ui))
                    .child(hairline(ui))
            })
            .when(matches!(editor.agent, TerminalAgent::ClaudeCode | TerminalAgent::Codex), |column| {
                column
                    .child(editor_stacked_row("Skills", "Find a local skill by name or paste a Markdown file path. Its guidance is added to new sessions; resources stay in place.", skills, ui))
                    .child(hairline(ui))
                    .child(editor_row(
                        "Model",
                        "Choose a model, use the account default, or enter another model ID.",
                        div().w(px(CONTROL_WIDTH)).child(model_select),
                        ui,
                    ))
                    .child(hairline(ui))
                    .child(editor_row("Effort", "Choose a supported level, or use the agent default. Model support varies.", div().w(px(CONTROL_WIDTH)).child(effort_select), ui))
                    .child(hairline(ui))
            })
            .child(editor_stacked_row(
                "Environment variables",
                "Set on the agent process. A leading ~ is your home, and $NAME, ${NAME} or \
                 %NAME% takes the value from your environment.",
                env_column,
                ui,
            ))
            .child(hairline(ui))
            .child(editor_row(
                "Extra arguments",
                "Appended after the agent's own flags.",
                div().w(px(CONTROL_WIDTH)).child(input_box(
                    self.agent_profile_args_input.clone(),
                    true,
                    ui,
                )),
                ui,
            ))
            .child(hairline(ui))
            .child(
                div()
                    .flex()
                    .flex_row()
                    .items_center()
                    .justify_between()
                    .gap(px(12.))
                    .px(px(CARD_PADDING_X))
                    .py(px(10.))
                    .child(footer_text)
                    .child(
                        div()
                            .flex()
                            .flex_row()
                            .gap(px(6.))
                            .flex_shrink_0()
                            .child(soft_button(
                                "agent-profile-cancel",
                                "Cancel",
                                ui,
                                cx.listener(|this, _: &ClickEvent, _w, cx| {
                                    this.close_agent_profile_editor(cx);
                                }),
                            ))
                            .child(save),
                    ),
            )
            .into_any_element()
    }

    fn apply_agent_profile_skill_paths(
        &mut self,
        identity: gpui::EntityId,
        result: std::io::Result<Vec<PathBuf>>,
        cx: &mut Context<Self>,
    ) {
        let Some(editor) = self
            .agent_profile_editor
            .as_mut()
            .filter(|editor| editor.model_input.entity_id() == identity)
        else {
            return;
        };
        match result {
            Ok(paths) => {
                let mut selected = editor.skills.clone();
                for path in paths {
                    if !selected.contains(&path) {
                        selected.push(path);
                    }
                }
                match crate::agent_profile_context::validate_skill_selection(&selected) {
                    Ok(()) => {
                        editor.skills = selected;
                        editor.error = None;
                        editor
                            .skill_input
                            .update(cx, |input, cx| input.set_value(SharedString::from(""), cx));
                    }
                    Err(error) => editor.error = Some(error),
                }
            }
            Err(error) => editor.error = Some(format!("Could not attach skill: {error}")),
        }
        cx.notify();
    }

    fn attach_agent_profile_skill_paths(&mut self, paths: Vec<PathBuf>, cx: &mut Context<Self>) {
        let Some(editor) = self.agent_profile_editor.as_ref() else {
            return;
        };
        let identity = editor.model_input.entity_id();
        let base = editor
            .project
            .clone()
            .or_else(|| self.active_profile_project());
        cx.spawn(async move |this, cx| {
            let result = smol::unblock(move || {
                crate::agent_profile_context::prepare_skill_paths(paths, base.as_deref())
            })
            .await;
            let _ = this.update(cx, |this, cx| {
                this.apply_agent_profile_skill_paths(identity, result, cx)
            });
        })
        .detach();
    }

    fn attach_agent_profile_skill_query(&mut self, cx: &mut Context<Self>) {
        let Some(editor) = self.agent_profile_editor.as_ref() else {
            return;
        };
        let query = editor.skill_input.read(cx).value();
        if crate::agent_profile_context::is_skill_path_query(&query) {
            match crate::agent_profile_context::pasted_skill_path(&query) {
                Ok(path) => self.attach_agent_profile_skill_paths(vec![path], cx),
                Err(error) => {
                    if let Some(editor) = self.agent_profile_editor.as_mut() {
                        editor.error = Some(error.to_string());
                    }
                    cx.notify();
                }
            }
            return;
        }
        let matches =
            crate::agent_profile_context::matching_skills(&editor.available_skills, &query);
        if !query.trim().is_empty() && !editor.skills_loading && matches.len() == 1 {
            let path = matches[0].path.clone();
            self.attach_agent_profile_skill_paths(vec![path], cx);
        } else {
            let message = if editor.skills_loading {
                "Local skills are still loading; choose a result when ready"
            } else if matches.is_empty() {
                "No matching local skill. Paste a Markdown file path or use Browse"
            } else {
                "Choose a matching skill from the list, or refine your search"
            };
            if let Some(editor) = self.agent_profile_editor.as_mut() {
                editor.error = Some(message.to_string());
            }
            cx.notify();
        }
    }

    fn refresh_agent_profile_skills(&mut self, cx: &mut Context<Self>) {
        let Some(editor) = self.agent_profile_editor.as_mut() else {
            return;
        };
        editor.skills_generation = editor.skills_generation.wrapping_add(1);
        let generation = editor.skills_generation;
        editor.skills_loading = true;
        let identity = editor.model_input.entity_id();
        let project = editor.project.clone();
        let env = collect_env(
            &editor
                .env_rows
                .iter()
                .map(|row| (row.key.read(cx).value(), row.value.read(cx).value()))
                .collect::<Vec<_>>(),
        )
        .unwrap_or_default();
        cx.spawn(async move |this, cx| {
            let choices =
                smol::unblock(move || crate::agent_profile_context::installed_skills(project, env))
                    .await;
            let _ = this.update(cx, |this, cx| {
                if let Some(editor) = this.agent_profile_editor.as_mut()
                    && editor.model_input.entity_id() == identity
                    && editor.skills_generation == generation
                {
                    editor.available_skills = choices;
                    editor.skills_loading = false;
                    cx.notify();
                }
            });
        })
        .detach();
        cx.notify();
    }

    fn choose_agent_profile_skills(&mut self, cx: &mut Context<Self>) {
        let Some(editor) = self.agent_profile_editor.as_ref() else {
            return;
        };
        let identity = editor.model_input.entity_id();
        let receiver = cx.prompt_for_paths(PathPromptOptions {
            files: true,
            directories: true,
            multiple: true,
            prompt: Some("Attach Markdown skill".into()),
        });
        cx.spawn(async move |this, cx| {
            let Ok(Ok(Some(paths))) = receiver.await else {
                return;
            };
            let result = smol::unblock(move || {
                crate::agent_profile_context::prepare_skill_paths(paths, None)
            })
            .await;
            let _ = this.update(cx, |this, cx| {
                this.apply_agent_profile_skill_paths(identity, result, cx)
            });
        })
        .detach();
    }

    fn refresh_agent_profile_models(&mut self, cx: &mut Context<Self>) {
        let Some(editor) = self.agent_profile_editor.as_mut() else {
            return;
        };
        editor.model_catalog_generation = editor.model_catalog_generation.wrapping_add(1);
        let generation = editor.model_catalog_generation;
        let identity = editor.model_input.entity_id();
        let agent = editor.agent;
        let env = collect_env(
            &editor
                .env_rows
                .iter()
                .map(|row| (row.key.read(cx).value(), row.value.read(cx).value()))
                .collect::<Vec<_>>(),
        )
        .unwrap_or_default();
        cx.spawn(async move |this, cx| {
            let choices =
                smol::unblock(move || crate::agent_profile_context::model_choices(agent, env))
                    .await;
            let _ = this.update(cx, |this, cx| {
                if let Some(editor) = this.agent_profile_editor.as_mut()
                    && editor.model_input.entity_id() == identity
                    && editor.agent == agent
                    && editor.model_catalog_generation == generation
                {
                    let current = editor.model_input.read(cx).value();
                    if choices.iter().any(|choice| choice.id == current) {
                        editor.custom_model = false;
                    }
                    editor.models = choices;
                    cx.notify();
                }
            });
        })
        .detach();
    }

    fn render_agent_profile_model_select(
        &self,
        editor: &AgentProfileEditor,
        ui: crate::theme::UiColors,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let open = editor.model_menu_open;
        let current = editor.model_input.read(cx).value();
        let label = if current == ORCHESTRATOR_MODEL {
            "Default (Orchestrator’s model)".to_string()
        } else if editor.custom_model {
            "Other model…".to_string()
        } else {
            editor
                .models
                .iter()
                .find(|choice| choice.id == current)
                .map(|choice| choice.label.clone())
                .unwrap_or_else(|| {
                    if current.is_empty() {
                        "Account default".to_string()
                    } else {
                        current.clone()
                    }
                })
        };
        let mut trigger = select_trigger("agent-profile-model", "Model", open, ui)
            .w_full()
            .max_w(px(CONTROL_WIDTH))
            .h(px(INPUT_HEIGHT))
            .py(px(0.))
            .on_press(cx.listener(move |this, _, _, cx| {
                if let Some(editor) = this.agent_profile_editor.as_mut() {
                    editor.model_menu_open = !open;
                    editor.role_menu_open = false;
                    editor.effort_menu_open = false;
                    editor.agent_menu_open = false;
                }
                cx.notify();
            }))
            .child(
                div()
                    .flex_1()
                    .min_w_0()
                    .truncate()
                    .text_size(BODY)
                    .text_color(ui.text)
                    .child(label),
            )
            .child(select_chevron(ui));
        if open {
            let mut menu = select_menu("agent-profile-model-menu", ui).on_mouse_down_out(
                cx.listener(|this, _, _, cx| {
                    if let Some(editor) = this.agent_profile_editor.as_mut() {
                        editor.model_menu_open = false;
                    }
                    cx.notify();
                }),
            );
            let options = (editor.project.is_some()
                && editor.agency_role.as_deref() != Some("orchestrator"))
            .then(|| {
                (
                    ORCHESTRATOR_MODEL.to_string(),
                    "Default (Orchestrator’s model)".to_string(),
                    false,
                )
            })
            .into_iter()
            .chain(std::iter::once((
                String::new(),
                "Account default".to_string(),
                false,
            )))
            .chain(
                editor
                    .models
                    .iter()
                    .map(|choice| (choice.id.clone(), choice.label.clone(), false)),
            )
            .chain(std::iter::once((
                String::new(),
                "Other model…".to_string(),
                true,
            )));
            for (index, (id, label, custom)) in options.enumerate() {
                menu = menu.child(
                    select_option(
                        SharedString::from(format!("agent-profile-model-option-{index}")),
                        if custom {
                            editor.custom_model
                        } else {
                            !editor.custom_model && current == id
                        },
                        ui,
                    )
                    .text_color(ui.text)
                    .accessible_control(
                        gpui::accesskit::Role::MenuItem,
                        SharedString::from(label.clone()),
                    )
                    .on_press(cx.listener(move |this, _, window, cx| {
                        if let Some(editor) = this.agent_profile_editor.as_mut() {
                            let effective_id = if id == ORCHESTRATOR_MODEL {
                                match editor
                                    .project
                                    .as_deref()
                                    .ok_or_else(|| {
                                        "Choose this option in project settings".to_string()
                                    })
                                    .and_then(|project| {
                                        AgentProfile::orchestrator_model(
                                            &this.cached_config,
                                            project,
                                            editor.agent,
                                        )
                                    }) {
                                    Ok(model) => model,
                                    Err(error) => {
                                        editor.error = Some(error);
                                        editor.model_menu_open = false;
                                        cx.notify();
                                        return;
                                    }
                                }
                            } else {
                                id.clone()
                            };
                            editor.custom_model = custom;
                            editor.model_menu_open = false;
                            editor.model_input.update(cx, |input, cx| {
                                input.set_value(SharedString::from(id.clone()), cx)
                            });
                            if let Some(choice) = editor
                                .models
                                .iter()
                                .find(|choice| choice.id == effective_id)
                                && !choice.efforts.is_empty()
                                && editor
                                    .effort
                                    .as_ref()
                                    .is_some_and(|effort| !choice.efforts.contains(effort))
                            {
                                editor.effort = None;
                            }
                            if custom {
                                let focus = editor.model_input.read(cx).focus_handle.clone();
                                focus.focus(window, cx);
                            }
                            editor.error = None;
                        }
                        cx.notify();
                    }))
                    .child(label),
                );
            }
            trigger = trigger.child(deferred_select_menu(menu));
        }
        div()
            .relative()
            .w_full()
            .flex()
            .flex_col()
            .gap(px(6.))
            .child(trigger)
            .when(editor.custom_model, |column| {
                column.child(input_box(editor.model_input.clone(), false, ui))
            })
            .into_any_element()
    }

    fn render_agent_profile_effort_select(
        &self,
        editor: &AgentProfileEditor,
        ui: crate::theme::UiColors,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let open = editor.effort_menu_open;
        let mut model_id = editor.model_input.read(cx).value();
        if model_id == ORCHESTRATOR_MODEL {
            model_id = editor
                .project
                .as_deref()
                .and_then(|project| {
                    AgentProfile::orchestrator_model(&self.cached_config, project, editor.agent)
                        .ok()
                })
                .unwrap_or_default();
        }
        let efforts: Vec<String> = editor
            .models
            .iter()
            .find(|model| model.id == model_id)
            .filter(|model| !model.efforts.is_empty())
            .map(|model| model.efforts.clone())
            .unwrap_or_else(|| {
                editor
                    .agent
                    .effort_levels()
                    .iter()
                    .map(|level| (*level).to_string())
                    .collect()
            });
        let mut trigger = select_trigger("agent-profile-effort", "Reasoning effort", open, ui)
            .w_full()
            .max_w(px(CONTROL_WIDTH))
            .h(px(INPUT_HEIGHT))
            .py(px(0.))
            .on_press(cx.listener(move |this, _, _, cx| {
                if let Some(editor) = this.agent_profile_editor.as_mut() {
                    editor.effort_menu_open = !open;
                    editor.model_menu_open = false;
                    editor.agent_menu_open = false;
                    editor.role_menu_open = false;
                }
                cx.notify();
            }))
            .child(
                div()
                    .flex_1()
                    .min_w_0()
                    .truncate()
                    .text_size(BODY)
                    .text_color(ui.text)
                    .child(
                        editor
                            .effort
                            .clone()
                            .unwrap_or_else(|| "Agent default".to_string()),
                    ),
            )
            .child(select_chevron(ui));
        if open {
            let mut menu = select_menu("agent-profile-effort-menu", ui).on_mouse_down_out(
                cx.listener(|this, _, _, cx| {
                    if let Some(editor) = this.agent_profile_editor.as_mut() {
                        editor.effort_menu_open = false;
                    }
                    cx.notify();
                }),
            );
            for value in std::iter::once(None).chain(efforts.into_iter().map(Some)) {
                let label = value.clone().unwrap_or_else(|| "Agent default".to_string());
                menu = menu.child(
                    select_option(
                        SharedString::from(format!("agent-profile-effort-{label}")),
                        editor.effort == value,
                        ui,
                    )
                    .text_color(ui.text)
                    .accessible_control(
                        gpui::accesskit::Role::MenuItem,
                        SharedString::from(label.clone()),
                    )
                    .on_press(cx.listener(move |this, _, _, cx| {
                        if let Some(editor) = this.agent_profile_editor.as_mut() {
                            editor.effort = value.clone();
                            editor.effort_menu_open = false;
                            editor.error = None;
                        }
                        cx.notify();
                    }))
                    .child(label),
                );
            }
            trigger = trigger.child(deferred_select_menu(menu));
        }
        div().relative().w_full().child(trigger).into_any_element()
    }

    fn render_agency_role_select(
        &self,
        editor: &AgentProfileEditor,
        ui: crate::theme::UiColors,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let selected = editor
            .agency_role
            .as_deref()
            .and_then(paneflow_mcp_install::agency::role);
        let open = editor.role_menu_open;
        let mut trigger = select_trigger("agent-profile-role", "Agency role", open, ui)
            .w_full()
            .max_w(px(CONTROL_WIDTH))
            .h(px(INPUT_HEIGHT))
            .py(px(0.))
            .on_press(cx.listener(move |this, _, window, cx| {
                if let Some(editor) = this.agent_profile_editor.as_mut() {
                    editor.role_menu_open = !open;
                    editor.model_menu_open = false;
                    editor.effort_menu_open = false;
                    editor.agent_menu_open = false;
                    if !open {
                        let focus = editor.role_search.read(cx).focus_handle.clone();
                        focus.focus(window, cx);
                    }
                }
                cx.notify();
            }))
            .child(
                div()
                    .flex_1()
                    .min_w_0()
                    .truncate()
                    .text_size(BODY)
                    .text_color(ui.text)
                    .child(selected.map_or("No Agency role", |role| role.name)),
            )
            .child(select_chevron(ui));
        if open {
            let query = editor.role_search.read(cx).value().to_lowercase();
            let mut menu = select_menu("agent-profile-role-menu", ui)
                .on_mouse_down_out(cx.listener(|this, _, _, cx| {
                    if let Some(editor) = this.agent_profile_editor.as_mut() {
                        editor.role_menu_open = false;
                    }
                    cx.notify();
                }))
                .child(
                    div()
                        .on_key_down(cx.listener(|this, event: &gpui::KeyDownEvent, window, cx| {
                            if event.keystroke.key != "enter" {
                                return;
                            }
                            if let Some(editor) = this.agent_profile_editor.as_mut() {
                                let query = editor.role_search.read(cx).value();
                                if !query.trim().is_empty() {
                                    if let Some(role) = matching_agency_roles(&query).first() {
                                        editor.agency_role = Some(role.id.to_string());
                                    } else {
                                        return;
                                    }
                                }
                                editor.role_menu_open = false;
                                editor.error = None;
                            }
                            this.update_agent_profile_suggested_name(cx);
                            this.settings_focus.focus(window, cx);
                            cx.stop_propagation();
                            cx.notify();
                        }))
                        .child(input_box(editor.role_search.clone(), false, ui)),
                )
                .child(
                    select_option("agent-profile-role-none", selected.is_none(), ui)
                        .text_color(ui.text)
                        .accessible_control(gpui::accesskit::Role::MenuItem, "No Agency role")
                        .on_press(cx.listener(|this, _, _, cx| {
                            if let Some(editor) = this.agent_profile_editor.as_mut() {
                                editor.agency_role = None;
                                editor.role_menu_open = false;
                                editor.error = None;
                            }
                            this.update_agent_profile_suggested_name(cx);
                            cx.notify();
                        }))
                        .child("No Agency role"),
                );
            for role in matching_agency_roles(&query) {
                let id = role.id;
                menu = menu.child(
                    select_option(
                        SharedString::from(format!("agency-role-{id}")),
                        editor.agency_role.as_deref() == Some(id),
                        ui,
                    )
                    .accessible_control(gpui::accesskit::Role::MenuItem, role.name)
                    .on_press(cx.listener(move |this, _, _, cx| {
                        if let Some(editor) = this.agent_profile_editor.as_mut() {
                            editor.agency_role = Some(id.to_string());
                            editor.role_menu_open = false;
                            editor.error = None;
                        }
                        this.update_agent_profile_suggested_name(cx);
                        cx.notify();
                    }))
                    .h(px(AGENT_ROW_HEIGHT))
                    .child(
                        div()
                            .flex_col()
                            .min_w_0()
                            .text_color(ui.text)
                            .child(div().truncate().child(role.name))
                            .child(
                                div()
                                    .text_size(LABEL_SM)
                                    .text_color(ui.muted)
                                    .truncate()
                                    .child(format!("{} · {}", role.category, role.description)),
                            ),
                    ),
                );
            }
            trigger = trigger.child(deferred_select_menu(menu));
        }
        div().relative().w_full().child(trigger).into_any_element()
    }

    fn render_agent_profile_base_select(
        &self,
        editor: &AgentProfileEditor,
        ui: crate::theme::UiColors,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let is_open = editor.agent_menu_open;
        let current = editor.agent;
        let mut trigger = select_trigger(
            SharedString::from("agent-profile-agent"),
            "Agent",
            is_open,
            ui,
        )
        .w_full()
        .max_w(px(CONTROL_WIDTH))
        .h(px(INPUT_HEIGHT))
        .py(px(0.))
        .on_press(cx.listener(move |this, _, window, cx| {
            cx.stop_propagation();
            if let Some(editor) = this.agent_profile_editor.as_mut() {
                editor.agent_menu_open = !is_open;
                editor.model_menu_open = false;
                editor.role_menu_open = false;
                editor.effort_menu_open = false;
            }
            this.settings_focus.focus(window, cx);
            cx.notify();
        }))
        .child(
            div()
                .flex()
                .flex_row()
                .items_center()
                .gap(px(8.))
                .flex_1()
                .min_w_0()
                .child(agent_icon_sized(current, 14., ui))
                .child(
                    div()
                        .min_w_0()
                        .text_size(BODY)
                        .text_color(ui.text)
                        .truncate()
                        .child(current.display_name()),
                ),
        )
        .child(select_chevron(ui));
        if is_open {
            let mut menu = select_menu(SharedString::from("agent-profile-agent-list"), ui)
                .on_mouse_down_out(cx.listener(|this, _, _w, cx| {
                    if let Some(editor) = this.agent_profile_editor.as_mut() {
                        editor.agent_menu_open = false;
                        cx.notify();
                    }
                }));
            for agent in TerminalAgent::for_this_platform() {
                let item = select_option(
                    SharedString::from(format!("agent-profile-agent-item-{}", agent.tag())),
                    agent == current,
                    ui,
                )
                .cursor(CursorStyle::Arrow)
                .on_click(cx.listener(move |this, _: &ClickEvent, _w, cx| {
                    if let Some(editor) = this.agent_profile_editor.as_mut() {
                        if editor.agent != agent {
                            editor.model_input.update(cx, |input, cx| {
                                input.set_value(SharedString::from(""), cx)
                            });
                            editor.effort = None;
                            editor.effort_menu_open = false;
                            editor.custom_model = false;
                            editor.model_menu_open = false;
                            editor.models.clear();
                            if !matches!(agent, TerminalAgent::ClaudeCode | TerminalAgent::Codex) {
                                editor.agency_role = None;
                                editor.skills.clear();
                            }
                        }
                        editor.agent = agent;
                        editor.agent_menu_open = false;
                        editor.error = None;
                    }
                    this.update_agent_profile_suggested_name(cx);
                    this.refresh_agent_profile_models(cx);
                    cx.notify();
                }))
                .child(agent_icon_sized(agent, 14., ui))
                .child(
                    div()
                        .flex_1()
                        .min_w_0()
                        .truncate()
                        .text_color(ui.text)
                        .child(agent.display_name()),
                );
                menu = menu.child(item);
            }
            trigger = trigger.child(deferred_select_menu(menu));
        }
        div().relative().w_full().child(trigger).into_any_element()
    }

    fn render_agent_permissions_section(
        &self,
        ui: crate::theme::UiColors,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let bypass = self
            .cached_config
            .claude_code_bypass_permissions
            .unwrap_or(false);
        let unrestricted = self.cached_config.ai_unrestricted_enabled();
        let fence = self.cached_config.ai_injection_fence_enabled();

        let mut card = SearchCard::new(ui)
            .row(
                &search::CLAUDE_FULL_ACCESS,
                toggle_row(
                    "row-claude-bypass",
                    search::CLAUDE_FULL_ACCESS.title,
                    search::CLAUDE_FULL_ACCESS.description,
                    None,
                    bypass,
                    "claude_code_bypass_permissions",
                    ui,
                    cx,
                ),
            )
            .row(
                &search::AI_FREE_ACCESS,
                toggle_row(
                    "row-ai-unrestricted",
                    search::AI_FREE_ACCESS.title,
                    search::AI_FREE_ACCESS.description,
                    None,
                    unrestricted,
                    "ai_unrestricted",
                    ui,
                    cx,
                ),
            );
        if unrestricted {
            card = card.row(
                &search::INJECTION_FENCE,
                toggle_row(
                    "row-ai-injection-fence",
                    search::INJECTION_FENCE.title,
                    search::INJECTION_FENCE.description,
                    None,
                    fence,
                    "ai_injection_fence",
                    ui,
                    cx,
                ),
            );
            if !fence {
                card = card.fixed(
                    div()
                        .px(px(CARD_PADDING_X))
                        .py(px(8.))
                        .text_size(BODY)
                        .text_color(destructive_color())
                        .child("Fence off: a malicious pane can silently redirect your agent."),
                );
            }
        }

        Block::new("Permissions")
            .top_gap(24.)
            .child(section_title(ui, "Permissions", None))
            .card(card)
            .finish()
    }

    fn agent_profile_command_preview(
        &self,
        editor: &AgentProfileEditor,
        cx: &mut Context<Self>,
    ) -> String {
        let model = editor.model_input.read(cx).value();
        let entry = AgentProfileConfig {
            name: "Preview".to_string(),
            agent: editor.agent.tag().to_string(),
            model: (!model.trim().is_empty()).then(|| model.trim().to_string()),
            agency_role: editor.agency_role.clone(),
            skills: editor.skills.clone(),
            default_profile: editor.default_profile.clone(),
            effort: editor.effort.clone(),
            project: editor.project.clone(),
            args: self
                .agent_profile_args_input
                .read(cx)
                .value()
                .split_whitespace()
                .map(str::to_string)
                .collect(),
            ..AgentProfileConfig::default()
        };
        match AgentProfile::for_project(&entry, &self.cached_config, editor.project.as_deref()) {
            Ok(profile) => {
                let skip = if entry.agency_role.is_some() {
                    if profile.agent == TerminalAgent::ClaudeCode {
                        4
                    } else {
                        2
                    }
                } else {
                    0
                };
                let mut display_args = Vec::new();
                let mut args = profile.args.into_iter().skip(skip).peekable();
                while let Some(arg) = args.next() {
                    if arg == "--append-system-prompt"
                        || (arg == "-c"
                            && args
                                .peek()
                                .is_some_and(|value| value.starts_with("developer_instructions=")))
                    {
                        args.next();
                    } else {
                        display_args.push(arg);
                    }
                }
                let mut preview = std::iter::once(profile.agent.binary().to_string())
                    .chain(display_args)
                    .collect::<Vec<_>>()
                    .join(" ");
                if let Some(role) = entry
                    .agency_role
                    .as_deref()
                    .and_then(paneflow_mcp_install::agency::role)
                {
                    preview.push_str(&format!(" · {}", role.name));
                }
                if !entry.skills.is_empty() {
                    preview.push_str(&format!(" · {} skills", entry.skills.len()));
                }
                preview
            }
            Err(error) => error,
        }
    }

    pub(crate) fn open_agent_profile_editor(
        &mut self,
        index: Option<usize>,
        cx: &mut Context<Self>,
    ) {
        if self.agent_profiles_busy || self.agent_setup_review.is_some() {
            self.show_toast("Profiles are still saving; try again shortly", cx);
            return;
        }
        let mut index = index;
        let mut entry = index.and_then(|idx| self.cached_config.agent_profiles.get(idx).cloned());
        if let Some(project) = &self.agent_profiles_project
            && let Some(profile) = entry.as_mut()
            && profile.project.is_none()
        {
            profile.default_profile = Some(profile.name.clone());
            profile.project = Some(project.clone());
            profile.name = suggested_profile_name(
                Some(project),
                profile.agency_role.as_deref(),
                TerminalAgent::from_tag(&profile.agent).unwrap_or(TerminalAgent::ClaudeCode),
                &self.cached_config.agent_profiles,
                None,
            );
            index = None;
        }
        let entry_default_profile = entry
            .as_ref()
            .and_then(|profile| profile.default_profile.clone());
        let entry_skills = entry
            .as_ref()
            .map_or_else(Vec::new, |profile| profile.skills.clone());
        let agent = entry
            .as_ref()
            .and_then(|entry| TerminalAgent::from_tag(entry.agent.trim()))
            .unwrap_or_else(|| {
                if self
                    .agent_profiles_project
                    .as_deref()
                    .is_some_and(|project| {
                        AgentProfile::orchestrator_model(
                            &self.cached_config,
                            project,
                            TerminalAgent::Codex,
                        )
                        .is_ok()
                    })
                {
                    TerminalAgent::Codex
                } else {
                    TerminalAgent::ClaudeCode
                }
            });
        let agency_role = entry.as_ref().and_then(|entry| entry.agency_role.clone());
        let project = entry.as_ref().map_or_else(
            || self.agent_profiles_project.clone(),
            |entry| entry.project.clone(),
        );
        let model = entry
            .as_ref()
            .and_then(|entry| entry.model.clone())
            .unwrap_or_else(|| {
                if entry.is_none()
                    && project.as_ref().is_some_and(|project| {
                        AgentProfile::orchestrator_model(&self.cached_config, project, agent)
                            .is_ok()
                    })
                {
                    ORCHESTRATOR_MODEL.to_string()
                } else {
                    String::new()
                }
            });
        let model_input = cx.new(|cx| {
            TextInput::new(model.clone(), "Model ID", cx).with_accessible_name("Agent model")
        });
        let effort = entry.as_ref().and_then(|entry| entry.effort.clone());

        let suggested_name = suggested_profile_name(
            project.as_deref(),
            agency_role.as_deref(),
            agent,
            &self.cached_config.agent_profiles,
            index,
        );
        let role_search = cx.new(|cx| {
            TextInput::new("", "Find a role or language…", cx)
                .with_accessible_name("Find an Agency role")
        });
        cx.observe(&role_search, |_, _, cx| cx.notify()).detach();
        cx.observe(&model_input, |_, _, cx| cx.notify()).detach();
        let (name, env, args) = entry
            .map(|entry| (entry.name, entry.env, entry.args.join(" ")))
            .unwrap_or_else(|| (suggested_name.clone(), BTreeMap::new(), String::new()));
        self.agent_profile_name_input.update(cx, |input, cx| {
            input.set_value(SharedString::from(name), cx)
        });
        self.agent_profile_args_input.update(cx, |input, cx| {
            input.set_value(SharedString::from(args), cx)
        });
        let mut env_rows: Vec<EnvRowInputs> = env
            .iter()
            .map(|(key, value)| new_env_row(key, value, cx))
            .collect();
        if env_rows.is_empty() {
            env_rows.push(new_env_row("", "", cx));
        }
        let skill_input = cx.new(|cx| {
            TextInput::new("", "Taste, use taste skill, or ~/.../SKILL.md", cx)
                .with_accessible_name("Skill name or Markdown path")
        });
        cx.observe(&skill_input, |_, _, cx| cx.notify()).detach();
        self.agent_profile_editor = Some(AgentProfileEditor {
            model_input,
            models: Vec::new(),
            model_menu_open: false,
            model_catalog_generation: 0,
            custom_model: !model.trim().is_empty() && model != ORCHESTRATOR_MODEL,
            effort,
            effort_menu_open: false,
            project,
            default_profile: entry_default_profile,
            skills: entry_skills,
            skill_input,
            available_skills: Vec::new(),
            skills_loading: false,
            skills_generation: 0,
            suggested_name,
            role_search,
            role_menu_open: false,
            agency_role,
            index,
            agent,
            agent_menu_open: false,
            error: None,
            env_rows,
        });
        self.refresh_agent_profile_models(cx);
        self.refresh_agent_profile_skills(cx);
        cx.notify();
    }

    pub(crate) fn close_agent_profile_editor(&mut self, cx: &mut Context<Self>) {
        self.agent_profile_editor = None;
        cx.notify();
    }

    fn add_agent_profile_env_row(&mut self, cx: &mut Context<Self>) {
        let row = new_env_row("", "", cx);
        if let Some(editor) = self.agent_profile_editor.as_mut() {
            editor.env_rows.push(row);
        }
        cx.notify();
    }

    fn save_agent_profile(&mut self, cx: &mut Context<Self>) {
        if self.agent_profiles_busy || self.agent_setup_review.is_some() {
            self.show_toast("Profiles are still saving; try again shortly", cx);
            return;
        }
        let Some(editor) = self.agent_profile_editor.as_ref() else {
            return;
        };
        let index = editor.index;
        let agent = editor.agent;
        let name = self.agent_profile_name_input.read(cx).value();
        let args_text = self.agent_profile_args_input.read(cx).value();
        let model = editor.model_input.read(cx).value();
        let agency_role = editor.agency_role.clone();
        let effort = editor.effort.clone();
        let project = editor.project.clone();
        let default_profile = editor.default_profile.clone();
        let skills = editor.skills.clone();
        let raw_env: Vec<(String, String)> = editor
            .env_rows
            .iter()
            .map(|row| (row.key.read(cx).value(), row.value.read(cx).value()))
            .collect();

        let home = dirs::home_dir();
        let draft = collect_env(&raw_env).and_then(|env| {
            validate_env_values(&env, home.as_deref())?;
            let entry = AgentProfileConfig {
                model: (!model.trim().is_empty()).then(|| model.trim().to_string()),
                agency_role,
                effort,
                project,
                default_profile,
                skills,
                name: name.trim().to_string(),
                agent: agent.tag().to_string(),
                env,
                args: args_text.split_whitespace().map(str::to_string).collect(),
            };
            AgentProfile::for_project(&entry, &self.cached_config, entry.project.as_deref())
                .map(|_| entry)
        });
        let entry = match draft {
            Ok(entry) => entry,
            Err(message) => {
                if let Some(editor) = self.agent_profile_editor.as_mut() {
                    editor.error = Some(message);
                }
                cx.notify();
                return;
            }
        };

        let mut profiles = self.cached_config.agent_profiles.clone();
        if profiles.iter().enumerate().any(|(idx, existing)| {
            Some(idx) != index
                && existing.name.eq_ignore_ascii_case(&entry.name)
                && entry.default_profile.as_deref() != Some(existing.name.as_str())
                && (existing.applies_to_project(entry.project.as_deref())
                    || entry.applies_to_project(existing.project.as_deref()))
        }) {
            if let Some(editor) = self.agent_profile_editor.as_mut() {
                editor.error = Some("Choose a unique profile name for this project".to_string());
            }
            cx.notify();
            return;
        }
        if let Some(idx) = index
            && let Some(previous) = profiles.get(idx)
            && previous.project.is_none()
            && previous.name != entry.name
        {
            let old_name = previous.name.clone();
            for profile in &mut profiles {
                if profile
                    .default_profile
                    .as_deref()
                    .is_some_and(|name| name.eq_ignore_ascii_case(&old_name))
                {
                    profile.default_profile = Some(entry.name.clone());
                }
            }
        }
        match index {
            Some(idx) if idx < profiles.len() => profiles[idx] = entry,
            _ => profiles.push(entry),
        }
        let pending_editor = self.agent_profile_editor.take();
        self.persist_agent_profiles(profiles, pending_editor, cx);
    }

    fn add_agency_profiles(&mut self, cx: &mut Context<Self>) {
        if self.integration_busy.is_some()
            || self.agent_profiles_busy
            || self.agent_setup_review.is_some()
        {
            return;
        }
        let mut profiles = self.cached_config.agent_profiles.clone();
        let project = self.agent_profiles_project.clone();
        for mut profile in AgentProfile::agency_defaults() {
            if !profiles.iter().any(|existing| {
                existing.agency_role == profile.agency_role && existing.project == project
            }) {
                profile.project = project.clone();
                profile.name = suggested_profile_name(
                    project.as_deref(),
                    profile.agency_role.as_deref(),
                    TerminalAgent::ClaudeCode,
                    &profiles,
                    None,
                );
                profiles.push(profile);
            }
        }
        self.persist_agent_profiles(profiles, None, cx);
        if self.agent_profiles_project.is_none() {
            self.change_integration("claude-code".to_string(), true, cx);
        }
    }

    fn delete_agent_profile(&mut self, idx: usize, cx: &mut Context<Self>) {
        if self.agent_profiles_busy || self.agent_setup_review.is_some() {
            self.show_toast("Profiles are still saving; try again shortly", cx);
            return;
        }
        if idx >= self.cached_config.agent_profiles.len() {
            return;
        }
        let mut profiles = self.cached_config.agent_profiles.clone();
        if self.agent_profiles_project.is_some() && profiles[idx].project.is_none() {
            return;
        }
        let removed = profiles.remove(idx);
        if removed.project.is_none() {
            for profile in &mut profiles {
                if profile
                    .default_profile
                    .as_deref()
                    .is_some_and(|name| name.eq_ignore_ascii_case(&removed.name))
                {
                    profile.default_profile = None;
                }
            }
        }
        if self
            .agent_profile_editor
            .as_ref()
            .is_some_and(|editor| editor.index == Some(idx))
        {
            self.agent_profile_editor = None;
        }
        self.persist_agent_profiles(profiles, None, cx);
    }

    fn persist_agent_profiles(
        &mut self,
        profiles: Vec<AgentProfileConfig>,
        pending_editor: Option<AgentProfileEditor>,
        cx: &mut Context<Self>,
    ) {
        let scope = self.agent_profiles_project.clone();
        let skills_to_validate = pending_editor.as_ref().map(|editor| editor.skills.clone());
        self.agent_profiles_busy = true;
        cx.spawn(async move |this, cx| {
            let (profiles, prepared) = smol::unblock(move || {
                let result = skills_to_validate
                    .as_deref()
                    .map_or(Ok(()), crate::agent_profile_context::validate_skill_paths)
                    .and_then(|()| crate::agent_launcher::prepare_profile_roles(&profiles));
                (profiles, result)
            })
            .await;
            let _ = this.update(cx, |this, cx| {
                this.agent_profiles_busy = false;

                if let Err(error) = prepared {
                    if this.settings_section == Some(crate::SettingsSection::Agents)
                        && this.agent_profiles_project == scope
                    {
                        this.agent_profile_editor = pending_editor;
                    }
                    this.show_toast(format!("Could not save agent profiles: {error}"), cx);
                    return;
                }
                let value = if profiles.is_empty() {
                    serde_json::Value::Null
                } else {
                    match serde_json::to_value(profiles) {
                        Ok(value) => value,
                        Err(error) => {
                            if this.settings_section == Some(crate::SettingsSection::Agents)
                                && this.agent_profiles_project == scope
                            {
                                this.agent_profile_editor = pending_editor;
                            }
                            this.show_toast(
                                format!("Could not serialize agent profiles: {error}"),
                                cx,
                            );
                            return;
                        }
                    }
                };
                this.persist_setting(false, "agent_profiles", value, cx);
            });
        })
        .detach();
    }
}

pub(crate) fn new_env_row(key: &str, value: &str, cx: &mut Context<PaneFlowApp>) -> EnvRowInputs {
    let key = cx
        .new(|cx| TextInput::new(key.to_string(), "KEY", cx).with_accessible_name("Variable name"));
    cx.observe(&key, |_, _, cx| cx.notify()).detach();
    let value = cx.new(|cx| {
        TextInput::new(value.to_string(), "value", cx).with_accessible_name("Variable value")
    });
    cx.observe(&value, |_, _, cx| cx.notify()).detach();
    EnvRowInputs { key, value }
}

pub(crate) fn validate_env_values(
    env: &BTreeMap<String, String>,
    home: Option<&std::path::Path>,
) -> Result<(), String> {
    for (key, value) in env {
        if let Err(name) = crate::env_expand::expand_with_process_env(value, home) {
            return Err(format!(
                "'{key}' references {name}, which is not set in your environment"
            ));
        }
    }
    Ok(())
}

pub(crate) fn collect_env(rows: &[(String, String)]) -> Result<BTreeMap<String, String>, String> {
    let mut env = BTreeMap::new();
    for (key, value) in rows {
        let key = key.trim();
        let value = value.trim();
        if key.is_empty() && value.is_empty() {
            continue;
        }
        if key.is_empty() {
            return Err(format!("the value '{value}' needs a variable name"));
        }
        if env.insert(key.to_string(), value.to_string()).is_some() {
            return Err(format!("'{key}' is listed twice"));
        }
    }
    Ok(env)
}

fn editor_row(
    title: &'static str,
    description: &'static str,
    control: impl IntoElement,
    ui: crate::theme::UiColors,
) -> impl IntoElement {
    div()
        .flex()
        .flex_row()
        .items_start()
        .gap(px(16.))
        .px(px(CARD_PADDING_X))
        .py(px(10.))
        .child(setting_text(ui, title, description))
        .child(div().flex_shrink_0().child(control))
}

fn editor_stacked_row(
    title: &'static str,
    description: &'static str,
    control: impl IntoElement,
    ui: crate::theme::UiColors,
) -> impl IntoElement {
    div()
        .flex()
        .flex_col()
        .gap(px(10.))
        .px(px(CARD_PADDING_X))
        .py(px(10.))
        .child(
            div()
                .flex()
                .flex_row()
                .child(setting_text(ui, title, description)),
        )
        .child(control)
}

fn env_value_hint(
    value: &str,
    home: Option<&std::path::Path>,
    ui: crate::theme::UiColors,
) -> Option<AnyElement> {
    let value = value.trim();
    if value.is_empty() {
        return None;
    }
    let base = div()
        .pl(px(ENV_NAME_WIDTH + 6.))
        .text_size(LABEL_XS)
        .truncate();
    match crate::env_expand::expand_with_process_env(value, home) {
        Ok(expanded) if expanded == value => None,
        Ok(expanded) => Some(
            base.font_family(mono_family())
                .text_color(ui.muted)
                .child(expanded)
                .into_any_element(),
        ),
        Err(name) => Some(
            base.text_color(destructive_color())
                .child(format!("{name} is not set in your environment"))
                .into_any_element(),
        ),
    }
}

fn missing_agent_row(agent: TerminalAgent, ui: crate::theme::UiColors) -> AnyElement {
    div()
        .flex()
        .flex_row()
        .items_center()
        .flex_none()
        .h(px(AGENT_ROW_HEIGHT))
        .gap(px(12.))
        .px(px(CARD_PADDING_X))
        .opacity(0.45)
        .child(agent_icon_el(agent, ui))
        .child(
            div()
                .flex_1()
                .min_w_0()
                .text_size(BODY_EMPHASIS)
                .font_weight(gpui::FontWeight::MEDIUM)
                .text_color(ui.text)
                .child(agent.display_name()),
        )
        .child(mono_text(agent.binary().into(), ui.muted))
        .into_any_element()
}

fn detail_row(
    title: &'static str,
    description: AnyElement,
    control: Option<AnyElement>,
    ui: crate::theme::UiColors,
) -> AnyElement {
    div()
        .flex()
        .flex_row()
        .items_center()
        .gap(px(16.))
        .py(px(10.))
        .child(
            div()
                .flex_1()
                .min_w_0()
                .flex()
                .flex_col()
                .gap(px(2.))
                .child(
                    div()
                        .text_size(BODY)
                        .font_weight(gpui::FontWeight::MEDIUM)
                        .text_color(ui.text)
                        .child(title),
                )
                .child(description),
        )
        .children(control)
        .into_any_element()
}

fn detail_text(text: String, color: Hsla) -> AnyElement {
    div()
        .text_size(LABEL_SM)
        .text_color(color)
        .child(text)
        .into_any_element()
}

fn mono_family() -> SharedString {
    crate::terminal::element::resolve_font_family(None).into()
}

fn mono_text(text: SharedString, color: Hsla) -> impl IntoElement {
    div()
        .font_family(mono_family())
        .text_size(LABEL_SM)
        .text_color(color)
        .child(text)
}

pub(crate) fn input_box(
    input: Entity<TextInput>,
    mono: bool,
    ui: crate::theme::UiColors,
) -> impl IntoElement {
    crate::settings::components::input_frame(
        div()
            .w_full()
            .h(px(INPUT_HEIGHT))
            .flex()
            .items_center()
            .px(px(10.)),
        &input,
        false,
        ui,
    )
    .when(mono, |d| d.font_family(mono_family()))
    .text_size(if mono { LABEL_SM } else { BODY })
    .text_color(ui.text)
    .child(input)
}

fn chip(text: String, ui: crate::theme::UiColors) -> impl IntoElement {
    div()
        .flex_none()
        .px(px(6.))
        .py(px(1.))
        .rounded(px(6.))
        .bg(with_alpha(ui.text, 0.06))
        .font_family(mono_family())
        .text_size(LABEL_XS)
        .text_color(ui.text)
        .child(text)
}

pub(crate) fn icon_button(
    id: SharedString,
    icon: &'static str,
    resting: Hsla,
    hovered: Hsla,
    ui: crate::theme::UiColors,
) -> crate::ui_primitives::AnimatedHover {
    let group = SharedString::from(format!("{id}-squircle"));
    ghost_skin(
        div()
            .id(id)
            .flex_none()
            .w(px(ICON_BUTTON_SIZE))
            .h(px(ICON_BUTTON_SIZE))
            .flex()
            .items_center()
            .justify_center(),
        group.clone(),
        px(7.),
        ui,
    )
    .child(
        svg()
            .size(px(13.))
            .flex_none()
            .path(icon)
            .text_color(resting)
            .group_hover(group, |style| style.text_color(hovered)),
    )
}

fn agent_icon_el(agent: TerminalAgent, ui: crate::theme::UiColors) -> AnyElement {
    agent_icon_sized(agent, ROW_ICON, ui)
}

fn agent_icon_sized(agent: TerminalAgent, size: f32, ui: crate::theme::UiColors) -> AnyElement {
    crate::settings::components::render_logo(
        agent.icon_path(),
        agent.icon_multicolor(),
        px(size),
        agent.accent().map(|c| rgb(c).into()).unwrap_or(ui.text),
    )
}

#[cfg(test)]
mod tests {
    #[test]
    fn project_role_names_are_suggested_and_numbered_without_cross_project_collisions() {
        use super::*;
        let project = std::env::temp_dir().join("felco-app");
        let name = suggested_profile_name(
            Some(&project),
            Some("ui-designer"),
            TerminalAgent::Codex,
            &[],
            None,
        );
        assert_eq!(name, "felco-app UI Designer");
        let profiles = vec![AgentProfileConfig {
            name: name.clone(),
            project: Some(project.clone()),
            ..Default::default()
        }];
        assert_eq!(
            suggested_profile_name(
                Some(&project),
                Some("ui-designer"),
                TerminalAgent::Codex,
                &profiles,
                None
            ),
            "felco-app UI Designer 2"
        );
        assert_eq!(
            suggested_profile_name(
                Some(&project),
                Some("ui-designer"),
                TerminalAgent::Codex,
                &profiles,
                Some(0)
            ),
            name
        );
        assert_eq!(
            suggested_profile_name(None, Some("ui-designer"), TerminalAgent::Codex, &[], None),
            "Agency UI Designer"
        );
    }

    #[test]
    fn agency_role_search_finds_language_and_division_expertise() {
        assert!(
            super::matching_agency_roles("RuSt")
                .iter()
                .any(|role| role.id == "engineering-rust-refactoring-specialist")
        );
        assert!(
            super::matching_agency_roles("Design")
                .iter()
                .any(|role| role.id == "ui-designer")
        );
        assert!(!super::matching_agency_roles("Laravel").is_empty());
        assert!(super::matching_agency_roles("no-such-expertise-42").is_empty());
    }

    use super::*;

    #[test]
    fn the_settings_agent_lists_hold_exactly_this_platforms_runtimes() {
        let listed: std::collections::HashSet<TerminalAgent> =
            installed_agents().chain(missing_agents()).collect();
        let expected: std::collections::HashSet<TerminalAgent> =
            TerminalAgent::for_this_platform().collect();
        assert_eq!(listed, expected);
        assert_eq!(listed.contains(&TerminalAgent::Fx), !cfg!(windows));
    }

    #[test]
    fn collect_env_skips_blank_rows_and_rejects_nameless_or_duplicate_keys() {
        let rows = vec![
            (
                "CLAUDE_CONFIG_DIR".to_string(),
                "~/.claude-perso".to_string(),
            ),
            (String::new(), String::new()),
            (" A ".to_string(), " 1 ".to_string()),
        ];
        let env = collect_env(&rows).unwrap();
        assert_eq!(env.len(), 2);
        assert_eq!(env["A"], "1");
        assert!(collect_env(&[(String::new(), "value".to_string())]).is_err());
        assert!(
            collect_env(&[
                ("A".to_string(), "1".to_string()),
                ("A".to_string(), "2".to_string())
            ])
            .is_err()
        );
    }

    #[test]
    fn validate_env_values_rejects_a_name_the_environment_does_not_define() {
        let known = BTreeMap::from([("A".to_string(), "$PATH".to_string())]);
        assert!(validate_env_values(&known, None).is_ok());
        let literal = BTreeMap::from([("A".to_string(), "~/.claude-perso".to_string())]);
        assert!(validate_env_values(&literal, None).is_ok());
        let unknown = BTreeMap::from([(
            "CLAUDE_CONFIG_DIR".to_string(),
            "$PANEFLOW_NOT_A_REAL_VARIABLE/x".to_string(),
        )]);
        let error = validate_env_values(&unknown, None).unwrap_err();
        assert!(error.contains("CLAUDE_CONFIG_DIR"), "{error}");
        assert!(error.contains("PANEFLOW_NOT_A_REAL_VARIABLE"), "{error}");
    }
}
