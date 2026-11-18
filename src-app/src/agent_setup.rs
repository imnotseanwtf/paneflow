use std::path::Path;

use paneflow_config::schema::{AgentProfileConfig, PaneFlowConfig};
use serde::{Deserialize, Serialize};
use serde_json::Value;

const FORMAT: &str = "paneflow-agent-setup";
const MAX_BYTES: u64 = 2 * 1024 * 1024;
const MAX_PROFILES: usize = 512;

#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct AgentSetup {
    format: String,
    version: u32,
    profiles: Value,
}

pub(crate) struct ImportReview {
    pub(crate) source: std::path::PathBuf,
    pub(crate) expected: Value,
    pub(crate) incoming: Vec<AgentProfileConfig>,
    pub(crate) added: usize,
    pub(crate) updated: usize,
    pub(crate) missing_skills: usize,
}

pub(crate) fn encode(profiles: &[AgentProfileConfig]) -> Result<Vec<u8>, String> {
    let value = serde_json::to_value(profiles).map_err(|error| error.to_string())?;
    decode_profiles(&value)?;
    let bytes = serde_json::to_vec_pretty(&AgentSetup {
        format: FORMAT.into(),
        version: 1,
        profiles: value,
    })
    .map_err(|error| error.to_string())?;
    if bytes.len() as u64 > MAX_BYTES {
        return Err("Agent setup exceeds the 2 MiB limit".into());
    }
    Ok(bytes)
}

fn decode_profiles(value: &Value) -> Result<Vec<AgentProfileConfig>, String> {
    let entries = value.as_array().ok_or("Profiles must be an array")?;
    if entries.len() > MAX_PROFILES {
        return Err("Agent setup supports at most 512 profiles".into());
    }
    let schema: Value = serde_json::from_str(include_str!("../../schemas/paneflow.schema.json"))
        .map_err(|error| error.to_string())?;
    let properties = schema["definitions"]["agentProfile"]["properties"]
        .as_object()
        .ok_or("Agent profile schema is unavailable")?;
    for entry in entries {
        let fields = entry.as_object().ok_or("Each profile must be an object")?;
        if let Some(key) = fields.keys().find(|key| !properties.contains_key(*key)) {
            return Err(format!(
                "Unknown profile field '{key}'; update Paneflow before importing"
            ));
        }
        if !fields.contains_key("name") || !fields.contains_key("agent") {
            return Err("Each profile requires name and agent".into());
        }
    }
    let profiles: Vec<AgentProfileConfig> =
        serde_json::from_value(value.clone()).map_err(|error| error.to_string())?;
    for (index, profile) in profiles.iter().enumerate() {
        if profiles[..index]
            .iter()
            .any(|other| same_identity(profile, other))
        {
            return Err(format!(
                "Duplicate profile '{}' in the same project",
                profile.name
            ));
        }
    }
    Ok(profiles)
}

fn same_identity(a: &AgentProfileConfig, b: &AgentProfileConfig) -> bool {
    let same_project = match (&a.project, &b.project) {
        (Some(a), Some(b)) => {
            #[cfg(windows)]
            {
                a.to_string_lossy().to_lowercase() == b.to_string_lossy().to_lowercase()
            }
            #[cfg(not(windows))]
            {
                a == b
            }
        }
        (None, None) => true,
        _ => false,
    };
    same_project && a.name.trim().eq_ignore_ascii_case(b.name.trim())
}

pub(crate) fn merge(
    current: &Value,
    incoming: &[AgentProfileConfig],
) -> Result<(Vec<AgentProfileConfig>, usize, usize), String> {
    let mut profiles = if current.is_null() {
        vec![]
    } else {
        decode_profiles(current)?
    };
    let mut added = 0;
    let mut updated = 0;
    for entry in incoming {
        if let Some(existing) = profiles
            .iter_mut()
            .find(|other| same_identity(entry, other))
        {
            *existing = entry.clone();
            updated += 1;
        } else {
            profiles.push(entry.clone());
            added += 1;
        }
    }
    if profiles.len() > MAX_PROFILES {
        return Err("Merged setup exceeds 512 profiles".into());
    }
    let config = PaneFlowConfig {
        agent_profiles: profiles.clone(),
        ..Default::default()
    };
    for entry in profiles.iter().filter(|_| !incoming.is_empty()) {
        crate::agent_launcher::AgentProfile::for_project(entry, &config, entry.project.as_deref())
            .map_err(|error| format!("{}: {error}", entry.name))?;
    }
    Ok((profiles, added, updated))
}

pub(crate) fn read_review(
    path: std::path::PathBuf,
    current: Value,
) -> Result<ImportReview, String> {
    let bytes =
        paneflow_home::read_regular_capped(&path, MAX_BYTES).map_err(|error| error.to_string())?;
    let setup: AgentSetup = serde_json::from_slice(&bytes).map_err(|error| error.to_string())?;
    if setup.format != FORMAT || setup.version != 1 {
        return Err("Unsupported agent setup format or version".into());
    }
    let incoming = decode_profiles(&setup.profiles)?;
    let (_, added, updated) = merge(&current, &incoming)?;
    let missing_skills = incoming
        .iter()
        .flat_map(|profile| &profile.skills)
        .filter(|path| {
            crate::agent_profile_context::validate_skill_paths(std::slice::from_ref(*path)).is_err()
        })
        .count();
    Ok(ImportReview {
        source: path,
        expected: current,
        incoming,
        added,
        updated,
        missing_skills,
    })
}

pub(crate) fn export(path: &Path, profiles: &[AgentProfileConfig]) -> Result<(), String> {
    let bytes = encode(profiles)?;
    let home = paneflow_home::paneflow_home().ok_or("Cannot determine Paneflow home")?;
    let target = export_target(path, &home)?;
    paneflow_home::write_atomically(&target, &bytes).map_err(|error| error.to_string())
}

fn export_target(path: &Path, home: &Path) -> Result<std::path::PathBuf, String> {
    let target = paneflow_home::resolve_write_target(path).map_err(|error| error.to_string())?;
    let parent = target
        .parent()
        .ok_or("Choose a file in an existing directory")?
        .canonicalize()
        .map_err(|error| error.to_string())?;
    let target = parent.join(target.file_name().ok_or("Choose a JSON filename")?);
    let home = home.canonicalize().map_err(|error| error.to_string())?;
    if target.starts_with(&home) {
        return Err("Export outside Paneflow's state directory".into());
    }
    for state in ["paneflow.json", "session.json", "window-state.json"] {
        if home
            .join(state)
            .canonicalize()
            .is_ok_and(|state| state == target)
        {
            return Err("Choose a file other than Paneflow's state files".into());
        }
    }
    Ok(target)
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn profile(name: &str) -> AgentProfileConfig {
        AgentProfileConfig {
            name: name.into(),
            agent: "codex".into(),
            ..Default::default()
        }
    }

    #[test]
    fn export_cannot_overwrite_state_even_through_a_symlink() {
        let root = tempfile::tempdir().unwrap();
        let home = root.path().join("home");
        std::fs::create_dir(&home).unwrap();
        let state = home.join("paneflow.json");
        std::fs::write(&state, b"keep").unwrap();
        assert!(export_target(&state, &home).is_err());
        let outside = root.path().join("export.json");
        assert!(export_target(&outside, &home).is_ok());
        #[cfg(unix)]
        {
            std::os::unix::fs::symlink(&state, &outside).unwrap();
            assert!(export_target(&outside, &home).is_err());
            std::fs::remove_file(&outside).unwrap();
            std::fs::write(&outside, b"keep session").unwrap();
            std::os::unix::fs::symlink(&outside, home.join("session.json")).unwrap();
            assert!(export_target(&outside, &home).is_err());
            assert_eq!(std::fs::read(&outside).unwrap(), b"keep session");
        }
        assert_eq!(std::fs::read(state).unwrap(), b"keep");
    }

    #[test]
    fn import_rejects_future_versions_oversized_files_and_reports_missing_skill_references() {
        let root = tempfile::tempdir().unwrap();
        let path = root.path().join("setup.json");
        std::fs::write(
            &path,
            serde_json::to_vec(&json!({"format":FORMAT,"version":2,"profiles":[]})).unwrap(),
        )
        .unwrap();
        assert!(
            read_review(path.clone(), Value::Null)
                .err()
                .unwrap()
                .contains("version")
        );
        std::fs::write(&path, vec![b' '; MAX_BYTES as usize + 1]).unwrap();
        assert!(read_review(path.clone(), Value::Null).is_err());
        let mut entry = profile("Designer");
        entry.skills.push(root.path().join("missing.md"));
        std::fs::write(&path, encode(&[entry.clone()]).unwrap()).unwrap();
        let review = read_review(path, Value::Null).unwrap();
        assert_eq!(review.missing_skills, 1);
        assert_eq!(review.incoming, vec![entry]);
        assert_eq!(review.added, 1);
    }

    #[test]
    fn setup_round_trips_profiles_and_rejects_unknown_fields_and_duplicates() {
        let mut entry = profile("Designer");
        entry.model = Some("gpt-reserve".into());
        entry.effort = Some("high".into());
        entry.project = Some(std::env::temp_dir().join("project"));
        entry.default_profile = Some("Designer default".into());
        entry.skills = vec![std::env::temp_dir().join("design.md")];
        entry.agency_role = Some("ui-designer".into());
        entry.args = vec!["--verbose".into()];
        entry.env.insert("CUSTOM_ENV".into(), "saved-value".into());
        let setup: AgentSetup = serde_json::from_slice(&encode(&[entry.clone()]).unwrap()).unwrap();
        assert_eq!(decode_profiles(&setup.profiles).unwrap(), vec![entry]);
        assert!(decode_profiles(&json!([{"name":"D","agent":"codex","modle":"x"}])).is_err());
        assert!(decode_profiles(&json!([{"agent":"codex"}])).is_err());
        assert!(
            decode_profiles(&json!([{"name":"D","agent":"codex"},{"name":"d","agent":"codex"}]))
                .is_err()
        );
        assert!(
            serde_json::from_value::<AgentSetup>(
                json!({"format":FORMAT,"version":1,"profiles":[],"extra":true})
            )
            .is_err()
        );
    }

    #[test]
    fn merge_updates_matching_scope_and_keeps_other_projects_and_unrelated_profiles() {
        let current = json!([profile("Designer"), profile("Developer")]);
        let mut update = profile("designer");
        update.model = Some("gpt-reserve".into());
        let mut project = profile("Designer");
        project.project = Some(std::env::temp_dir().join("setup-project"));
        let (merged, added, updated) = merge(&current, &[update.clone(), project.clone()]).unwrap();
        assert_eq!((added, updated), (1, 1));
        assert_eq!(merged, vec![update, profile("Developer"), project]);
        let mut bad = profile("Invalid");
        bad.args = vec![";touch".into()];
        assert!(merge(&current, &[bad]).is_err());
        assert_eq!(current[0]["name"], "Designer");
    }
}
