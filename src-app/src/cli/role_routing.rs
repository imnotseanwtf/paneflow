use std::collections::BTreeMap;
use std::io::Read;
use std::path::Path;

use paneflow_config::schema::PaneFlowConfig;

use super::CliError;
use super::workspace_spec::PaneSpec;

const STATE_LIMIT: u64 = 64 * 1024;

type Cursors = BTreeMap<String, String>;

pub(super) fn apply(
    panes: &mut [PaneSpec],
    config: &PaneFlowConfig,
    dry_run: bool,
) -> Result<(), CliError> {
    let home = paneflow_home::paneflow_home()
        .ok_or_else(|| CliError::runtime("cannot resolve Paneflow home for role rotation"))?;
    apply_in(&home, panes, config, dry_run)
}

fn apply_in(
    home: &Path,
    panes: &mut [PaneSpec],
    config: &PaneFlowConfig,
    dry_run: bool,
) -> Result<(), CliError> {
    let path = home.join("cache").join("agency-role-rotation.json");
    let _lock = if dry_run {
        None
    } else {
        Some(
            paneflow_agent_config::lock_config_in(home, &path)
                .map_err(|error| CliError::runtime(error.to_string()))?,
        )
    };
    let mut cursors = read_cursors(&path)?;
    let previous = cursors.clone();
    rotate(panes, config, &mut cursors)?;
    if !dry_run && previous != cursors {
        let bytes =
            serde_json::to_vec(&cursors).map_err(|error| CliError::runtime(error.to_string()))?;
        paneflow_home::write_atomically(&path, &bytes)
            .map_err(|error| CliError::runtime(format!("cannot save role rotation: {error}")))?;
    }
    Ok(())
}

fn read_cursors(path: &Path) -> Result<Cursors, CliError> {
    let file = match paneflow_home::open_for_reading(path) {
        Ok(file) => file,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(Cursors::new()),
        Err(error) => {
            return Err(CliError::runtime(format!(
                "cannot read role rotation: {error}"
            )));
        }
    };
    let mut bytes = Vec::new();
    file.take(STATE_LIMIT + 1)
        .read_to_end(&mut bytes)
        .map_err(|error| CliError::runtime(error.to_string()))?;
    if bytes.len() as u64 > STATE_LIMIT {
        return Err(CliError::runtime("role rotation state exceeds 64 KiB"));
    }
    serde_json::from_slice(&bytes).map_err(|error| {
        CliError::runtime(format!(
            "invalid role rotation state in {}: {error}",
            path.display()
        ))
    })
}

fn rotate(
    panes: &mut [PaneSpec],
    config: &PaneFlowConfig,
    cursors: &mut Cursors,
) -> Result<(), CliError> {
    for pane in panes {
        let Some(role) = pane
            .agent
            .as_deref()
            .and_then(|agent| agent.strip_prefix("agency-"))
        else {
            continue;
        };
        let project = super::up_cmd::profile_project(pane, config);
        if config.agent_profiles.iter().any(|profile| {
            config.agent_profile_applies(profile, project.as_deref())
                && (pane
                    .agent
                    .as_deref()
                    .is_some_and(|agent| profile.name.eq_ignore_ascii_case(agent))
                    || profile
                        .default_profile
                        .as_deref()
                        .zip(pane.agent.as_deref())
                        .is_some_and(|(name, agent)| name.eq_ignore_ascii_case(agent)))
        }) {
            continue;
        }
        let profiles: Vec<_> = config
            .agency_profiles_for_project(role, project.as_deref())
            .collect();
        if profiles.is_empty() {
            continue;
        }
        let cursor = project.as_ref().map_or_else(
            || role.to_string(),
            |project| format!("{}::{role}", project.display()),
        );
        let next = cursors
            .get(&cursor)
            .and_then(|last| profiles.iter().position(|profile| &profile.name == last))
            .map_or(0, |index| (index + 1) % profiles.len());
        let profile = profiles[next];
        if config
            .agent_profiles
            .iter()
            .filter(|candidate| config.agent_profile_applies(candidate, project.as_deref()))
            .filter(|candidate| candidate.name.eq_ignore_ascii_case(&profile.name))
            .count()
            != 1
        {
            return Err(CliError::runtime(format!(
                "round robin requires unique profile names: '{}'",
                profile.name
            )));
        }
        crate::agent_launcher::AgentProfile::for_project(profile, config, project.as_deref())
            .map_err(CliError::runtime)?;
        cursors.insert(cursor, profile.name.clone());
        pane.agent = Some(profile.name.clone());
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use paneflow_config::schema::AgentProfileConfig;

    fn config() -> PaneFlowConfig {
        PaneFlowConfig {
            agent_profiles: [
                ("Claude Designer", "claude_code", "haiku"),
                ("Codex Designer", "codex", "gpt-6"),
            ]
            .map(|(name, agent, model)| AgentProfileConfig {
                name: name.into(),
                agent: agent.into(),
                model: Some(model.into()),
                agency_role: Some("ui-designer".into()),
                ..AgentProfileConfig::default()
            })
            .into(),
            ..PaneFlowConfig::default()
        }
    }

    fn panes() -> Vec<PaneSpec> {
        super::super::workspace_spec::load(
            "round_robin = true\n[[panes]]\nagent = \"agency-ui-designer\"\n",
        )
        .unwrap()
        .panes
    }

    #[test]
    fn rotation_persists_across_launches_and_dry_run_does_not_advance_it() {
        let home = tempfile::tempdir().unwrap();
        let config = config();
        let mut first = panes();
        apply_in(home.path(), &mut first, &config, false).unwrap();
        assert_eq!(first[0].agent.as_deref(), Some("Claude Designer"));
        let mut preview = panes();
        apply_in(home.path(), &mut preview, &config, true).unwrap();
        assert_eq!(preview[0].agent.as_deref(), Some("Codex Designer"));
        let mut next = panes();
        apply_in(home.path(), &mut next, &config, false).unwrap();
        assert_eq!(next[0].agent, preview[0].agent);
        let mut last = panes();
        apply_in(home.path(), &mut last, &config, false).unwrap();
        assert_eq!(last[0].agent.as_deref(), Some("Claude Designer"));
    }

    #[test]
    fn project_role_rotation_uses_separate_pools_and_cursors() {
        let root = tempfile::tempdir().unwrap();
        let base = crate::runtime_paths::strip_verbatim_prefix(
            std::fs::canonicalize(root.path()).unwrap(),
        );
        let projects = [base.join("felco-app"), base.join("other-app")];
        let mut scoped = config();
        scoped.agent_profiles.clear();
        for project in &projects {
            std::fs::create_dir(project).unwrap();
            for mut profile in config().agent_profiles {
                profile.name = format!(
                    "{} {}",
                    project.file_name().unwrap().to_string_lossy(),
                    profile.name
                );
                profile.project = Some(project.clone());
                scoped.agent_profiles.push(profile);
            }
        }
        let mut cursors = Cursors::new();
        let mut run = |project: &std::path::Path| {
            let mut team = panes();
            team[0].cwd = Some(project.to_string_lossy().into_owned());
            rotate(&mut team, &scoped, &mut cursors).unwrap();
            team.remove(0).agent.unwrap()
        };
        assert_eq!(run(&projects[0]), "felco-app Claude Designer");
        assert_eq!(run(&projects[1]), "other-app Claude Designer");
        assert_eq!(run(&projects[0]), "felco-app Codex Designer");
        assert_eq!(cursors.len(), 2);
    }

    #[test]
    fn one_team_rotates_repeated_roles_and_preserves_explicit_combinations() {
        let mut panes = vec![panes().remove(0), panes().remove(0), panes().remove(0)];
        panes[2].agent = Some("Claude Designer".into());
        let mut state = Cursors::new();
        rotate(&mut panes, &config(), &mut state).unwrap();
        assert_eq!(
            panes
                .iter()
                .map(|pane| pane.agent.as_deref().unwrap())
                .collect::<Vec<_>>(),
            ["Claude Designer", "Codex Designer", "Claude Designer"]
        );
        let mut config = config();
        config.agent_profiles[1].name = "Claude Designer".into();
        assert!(rotate(&mut self::panes(), &config, &mut state).is_err());
    }
}
