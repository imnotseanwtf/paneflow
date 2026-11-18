use std::collections::{BTreeMap, HashMap, HashSet};
use std::sync::{Mutex, OnceLock};
use std::time::{Duration, Instant};

use paneflow_agent_config::{
    RUNTIMES, Runtime, RuntimePlatform, RuntimeSuggestedPreset, current_platform,
    launcher_runtimes, runtime_by_command_alias, runtime_by_id, runtime_by_preset_id,
};
use paneflow_config::schema::{AgentProfileConfig, PaneFlowConfig};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct TerminalAgent(&'static str);

paneflow_agent_config::runtime_identity_constants!(TerminalAgent);

impl TerminalAgent {
    pub fn all() -> impl Iterator<Item = TerminalAgent> {
        RUNTIMES.iter().map(|runtime| TerminalAgent(runtime.id))
    }

    pub fn on_platform(platform: RuntimePlatform) -> impl Iterator<Item = TerminalAgent> {
        RUNTIMES
            .iter()
            .filter(move |runtime| runtime.supports(platform))
            .map(|runtime| TerminalAgent(runtime.id))
    }

    pub fn for_this_platform() -> impl Iterator<Item = TerminalAgent> {
        Self::on_platform(current_platform())
    }

    #[allow(
        clippy::expect_used,
        reason = "TerminalAgent values are private catalog identities"
    )]
    pub fn runtime(self) -> &'static Runtime {
        runtime_by_id(self.0).expect("catalog identity must resolve")
    }

    pub fn visibility_config_key(self) -> &'static str {
        self.runtime().display.visibility_config_key
    }

    pub fn cached_version(self) -> Option<String> {
        version_cache()
            .lock()
            .ok()
            .and_then(|cache| cache.get(&self).cloned().flatten())
    }

    pub fn probe_missing_versions() {
        let pending: Vec<TerminalAgent> = {
            let Ok(cache) = version_cache().lock() else {
                return;
            };
            TerminalAgent::all()
                .filter(|agent| agent.is_installed() && !cache.contains_key(agent))
                .collect()
        };
        for agent in pending {
            agent.refresh_version();
        }
    }

    pub fn refresh_version(self) -> Option<String> {
        let version = self.real_executable().and_then(|path| probe_version(&path));
        if let Ok(mut cache) = version_cache().lock() {
            cache.insert(self, version.clone());
        }
        version
    }

    pub fn real_executable(self) -> Option<std::path::PathBuf> {
        which::which_all(self.binary())
            .ok()?
            .find(|candidate| !is_paneflow_shim(candidate))
    }

    pub fn display_rank(self) -> usize {
        usize::from(self.runtime().display.order)
    }

    pub fn display_name(self) -> &'static str {
        self.runtime().label
    }

    pub fn icon_path(self) -> &'static str {
        self.runtime().display.icon_asset_path
    }

    pub fn accent(self) -> Option<u32> {
        self.runtime().display.tint
    }

    pub fn icon_multicolor(self) -> bool {
        self.runtime().display.icon_multicolor
    }

    pub fn tag(self) -> &'static str {
        self.runtime().suggested_presets[0].id
    }

    pub fn from_binary(name: &str) -> Option<TerminalAgent> {
        runtime_by_command_alias(name).map(|runtime| TerminalAgent(runtime.id))
    }

    pub fn from_launch_command(command: &str) -> Option<TerminalAgent> {
        command.split(['&', '|', ';', '\n']).find_map(|segment| {
            let token = segment
                .split_whitespace()
                .find(|token| !is_env_assignment(token))?;
            TerminalAgent::from_binary(executable_stem(token))
        })
    }

    pub fn from_runtime_id(id: &str) -> Option<TerminalAgent> {
        runtime_by_id(id).map(|runtime| TerminalAgent(runtime.id))
    }

    pub fn from_tag(tag: &str) -> Option<TerminalAgent> {
        runtime_by_preset_id(tag).map(|runtime| TerminalAgent(runtime.id))
    }

    pub fn is_visible(self, config: &PaneFlowConfig) -> bool {
        config
            .agent_button_visible(self.visibility_config_key())
            .unwrap_or_else(|| self.is_installed())
    }

    pub fn binary(self) -> &'static str {
        self.runtime().detection.command_aliases[0]
    }

    pub fn is_installed(self) -> bool {
        installed_binaries_contains(self.binary())
    }

    pub fn is_installed_now(self) -> bool {
        if installed_binary_scan_pending() {
            refresh_installed_binaries();
        }
        installed_binaries_contains(self.binary())
    }

    fn launch_spec(self, config: &PaneFlowConfig) -> AgentCommandSpec {
        self.preset_spec(self.runtime().suggested_presets[0].command, config)
    }

    fn preset_spec(self, command: &'static str, config: &PaneFlowConfig) -> AgentCommandSpec {
        let mut tokens = command.split_whitespace();
        let mut spec = AgentCommandSpec::new(tokens.next().unwrap_or(self.binary()));
        spec.extend_args(tokens);
        self.push_launch_flags(&mut spec, config);
        spec
    }

    pub(crate) fn push_launch_flags(self, spec: &mut AgentCommandSpec, config: &PaneFlowConfig) {
        if self == TerminalAgent::ClaudeCode
            && config.claude_code_bypass_permissions.unwrap_or(false)
        {
            spec.push_arg("--permission-mode");
            spec.push_arg("bypassPermissions");
        } else if self == TerminalAgent::Codex && config.codex_bypass_permissions.unwrap_or(false) {
            spec.push_arg("--dangerously-bypass-approvals-and-sandbox");
        }
    }

    fn command(self, config: &PaneFlowConfig) -> String {
        self.launch_spec(config).render_shell_command()
    }

    pub fn session_agent(self) -> Option<crate::agent_sessions::SessionAgent> {
        crate::agent_sessions::SessionAgent::of(self)
    }

    pub fn launch_command(self, config: &PaneFlowConfig) -> String {
        wrap_for_shell(&self.command(config), config)
    }

    pub fn visible(config: &PaneFlowConfig) -> Vec<TerminalAgent> {
        launcher_runtimes(
            |key| config.agent_button_visible(key),
            |runtime| installed_binaries_contains(runtime.detection.command_aliases[0]),
        )
        .into_iter()
        .map(|runtime| TerminalAgent(runtime.id))
        .collect()
    }

    pub fn supports_current_platform(self) -> bool {
        self.runtime().supports_current_platform()
    }

    pub fn effort_levels(self) -> &'static [&'static str] {
        match self {
            Self::ClaudeCode => &["low", "medium", "high", "xhigh", "max"],
            Self::Codex => &[
                "none", "minimal", "low", "medium", "high", "xhigh", "max", "ultra",
            ],
            _ => &[],
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct AgentCommandSpec {
    program: &'static str,
    args: Vec<String>,
}

impl AgentCommandSpec {
    pub(crate) fn new(program: &'static str) -> Self {
        Self {
            program,
            args: Vec::new(),
        }
    }

    pub(crate) fn push_arg(&mut self, arg: impl Into<String>) {
        self.args.push(arg.into());
    }

    fn extend_args(&mut self, args: impl IntoIterator<Item = &'static str>) {
        self.args.extend(args.into_iter().map(str::to_string));
    }

    pub(crate) fn render_shell_command(&self) -> String {
        self.render_shell_command_for_shell(None)
    }

    fn render_shell_command_for_shell(&self, shell: Option<&str>) -> String {
        let quoting = shell
            .map(crate::terminal::types::ShellQuoting::for_shell)
            .unwrap_or_else(crate::terminal::types::ShellQuoting::default_for_platform);
        debug_assert!(is_plain_shell_token(self.program));
        let mut command = self.program.to_string();
        for arg in &self.args {
            command.push(' ');
            if is_plain_shell_token(arg) {
                command.push_str(arg);
            } else {
                command.push_str(&crate::terminal::quote_path_for_shell(arg, quoting));
            }
        }
        command
    }
}

pub(crate) fn is_plain_shell_token(token: &str) -> bool {
    !token.is_empty()
        && token
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'-' | b'_' | b'.' | b'='))
}

struct InstalledBinaryCache {
    checked_at: Option<Instant>,
    refreshing: bool,
    found: HashSet<&'static str>,
}

impl InstalledBinaryCache {
    fn is_stale(&self) -> bool {
        self.checked_at
            .is_none_or(|checked_at| checked_at.elapsed() >= INSTALLED_BINARIES_TTL)
    }
}

const INSTALLED_BINARIES_TTL: Duration = Duration::from_secs(2);

fn installed_binary_cache() -> &'static Mutex<InstalledBinaryCache> {
    static CACHE: OnceLock<Mutex<InstalledBinaryCache>> = OnceLock::new();
    CACHE.get_or_init(|| {
        Mutex::new(InstalledBinaryCache {
            checked_at: None,
            refreshing: false,
            found: HashSet::new(),
        })
    })
}

fn lock_installed_binary_cache() -> std::sync::MutexGuard<'static, InstalledBinaryCache> {
    match installed_binary_cache().lock() {
        Ok(cache) => cache,
        Err(poisoned) => {
            tracing::warn!(
                target: "paneflow_app::agent_launcher",
                "installed binary cache mutex poisoned; using recovered state"
            );
            poisoned.into_inner()
        }
    }
}

fn scan_installed_binaries() -> HashSet<&'static str> {
    TerminalAgent::all()
        .map(TerminalAgent::binary)
        .filter(|bin| which::which(bin).is_ok())
        .collect()
}

pub(crate) fn refresh_installed_binaries() {
    let found = scan_installed_binaries();
    let mut cache = lock_installed_binary_cache();
    cache.found = found;
    cache.checked_at = Some(Instant::now());
    cache.refreshing = false;
}

pub(crate) fn installed_binary_scan_pending() -> bool {
    let cache = lock_installed_binary_cache();
    cache.checked_at.is_none() || cache.refreshing
}

fn installed_binaries_contains(binary: &'static str) -> bool {
    let mut cache = lock_installed_binary_cache();
    if cache.is_stale() && !cache.refreshing {
        cache.refreshing = true;
        let spawned = std::thread::Builder::new()
            .name("paneflow-agent-scan".into())
            .spawn(refresh_installed_binaries);
        if let Err(error) = spawned {
            cache.refreshing = false;
            tracing::warn!(
                target: "paneflow_app::agent_launcher",
                "installed binary scan thread failed to start: {error}"
            );
        }
    }
    cache.found.contains(binary)
}

fn version_cache() -> &'static Mutex<HashMap<TerminalAgent, Option<String>>> {
    static CACHE: OnceLock<Mutex<HashMap<TerminalAgent, Option<String>>>> = OnceLock::new();
    CACHE.get_or_init(|| Mutex::new(HashMap::new()))
}

const VERSION_PROBE_TIMEOUT: Duration = Duration::from_secs(3);
const VERSION_PROBE_STDOUT_CAP: u64 = 4096;

fn is_paneflow_shim(candidate: &std::path::Path) -> bool {
    candidate.parent().is_some_and(|dir| {
        ["paneflow-ai-hook", "paneflow-ai-hook.exe"]
            .iter()
            .any(|hook| dir.join(hook).is_file())
    })
}

fn probe_version(binary: &std::path::Path) -> Option<String> {
    let mut command = std::process::Command::new(binary);
    command.arg("--version");
    let output = paneflow_process::run_with_timeout(
        command,
        VERSION_PROBE_TIMEOUT,
        VERSION_PROBE_STDOUT_CAP,
    )
    .ok()?;
    parse_version(std::str::from_utf8(&output.stdout).ok()?)
}

fn parse_version(output: &str) -> Option<String> {
    output
        .lines()
        .take(3)
        .flat_map(str::split_whitespace)
        .map(|token| {
            token
                .trim_start_matches(['v', 'V'])
                .trim_end_matches([',', ')', ';'])
        })
        .find(|token| {
            token.starts_with(|c: char| c.is_ascii_digit())
                && token.contains('.')
                && token.len() <= 32
                && token
                    .chars()
                    .all(|c| c.is_ascii_alphanumeric() || matches!(c, '.' | '-' | '+'))
        })
        .map(str::to_string)
}

fn is_env_assignment(token: &str) -> bool {
    match token.split_once('=') {
        Some((key, _)) => {
            !key.is_empty()
                && key.chars().all(|c| c.is_ascii_alphanumeric() || c == '_')
                && !key.starts_with(|c: char| c.is_ascii_digit())
        }
        None => false,
    }
}

pub(crate) fn executable_stem(token: &str) -> &str {
    let base = token.rsplit(['/', '\\']).next().unwrap_or(token);
    for suffix in [".exe", ".cmd", ".bat", ".ps1"] {
        if base
            .get(base.len().saturating_sub(suffix.len())..)
            .is_some_and(|s| s.eq_ignore_ascii_case(suffix))
        {
            return &base[..base.len() - suffix.len()];
        }
    }
    base
}

fn wrap_for_shell(command: &str, config: &PaneFlowConfig) -> String {
    let shell = config
        .default_shell
        .as_deref()
        .map(str::trim)
        .filter(|s| !s.is_empty());
    crate::terminal::shell::clear_then(command, shell)
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AgentProfile {
    pub name: String,
    pub agent: TerminalAgent,
    pub env: BTreeMap<String, String>,
    pub args: Vec<String>,
    pub agency_role: Option<String>,
    pub skills: Vec<std::path::PathBuf>,
}

pub(crate) const ORCHESTRATOR_MODEL: &str = "@orchestrator";

impl AgentProfile {
    pub(crate) fn orchestrator_model(
        config: &PaneFlowConfig,
        project: &std::path::Path,
        agent: TerminalAgent,
    ) -> Result<String, String> {
        let mut profiles: Vec<_> = config
            .agency_profiles_for_project("orchestrator", Some(project))
            .collect();
        if profiles.iter().any(|profile| profile.project.is_some()) {
            profiles.retain(|profile| profile.project.is_some());
        }
        let [lead] = profiles.as_slice() else {
            return Err(
                "Set up exactly one Orchestrator profile for this project before using its model"
                    .to_string(),
            );
        };
        if lead.model.as_deref() == Some(ORCHESTRATOR_MODEL) {
            return Err("The Orchestrator must select its own model".to_string());
        }
        let lead_profile = Self::from_config(lead)?;
        if lead_profile.agent != agent {
            return Err(format!(
                "The Orchestrator uses {}; choose the same base agent or select a model explicitly",
                lead_profile.agent.display_name()
            ));
        }
        lead.model
            .clone()
            .ok_or_else(|| "Select an explicit model in the Orchestrator profile first".to_string())
    }

    pub(crate) fn for_project(
        entry: &AgentProfileConfig,
        config: &PaneFlowConfig,
        project: Option<&std::path::Path>,
    ) -> Result<Self, String> {
        if entry.model.as_deref() != Some(ORCHESTRATOR_MODEL) {
            return Self::from_config(entry);
        }
        if entry.agency_role.as_deref() == Some("orchestrator") {
            return Err("The Orchestrator must select its own model".to_string());
        }
        let project = entry.project.as_deref().or(project).ok_or_else(|| {
            "Orchestrator model defaults are available in project settings".to_string()
        })?;
        let agent = TerminalAgent::from_tag(entry.agent.trim())
            .ok_or_else(|| format!("unknown base agent '{}'", entry.agent))?;
        let mut resolved = entry.clone();
        resolved.model = Some(Self::orchestrator_model(config, project, agent)?);
        Self::from_config(&resolved)
    }

    pub fn from_config(entry: &AgentProfileConfig) -> Result<AgentProfile, String> {
        if entry.default_profile.is_some() && entry.project.is_none() {
            return Err("Only project profiles can customize a default profile".to_string());
        }
        if entry.project.as_ref().is_some_and(|project| {
            !project.is_absolute()
                || project.to_str().is_none()
                || project
                    .components()
                    .any(|part| part == std::path::Component::ParentDir)
        }) {
            return Err("project must be an absolute directory without '..'".to_string());
        }
        let name = entry.name.trim();
        if name.is_empty() {
            return Err("profile name is empty".to_string());
        }
        let Some(agent) = TerminalAgent::from_tag(entry.agent.trim()) else {
            return Err(format!("unknown base agent '{}'", entry.agent));
        };
        if let Some(key) = entry.env.keys().find(|key| !is_env_key(key)) {
            return Err(format!("invalid environment variable name '{key}'"));
        }
        if let Some(arg) = entry.args.iter().find(|arg| !is_plain_shell_token(arg)) {
            return Err(format!(
                "argument '{arg}' may only contain letters, digits, '-', '_', '.', and '='"
            ));
        }
        let mut args = Vec::new();
        if !entry.skills.is_empty() {
            if !matches!(agent, TerminalAgent::ClaudeCode | TerminalAgent::Codex) {
                return Err("Attached skills support Claude Code and Codex".to_string());
            }
            crate::agent_profile_context::validate_skill_selection(&entry.skills)?;
        }
        if let Some(role_id) = entry.agency_role.as_deref() {
            if !matches!(agent, TerminalAgent::ClaudeCode | TerminalAgent::Codex) {
                return Err("Agency roles support Claude Code and Codex".to_string());
            }
            let role = paneflow_mcp_install::agency::role(role_id)
                .ok_or_else(|| format!("unknown Agency role '{role_id}'"))?;
            let home = paneflow_home::paneflow_home()
                .ok_or_else(|| "cannot resolve the Paneflow home".to_string())?;
            if agent == TerminalAgent::ClaudeCode {
                args.extend([
                    "--agent".to_string(),
                    format!("paneflow-agency:{}", role.agent_name()),
                    "--plugin-dir".to_string(),
                    paneflow_mcp_install::agency::launch_file(&home, role_id, "plugin")
                        .to_string_lossy()
                        .into_owned(),
                ]);
            } else {
                args.extend([
                    "-c".to_string(),
                    format!(
                        "model_instructions_file={}",
                        paneflow_mcp_install::agency::launch_file(&home, role_id, "md")
                            .to_string_lossy()
                    ),
                ]);
            }
        }
        if let Some(model) = entry.model.as_deref() {
            if !matches!(agent, TerminalAgent::ClaudeCode | TerminalAgent::Codex)
                || model.starts_with('-')
                || !is_plain_shell_token(model)
            {
                return Err("model must be a plain Claude Code or Codex model name".to_string());
            }
            args.extend(["--model".to_string(), model.to_string()]);
        }
        if let Some(effort) = entry.effort.as_deref() {
            let valid = agent.effort_levels().contains(&effort);
            if !valid {
                return Err("effort is not supported by the selected base agent".to_string());
            }
            if agent == TerminalAgent::ClaudeCode {
                args.extend(["--effort".to_string(), effort.to_string()]);
            } else {
                args.extend(["-c".to_string(), format!("model_reasoning_effort={effort}")]);
            }
        }
        if !entry.skills.is_empty() {
            let guidance = crate::agent_profile_context::skill_guidance(&entry.skills);
            if agent == TerminalAgent::ClaudeCode {
                args.extend(["--append-system-prompt".to_string(), guidance]);
            } else {
                args.extend([
                    "-c".to_string(),
                    format!(
                        "developer_instructions={}",
                        serde_json::to_string(&guidance).map_err(|error| error.to_string())?
                    ),
                ]);
            }
        }
        if entry.args.iter().any(|arg| {
            (entry.model.is_some()
                && (matches!(arg.as_str(), "--model" | "-m") || arg.starts_with("--model=")))
                || (entry.agency_role.is_some()
                    && (matches!(arg.as_str(), "--agent" | "--agents")
                        || arg.starts_with("--agent=")
                        || arg.starts_with("--agents=")
                        || arg.starts_with("model_instructions_file=")))
                || (entry.effort.is_some()
                    && (arg == "--effort"
                        || arg.starts_with("--effort=")
                        || arg.starts_with("model_reasoning_effort=")))
                || (!entry.skills.is_empty()
                    && (arg == "--append-system-prompt"
                        || arg.starts_with("--append-system-prompt=")
                        || arg.starts_with("developer_instructions=")))
        }) {
            return Err("set the model, effort and Agency role in their profile fields, without duplicate extra arguments".to_string());
        }
        args.extend(entry.args.clone());
        Ok(AgentProfile {
            name: name.to_string(),
            agent,
            env: entry.env.clone(),
            args,
            agency_role: entry.agency_role.clone(),
            skills: entry.skills.clone(),
        })
    }

    pub fn all(config: &PaneFlowConfig, project: Option<&std::path::Path>) -> Vec<AgentProfile> {
        config
            .agent_profiles
            .iter()
            .filter(|entry| config.agent_profile_applies(entry, project))
            .filter_map(|entry| match Self::for_project(entry, config, project) {
                Ok(profile) => Some(profile),
                Err(reason) => {
                    log::warn!("agent_profiles: skipping '{}': {reason}", entry.name);
                    None
                }
            })
            .collect()
    }

    pub fn agency_defaults() -> Vec<AgentProfileConfig> {
        paneflow_mcp_install::agency::STARTER_ROLES
            .iter()
            .filter_map(|id| paneflow_mcp_install::agency::role(id))
            .map(|role| AgentProfileConfig {
                name: format!("Agency {}", role.name),
                agent: "claude_code".to_string(),
                model: Some(role.default_model.to_string()),
                agency_role: Some(role.id.to_string()),
                ..AgentProfileConfig::default()
            })
            .collect()
    }

    pub fn launch_command(&self, config: &PaneFlowConfig) -> String {
        let mut spec = self.agent.launch_spec(config);
        for arg in &self.args {
            spec.push_arg(arg.clone());
        }
        wrap_for_shell(
            &spec.render_shell_command_for_shell(config.default_shell.as_deref()),
            config,
        )
    }

    pub fn process_env(&self) -> HashMap<String, String> {
        let home = dirs::home_dir();
        let lookup = |name: &str| std::env::var(name).ok();
        self.env
            .iter()
            .map(|(key, value)| {
                (
                    key.clone(),
                    expand_profile_value(key, value, home.as_deref(), &lookup),
                )
            })
            .collect()
    }
}

pub(crate) fn is_env_key(key: &str) -> bool {
    !key.is_empty()
        && !key.starts_with(|c: char| c.is_ascii_digit())
        && key.chars().all(|c| c.is_ascii_alphanumeric() || c == '_')
}

fn expand_profile_value(
    key: &str,
    value: &str,
    home: Option<&std::path::Path>,
    lookup: &dyn Fn(&str) -> Option<String>,
) -> String {
    match crate::env_expand::expand_env_value(value, home, lookup) {
        Ok(expanded) => expanded,
        Err(name) => {
            log::warn!(
                "agent_profiles: '{key}' references '{name}', which is not set; passing the value through unchanged"
            );
            value.to_string()
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum AgentLaunch {
    Builtin(TerminalAgent),
    Preset(TerminalAgent, &'static RuntimeSuggestedPreset),
    Profile(AgentProfile),
}

fn with_extra_presets(agents: impl Iterator<Item = TerminalAgent>) -> Vec<AgentLaunch> {
    agents
        .flat_map(|agent| {
            std::iter::once(AgentLaunch::Builtin(agent)).chain(
                agent
                    .runtime()
                    .suggested_presets
                    .iter()
                    .skip(1)
                    .filter(|preset| preset.supports_current_platform())
                    .map(move |preset| AgentLaunch::Preset(agent, preset)),
            )
        })
        .collect()
}

impl AgentLaunch {
    pub fn all(config: &PaneFlowConfig, project: Option<&std::path::Path>) -> Vec<AgentLaunch> {
        with_extra_presets(TerminalAgent::for_this_platform())
            .into_iter()
            .chain(
                AgentProfile::all(config, project)
                    .into_iter()
                    .filter(|profile| profile.agent.supports_current_platform())
                    .map(AgentLaunch::Profile),
            )
            .collect()
    }

    pub fn visible(config: &PaneFlowConfig, project: Option<&std::path::Path>) -> Vec<AgentLaunch> {
        with_extra_presets(
            TerminalAgent::visible(config)
                .into_iter()
                .filter(|agent| agent.supports_current_platform()),
        )
        .into_iter()
        .chain(
            AgentProfile::all(config, project)
                .into_iter()
                .filter(|profile| profile.agent.supports_current_platform())
                .map(AgentLaunch::Profile),
        )
        .collect()
    }

    pub fn agent(&self) -> TerminalAgent {
        match self {
            AgentLaunch::Builtin(agent) | AgentLaunch::Preset(agent, _) => *agent,
            AgentLaunch::Profile(profile) => profile.agent,
        }
    }

    pub fn label(&self) -> String {
        match self {
            AgentLaunch::Builtin(agent) => agent.display_name().to_string(),
            AgentLaunch::Preset(agent, preset) => preset
                .name
                .unwrap_or_else(|| agent.display_name())
                .to_string(),
            AgentLaunch::Profile(profile) => profile.name.clone(),
        }
    }

    pub fn tmux_compat(&self) -> bool {
        matches!(self, AgentLaunch::Preset(_, preset) if preset.tmux_compat)
    }

    pub fn is_installed(&self) -> bool {
        self.agent().is_installed()
    }

    pub fn launch_command(&self, config: &PaneFlowConfig) -> String {
        match self {
            AgentLaunch::Builtin(agent) => agent.launch_command(config),
            AgentLaunch::Preset(agent, preset) => {
                let command = agent
                    .preset_spec(preset.command, config)
                    .render_shell_command();
                let command = match self.tmux_prefix() {
                    Some(Ok(prefix)) => format!("{prefix} {command}"),
                    _ => command,
                };
                wrap_for_shell(&command, config)
            }
            AgentLaunch::Profile(profile) => profile.launch_command(config),
        }
    }

    pub fn tmux_prefix(&self) -> Option<Result<String, String>> {
        self.tmux_compat().then(|| {
            let dir = crate::tmux_compat::compat_dir()
                .ok_or_else(|| "the Paneflow home directory cannot be resolved".to_string())?;
            crate::tmux_compat::env_prefix(&dir, crate::tmux_compat::teams::LEADER)
        })
    }

    pub fn process_env(&self) -> Option<HashMap<String, String>> {
        match self {
            AgentLaunch::Builtin(_) => None,
            AgentLaunch::Preset(_, preset) => Some(
                preset
                    .env
                    .iter()
                    .map(|(key, value)| ((*key).to_string(), (*value).to_string()))
                    .collect::<HashMap<_, _>>(),
            )
            .filter(|env| !env.is_empty()),
            AgentLaunch::Profile(profile) => {
                Some(profile.process_env()).filter(|env| !env.is_empty())
            }
        }
    }
}

#[cfg(test)]
fn prime_installed_binary_cache(found: HashSet<&'static str>) {
    let mut cache = lock_installed_binary_cache();
    cache.found = found;
    cache.checked_at = Some(Instant::now());
    cache.refreshing = false;
}

pub(crate) fn prepare_profile_roles(profiles: &[AgentProfileConfig]) -> std::io::Result<()> {
    let ids: Vec<_> = profiles
        .iter()
        .filter_map(|profile| profile.agency_role.as_deref())
        .filter(|id| paneflow_mcp_install::agency::role(id).is_some())
        .collect();
    prepare_role_ids(&ids)
}

pub(crate) fn prepare_role_ids(ids: &[&str]) -> std::io::Result<()> {
    if ids.is_empty() {
        return Ok(());
    }
    let home = paneflow_home::paneflow_home().ok_or_else(|| {
        std::io::Error::new(std::io::ErrorKind::NotFound, "cannot resolve Paneflow home")
    })?;
    paneflow_mcp_install::agency::prepare_roles(&home, ids.iter().copied())
}

#[cfg(test)]
mod tests {
    #[test]
    fn installed_binary_reads_never_scan_on_the_caller_thread_once_warm() {
        super::prime_installed_binary_cache(std::collections::HashSet::from(["claude"]));
        assert!(!super::installed_binary_scan_pending());
        let started = std::time::Instant::now();
        for _ in 0..1_000 {
            assert!(super::installed_binaries_contains("claude"));
            assert!(!super::installed_binaries_contains(
                "paneflow-no-such-agent-binary"
            ));
        }
        assert!(
            started.elapsed() < std::time::Duration::from_secs(2),
            "warm reads must not walk PATH"
        );
    }

    use super::*;

    #[test]
    fn launch_command_declares_its_own_agent() {
        let config = PaneFlowConfig::default();
        for agent in TerminalAgent::all() {
            assert_eq!(
                TerminalAgent::from_launch_command(&agent.launch_command(&config)),
                Some(agent),
                "{} launch command must declare itself",
                agent.display_name()
            );
        }
    }

    #[test]
    fn executable_stem_strips_paths_and_windows_wrappers() {
        assert_eq!(executable_stem(r"C:\tools\codex.exe"), "codex");
        assert_eq!(executable_stem("vite.CMD"), "vite");
        assert_eq!(executable_stem("vite.cmd"), "vite");
        assert_eq!(executable_stem("script.ps1"), "script");
        assert_eq!(executable_stem("/usr/local/bin/claude"), "claude");
    }

    #[test]
    fn from_launch_command_handles_paths_env_and_windows_suffixes() {
        assert_eq!(
            TerminalAgent::from_launch_command("/usr/local/bin/claude --resume abc"),
            Some(TerminalAgent::ClaudeCode)
        );
        assert_eq!(
            TerminalAgent::from_launch_command("RUST_LOG=info NO_COLOR=1 codex"),
            Some(TerminalAgent::Codex)
        );
        assert_eq!(
            TerminalAgent::from_launch_command("C:\\Users\\a\\bin\\claude.CMD"),
            Some(TerminalAgent::ClaudeCode)
        );
        assert_eq!(TerminalAgent::from_launch_command("npm run claude"), None);
        assert_eq!(TerminalAgent::from_launch_command("claude-wrapper"), None);
        assert_eq!(TerminalAgent::from_launch_command(""), None);
        assert_eq!(TerminalAgent::from_launch_command("   "), None);
        assert_eq!(TerminalAgent::from_launch_command("--model=x codex"), None);
    }

    #[test]
    fn tag_roundtrip() {
        for agent in TerminalAgent::all() {
            assert_eq!(TerminalAgent::from_tag(agent.tag()), Some(agent));
        }
        assert_eq!(TerminalAgent::from_tag("unknown"), None);
    }

    #[test]
    fn from_tag_rejects_hostile_session_values() {
        assert_eq!(TerminalAgent::from_tag(""), None);
        assert_eq!(
            TerminalAgent::from_tag("Claude_Code"),
            None,
            "case-sensitive"
        );
        assert_eq!(TerminalAgent::from_tag("claude_code "), None, "no trim");
        assert_eq!(TerminalAgent::from_tag("claude_code\u{202e}"), None);
        assert_eq!(TerminalAgent::from_tag("codex\n"), None);
        assert_eq!(TerminalAgent::from_tag(&"x".repeat(10_000)), None);
    }

    #[test]
    fn binary_roundtrip_via_from_binary() {
        for agent in TerminalAgent::all() {
            assert_eq!(TerminalAgent::from_binary(agent.binary()), Some(agent));
        }
        assert_eq!(TerminalAgent::from_binary("bash"), None);
        assert_eq!(TerminalAgent::from_binary("claude-code-cli"), None);
    }

    #[test]
    fn binary_is_launch_command_leading_token() {
        let cfg = PaneFlowConfig::default();
        for agent in TerminalAgent::all() {
            let command = agent.command(&cfg);
            let leading = command.split_whitespace().next().unwrap_or_default();
            assert_eq!(
                leading,
                agent.binary(),
                "{} binary must match its launch command's leading token",
                agent.display_name()
            );
        }
    }

    #[test]
    fn explicit_visibility_overrides_install_detection() {
        let shown: PaneFlowConfig =
            serde_json::from_value(serde_json::json!({ "gemini_button_visible": true })).unwrap();
        assert!(TerminalAgent::Gemini.is_visible(&shown));

        let hidden: PaneFlowConfig =
            serde_json::from_value(serde_json::json!({ "gemini_button_visible": false })).unwrap();
        assert!(!TerminalAgent::Gemini.is_visible(&hidden));
    }

    #[test]
    fn icon_paths_are_embedded_assets() {
        for agent in TerminalAgent::all() {
            let p = agent.icon_path();
            assert!(
                p.starts_with("icons/") || p.starts_with("agents/"),
                "{} icon path `{p}` is not under an embedded asset root",
                agent.display_name()
            );
        }
    }

    #[test]
    fn claude_bypass_flag_toggles_command() {
        let off = PaneFlowConfig {
            claude_code_bypass_permissions: Some(false),
            ..Default::default()
        };
        assert_eq!(TerminalAgent::ClaudeCode.command(&off), "claude");
        let on = PaneFlowConfig {
            claude_code_bypass_permissions: Some(true),
            ..Default::default()
        };
        assert_eq!(
            TerminalAgent::ClaudeCode.command(&on),
            "claude --permission-mode bypassPermissions"
        );
    }

    #[test]
    fn codex_bypass_flag_is_opt_in_and_codex_only() {
        let default_config = PaneFlowConfig::default();
        assert_eq!(TerminalAgent::Codex.command(&default_config), "codex");

        let config = PaneFlowConfig {
            codex_bypass_permissions: Some(true),
            ..default_config
        };
        assert_eq!(
            TerminalAgent::Codex.command(&config),
            "codex --dangerously-bypass-approvals-and-sandbox"
        );
        assert_eq!(TerminalAgent::ClaudeCode.command(&config), "claude");
    }

    #[test]
    fn non_claude_agents_ignore_bypass() {
        let config = PaneFlowConfig {
            claude_code_bypass_permissions: Some(true),
            ..Default::default()
        };
        assert_eq!(TerminalAgent::Codex.command(&config), "codex");
        assert_eq!(TerminalAgent::Pi.command(&config), "pi");
        assert_eq!(TerminalAgent::Hermes.command(&config), "hermes");
    }

    #[test]
    fn launch_spec_keeps_program_and_args_structured_until_render() {
        let cfg = PaneFlowConfig {
            claude_code_bypass_permissions: Some(true),
            ..Default::default()
        };

        let spec = TerminalAgent::ClaudeCode.launch_spec(&cfg);

        assert_eq!(spec.program, "claude");
        assert_eq!(spec.args, vec!["--permission-mode", "bypassPermissions"]);
        assert_eq!(
            spec.render_shell_command(),
            "claude --permission-mode bypassPermissions"
        );
    }

    #[cfg(unix)]
    #[test]
    fn generated_role_paths_are_one_literal_shell_argument() {
        let value = "model_instructions_file=/tmp/a path/o'neil/$(false)/role.md";
        let mut command = AgentCommandSpec::new("printf");
        command.push_arg("%s");
        command.push_arg(value);
        let rendered = command.render_shell_command_for_shell(Some("bash"));
        let output = std::process::Command::new("sh")
            .args(["-c", &rendered])
            .output()
            .unwrap();
        assert!(output.status.success());
        assert_eq!(String::from_utf8(output.stdout).unwrap(), value);
    }

    #[test]
    fn launch_spec_plain_token_guard_matches_agent_command_surface() {
        for agent in TerminalAgent::all() {
            assert!(
                is_plain_shell_token(agent.binary()),
                "{} binary must stay a plain shell token",
                agent.display_name()
            );
            for arg in agent.runtime().suggested_presets[0]
                .command
                .split_whitespace()
                .skip(1)
            {
                assert!(
                    is_plain_shell_token(arg),
                    "{} arg `{arg}` must stay a plain shell token",
                    agent.display_name()
                );
            }
        }
        assert!(is_plain_shell_token(SAMPLE_UUID));
        assert!(!is_plain_shell_token("two words"));
        assert!(!is_plain_shell_token("$(reboot)"));
    }

    const SAMPLE_UUID: &str = "550e8400-e29b-41d4-a716-446655440000";

    #[test]
    fn catalog_tables_match_the_pre_catalog_reference() {
        let reference: [(&str, &str, Option<&str>); 13] = [
            (
                "com.anthropic.claude-code",
                "claude_code_button_visible",
                Some("claude"),
            ),
            ("com.openai.codex", "codex_button_visible", Some("codex")),
            (
                "ai.opencode.cli",
                "opencode_button_visible",
                Some("opencode"),
            ),
            ("dev.mariozechner.pi", "pi_button_visible", Some("pi")),
            ("ai.hermes.agent", "hermes_agent_button_visible", None),
            ("ai.x.grok-cli", "grok_button_visible", Some("grok")),
            ("com.sourcegraph.amp", "amp_button_visible", None),
            (
                "com.google.gemini-cli",
                "gemini_button_visible",
                Some("gemini"),
            ),
            ("com.github.copilot-cli", "copilot_button_visible", None),
            ("com.factory.droid", "factory_button_visible", None),
            ("ai.openclaw.cli", "openclaw_button_visible", None),
            ("com.muse.code", "muse_button_visible", None),
            ("sh.fx.cli", "fx_button_visible", None),
        ];
        let generated = TerminalAgent::all()
            .map(|agent| {
                (
                    agent.runtime().id,
                    agent.visibility_config_key(),
                    agent.session_agent().map(|session| session.reader().name()),
                )
            })
            .collect::<Vec<_>>();
        assert_eq!(generated, reference);

        let everything_visible: PaneFlowConfig = serde_json::from_value(serde_json::Value::Object(
            reference
                .iter()
                .map(|(_, key, _)| (key.to_string(), serde_json::Value::Bool(true)))
                .collect(),
        ))
        .unwrap();
        assert_eq!(
            TerminalAgent::visible(&everything_visible)
                .into_iter()
                .map(|agent| agent.runtime().id)
                .collect::<Vec<_>>(),
            reference.iter().map(|(id, _, _)| *id).collect::<Vec<_>>(),
            "launcher order"
        );
        assert_eq!(
            crate::agent_sessions::SessionAgent::all()
                .map(|session| session.terminal_agent().runtime().id)
                .collect::<Vec<_>>(),
            [
                "com.anthropic.claude-code",
                "com.openai.codex",
                "ai.opencode.cli",
                "dev.mariozechner.pi",
                "ai.x.grok-cli",
                "com.google.gemini-cli",
            ],
            "session sidebar groups keep the order of the former SessionAgent enum"
        );
        assert_eq!(
            crate::auto_naming::summarizers().collect::<Vec<_>>(),
            [
                TerminalAgent::ClaudeCode,
                TerminalAgent::Codex,
                TerminalAgent::Opencode,
                TerminalAgent::Pi,
            ],
            "summarizer order"
        );
    }

    #[test]
    fn bare_commands_preserve_multi_token_agent_commands() {
        let cfg = PaneFlowConfig::default();
        assert_eq!(TerminalAgent::Openclaw.command(&cfg), "openclaw tui");
        assert_eq!(TerminalAgent::MuseCode.command(&cfg), "muse");
    }

    fn profile_entry(name: &str, agent: &str) -> AgentProfileConfig {
        AgentProfileConfig {
            name: name.to_string(),
            agent: agent.to_string(),
            model: None,
            agency_role: None,
            effort: None,
            project: None,
            default_profile: None,
            skills: Vec::new(),
            env: BTreeMap::from([(
                "CLAUDE_CONFIG_DIR".to_string(),
                "~/.claude-perso".to_string(),
            )]),
            args: vec!["--model".to_string(), "opus".to_string()],
        }
    }

    #[test]
    fn agency_profiles_apply_their_role_and_selected_model_without_flag_injection() {
        let mut entry = AgentProfile::agency_defaults()
            .into_iter()
            .find(|entry| entry.agency_role.as_deref() == Some("ui-designer"))
            .unwrap();
        entry.model = Some("haiku".to_string());
        let profile = AgentProfile::from_config(&entry).unwrap();
        assert_eq!(
            &profile.args[..3],
            [
                "--agent",
                "paneflow-agency:paneflow-agency-ui-designer",
                "--plugin-dir"
            ]
        );
        assert!(profile.args[3].ends_with("ui-designer.plugin"));
        assert_eq!(&profile.args[4..], ["--model", "haiku"]);
        for model in ["", "-p", "sonnet;exit", "two words"] {
            entry.model = Some(model.to_string());
            assert!(AgentProfile::from_config(&entry).is_err(), "{model}");
        }
        entry.model = Some("sonnet".to_string());
        entry.args = vec!["--model=opus".to_string()];
        assert!(AgentProfile::from_config(&entry).is_err());
        entry.args.clear();
        entry.agent = "codex".to_string();
        entry.model = Some("gpt-6".to_string());
        entry.effort = Some("high".to_string());
        let profile = AgentProfile::from_config(&entry).unwrap();
        assert!(profile.args[1].starts_with("model_instructions_file="));
        assert!(
            profile
                .args
                .iter()
                .any(|arg| arg == "model_reasoning_effort=high")
        );
        assert!(
            profile
                .launch_command(&PaneFlowConfig::default())
                .contains("codex")
        );
        entry.effort = None;
        entry.agent = "claude_code".to_string();
        entry.agency_role = Some("unknown".to_string());
        assert!(AgentProfile::from_config(&entry).is_err());
    }

    #[test]
    fn project_launcher_hides_other_projects_and_validates_native_effort_options() {
        let project = std::env::temp_dir().join("felco-app");
        let mut config = PaneFlowConfig {
            agent_profiles: vec![
                profile_entry("Shared", "codex"),
                profile_entry("Project", "codex"),
            ],
            ..Default::default()
        };
        config.agent_profiles[1].project = Some(project.clone());
        assert_eq!(AgentProfile::all(&config, Some(&project)).len(), 2);
        assert_eq!(AgentProfile::all(&config, None).len(), 1);
        assert_eq!(
            AgentProfile::all(&config, Some(&project.with_file_name("other-app"))).len(),
            1
        );
        for agent in [TerminalAgent::ClaudeCode, TerminalAgent::Codex] {
            let mut entry = profile_entry("Test", agent.tag());
            for effort in agent.effort_levels() {
                entry.effort = Some((*effort).to_string());
                assert!(AgentProfile::from_config(&entry).is_ok(), "{effort}");
            }
            entry.effort = Some(
                if agent == TerminalAgent::Codex {
                    "unsupported"
                } else {
                    "minimal"
                }
                .to_string(),
            );
            assert!(AgentProfile::from_config(&entry).is_err());
        }
    }

    #[test]
    fn selected_skills_reach_both_native_launches_without_global_installation() {
        let path = std::env::temp_dir().join("selected-skill").join("SKILL.md");
        for agent in [TerminalAgent::ClaudeCode, TerminalAgent::Codex] {
            let mut entry = profile_entry("Skilled", agent.tag());
            entry.args.clear();
            entry.skills = vec![path.clone()];
            let profile = AgentProfile::from_config(&entry).unwrap();
            assert!(
                profile
                    .args
                    .iter()
                    .any(|arg| arg.contains("Skills assigned to this agent"))
            );
            assert_eq!(profile.skills.as_slice(), std::slice::from_ref(&path));
            if agent == TerminalAgent::Codex {
                let value = profile
                    .args
                    .iter()
                    .find_map(|arg| arg.strip_prefix("developer_instructions="))
                    .unwrap();
                let parsed: toml::Value = format!("instructions={value}").parse().unwrap();
                assert!(
                    parsed["instructions"]
                        .as_str()
                        .unwrap()
                        .contains("SKILL.md")
                );
            } else {
                assert!(
                    profile
                        .args
                        .iter()
                        .any(|arg| arg == "--append-system-prompt")
                );
            }
            entry.args = vec!["developer_instructions=override".to_string()];
            assert!(AgentProfile::from_config(&entry).is_err());
        }
    }

    #[test]
    fn orchestrator_model_defaults_follow_the_project_copy_without_flattening_the_setting() {
        let project = std::env::temp_dir().join("felco-app");
        let mut lead = profile_entry("Orchestrator", "codex");
        lead.args.clear();
        lead.agency_role = Some("orchestrator".to_string());
        lead.model = Some("gpt-reserve".to_string());
        let mut local = lead.clone();
        local.name = "felco-app Orchestrator".to_string();
        local.default_profile = Some(lead.name.clone());
        local.project = Some(project.clone());
        local.model = Some("gpt-6.2-sol".to_string());
        let mut config = PaneFlowConfig {
            agent_profiles: vec![lead, local],
            ..Default::default()
        };
        let mut worker = profile_entry("felco-app Designer", "codex");
        worker.args.clear();
        worker.project = Some(project.clone());
        worker.model = Some(ORCHESTRATOR_MODEL.to_string());
        worker.effort = Some("high".to_string());
        let args = AgentProfile::for_project(&worker, &config, Some(&project))
            .unwrap()
            .args;
        assert!(
            args.windows(2)
                .any(|pair| pair == ["--model", "gpt-6.2-sol"])
        );
        assert!(args.iter().any(|arg| arg == "model_reasoning_effort=high"));
        config.agent_profiles[1].model = Some("gpt-6.2-astra".to_string());
        assert!(
            AgentProfile::for_project(&worker, &config, Some(&project))
                .unwrap()
                .args
                .iter()
                .any(|arg| arg == "gpt-6.2-astra")
        );
        worker.project = Some(project.with_file_name("other-app"));
        assert!(
            AgentProfile::for_project(&worker, &config, worker.project.as_deref())
                .unwrap()
                .args
                .iter()
                .any(|arg| arg == "gpt-reserve")
        );
        assert_eq!(worker.model.as_deref(), Some(ORCHESTRATOR_MODEL));
        assert!(AgentProfile::from_config(&worker).is_err());
    }

    #[test]
    fn orchestrator_model_defaults_refuse_cross_cli_ambiguity_and_self_inheritance() {
        let project = std::env::temp_dir().join("felco-app");
        let mut lead = profile_entry("Orchestrator", "claude_code");
        lead.args.clear();
        lead.agency_role = Some("orchestrator".to_string());
        lead.model = Some("haiku".to_string());
        let mut config = PaneFlowConfig {
            agent_profiles: vec![lead],
            ..Default::default()
        };
        assert!(
            AgentProfile::orchestrator_model(&config, &project, TerminalAgent::Codex)
                .unwrap_err()
                .contains("same base agent")
        );
        assert_eq!(
            AgentProfile::orchestrator_model(&config, &project, TerminalAgent::ClaudeCode).unwrap(),
            "haiku"
        );
        config.agent_profiles[0].model = None;
        assert!(
            AgentProfile::orchestrator_model(&config, &project, TerminalAgent::ClaudeCode).is_err()
        );
        config.agent_profiles[0].model = Some(ORCHESTRATOR_MODEL.to_string());
        assert!(
            AgentProfile::orchestrator_model(&config, &project, TerminalAgent::ClaudeCode).is_err()
        );
        config.agent_profiles[0].model = Some("haiku".to_string());
        let mut alternative = config.agent_profiles[0].clone();
        alternative.name = "Alternative Orchestrator".to_string();
        config.agent_profiles.push(alternative);
        assert!(
            AgentProfile::orchestrator_model(&config, &project, TerminalAgent::ClaudeCode)
                .unwrap_err()
                .contains("exactly one")
        );
        let mut project_lead = config.agent_profiles[0].clone();
        project_lead.name = "Project Orchestrator".to_string();
        project_lead.project = Some(project.clone());
        config.agent_profiles.push(project_lead);
        assert_eq!(
            AgentProfile::orchestrator_model(&config, &project, TerminalAgent::ClaudeCode).unwrap(),
            "haiku"
        );
    }

    #[test]
    fn profile_launch_command_extends_base_agent_and_declares_it() {
        let config = PaneFlowConfig {
            claude_code_bypass_permissions: Some(true),
            ..PaneFlowConfig::default()
        };
        let profile =
            AgentProfile::from_config(&profile_entry("Claude perso", "claude_code")).unwrap();
        let command = profile.launch_command(&config);
        assert!(
            command.ends_with("claude --permission-mode bypassPermissions --model opus"),
            "{command}"
        );
        assert_eq!(
            TerminalAgent::from_launch_command(&command),
            Some(TerminalAgent::ClaudeCode)
        );
    }

    #[test]
    fn profile_rejects_unknown_agent_blank_name_and_unsafe_tokens() {
        assert!(AgentProfile::from_config(&profile_entry("x", "claude")).is_err());
        assert!(AgentProfile::from_config(&profile_entry("  ", "claude_code")).is_err());
        let mut bad_arg = profile_entry("x", "claude_code");
        bad_arg.args = vec!["--model opus; rm -rf /".to_string()];
        assert!(AgentProfile::from_config(&bad_arg).is_err());
        let mut bad_env = profile_entry("x", "claude_code");
        bad_env.env = BTreeMap::from([("1BAD KEY".to_string(), "v".to_string())]);
        assert!(AgentProfile::from_config(&bad_env).is_err());
    }

    #[test]
    fn invalid_profiles_are_skipped_not_fatal() {
        let config = PaneFlowConfig {
            agent_profiles: vec![profile_entry("Good", "codex"), profile_entry("Bad", "nope")],
            ..PaneFlowConfig::default()
        };
        let profiles = AgentProfile::all(&config, None);
        assert_eq!(profiles.len(), 1);
        assert_eq!(profiles[0].agent, TerminalAgent::Codex);
        assert_eq!(
            AgentLaunch::all(&config, None).len(),
            TerminalAgent::all()
                .filter(|agent| agent.supports_current_platform())
                .map(|agent| {
                    1 + agent
                        .runtime()
                        .suggested_presets
                        .iter()
                        .skip(1)
                        .filter(|preset| preset.supports_current_platform())
                        .count()
                })
                .sum::<usize>()
                + 1
        );
    }

    #[test]
    fn profile_env_expands_the_home_prefix_and_known_placeholders() {
        let home = std::path::Path::new("/home/u");
        let lookup = |name: &str| (name == "BASE").then(|| "/opt/base".to_string());
        assert_eq!(
            expand_profile_value("K", "~/.claude-perso", Some(home), &lookup),
            home.join(".claude-perso").display().to_string()
        );
        assert_eq!(
            expand_profile_value("K", "$BASE/x", Some(home), &lookup),
            "/opt/base/x"
        );
        assert_eq!(
            expand_profile_value("K", "%BASE%/x", Some(home), &lookup),
            "/opt/base/x"
        );
    }

    #[test]
    fn profile_env_passes_an_unresolvable_value_through_unchanged() {
        let home = std::path::Path::new("/home/u");
        let lookup = |_: &str| None;
        assert_eq!(
            expand_profile_value("K", "$NOPE/.claude-perso", Some(home), &lookup),
            "$NOPE/.claude-perso"
        );
        assert_eq!(
            expand_profile_value("K", "a%b%c", Some(home), &lookup),
            "a%b%c"
        );
    }

    #[test]
    fn profile_keeps_name_env_and_args_from_config() {
        let entry = profile_entry("  Claude perso ", "claude_code");
        let profile = AgentProfile::from_config(&entry).unwrap();
        assert_eq!(profile.name, "Claude perso");
        assert_eq!(profile.agent, TerminalAgent::ClaudeCode);
        assert_eq!(profile.env, entry.env);
        assert_eq!(profile.args, entry.args);
    }

    #[test]
    fn parse_version_takes_the_first_dotted_number() {
        assert_eq!(parse_version("2.1.0 (Claude Code)"), Some("2.1.0".into()));
        assert_eq!(parse_version("codex-cli 0.58.0"), Some("0.58.0".into()));
        assert_eq!(
            parse_version("v1.2.4-beta.1\n"),
            Some("1.2.4-beta.1".into())
        );
        assert_eq!(
            parse_version("Gemini CLI version: 0.22.1,"),
            Some("0.22.1".into())
        );
        assert_eq!(parse_version("no numbers here"), None);
        assert_eq!(parse_version("build 20260908"), None);
    }

    #[test]
    fn launch_presets_follow_catalog_platforms_without_hiding_presentation() {
        let config = PaneFlowConfig::default();
        let builtins = AgentLaunch::all(&config, None)
            .into_iter()
            .filter_map(|launch| match launch {
                AgentLaunch::Builtin(agent) => Some(agent),
                AgentLaunch::Preset(..) | AgentLaunch::Profile(_) => None,
            })
            .collect::<Vec<_>>();
        assert!(
            builtins
                .iter()
                .all(|agent| agent.supports_current_platform())
        );
        if cfg!(windows) {
            assert_eq!(builtins.len(), 5);
            assert_eq!(
                TerminalAgent::from_tag("grok").map(TerminalAgent::display_name),
                Some("Grok")
            );
            assert!(!builtins.contains(&TerminalAgent::Grok));
        } else {
            assert_eq!(builtins.len(), TerminalAgent::all().count());
        }
    }

    #[test]
    fn the_claude_team_preset_launches_through_the_tmux_shim_off_windows_only() {
        let config = PaneFlowConfig::default();
        let team = AgentLaunch::all(&config, None)
            .into_iter()
            .find(AgentLaunch::tmux_compat);
        if cfg!(windows) {
            assert!(team.is_none(), "the tmux team mode has no Windows path");
            return;
        }
        let team = team.expect("the team preset is offered on Linux and macOS");
        assert_eq!(team.agent(), TerminalAgent::ClaudeCode);
        assert_eq!(team.label(), "Claude Code (team)");
        let command = team.launch_command(&config);
        assert!(
            command.contains("TMUX=paneflow-tmux-compat,0,0 TMUX_PANE=%0 PATH=\""),
            "{command}"
        );
        assert!(
            command.contains("/bin/tmux-compat:$PATH\" claude --teammate-mode tmux"),
            "{command}"
        );
        assert_eq!(
            TerminalAgent::from_launch_command(&command),
            Some(TerminalAgent::ClaudeCode)
        );
        assert_eq!(
            team.process_env()
                .and_then(|env| env.get("CLAUDE_CODE_EXPERIMENTAL_AGENT_TEAMS").cloned())
                .as_deref(),
            Some("1")
        );
        let preset = TerminalAgent::ClaudeCode.runtime().suggested_presets[1];
        assert_eq!(
            preset.platforms,
            Some(&[RuntimePlatform::Linux, RuntimePlatform::Macos][..])
        );
    }

    #[test]
    fn fx_is_offered_on_linux_and_macos_and_never_on_windows() {
        assert!(
            !TerminalAgent::on_platform(RuntimePlatform::Windows).any(|a| a == TerminalAgent::Fx)
        );
        for platform in [RuntimePlatform::Linux, RuntimePlatform::Macos] {
            assert!(TerminalAgent::on_platform(platform).any(|a| a == TerminalAgent::Fx));
        }
        assert_eq!(
            TerminalAgent::on_platform(RuntimePlatform::Windows)
                .map(|agent| agent.runtime().slug)
                .collect::<Vec<_>>(),
            ["claude-code", "codex", "amp", "gemini", "github-copilot"]
        );
        let launcher = AgentLaunch::all(&PaneFlowConfig::default(), None);
        assert_eq!(
            launcher
                .iter()
                .any(|launch| launch.agent() == TerminalAgent::Fx),
            !cfg!(windows)
        );
    }

    #[test]
    fn visibility_config_key_matches_is_visible_field() {
        for agent in TerminalAgent::all() {
            let json = serde_json::json!({ agent.visibility_config_key(): false });
            let config: PaneFlowConfig = serde_json::from_value(json).unwrap();
            assert!(!agent.is_visible(&config), "{}", agent.display_name());
        }
    }
}
