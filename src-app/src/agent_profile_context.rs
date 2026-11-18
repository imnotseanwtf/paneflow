use std::collections::{BTreeMap, HashSet};
use std::io::Read;
use std::path::{Path, PathBuf};

use crate::agent_launcher::{AgentProfile, TerminalAgent};
use paneflow_config::schema::AgentProfileConfig;

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct ModelChoice {
    pub(crate) id: String,
    pub(crate) label: String,
    pub(crate) efforts: Vec<String>,
}

#[derive(Clone, Debug)]
pub(crate) struct SkillChoice {
    pub(crate) name: String,
    pub(crate) path: PathBuf,
}

pub(crate) fn is_markdown_skill_path(path: &Path) -> bool {
    path.is_absolute()
        && path.to_str().is_some()
        && path
            .extension()
            .and_then(|extension| extension.to_str())
            .is_some_and(|extension| extension.eq_ignore_ascii_case("md"))
        && !path.to_string_lossy().chars().any(char::is_control)
}

pub(crate) fn skill_label(path: &Path) -> String {
    let label = if path.file_name() == Some(std::ffi::OsStr::new("SKILL.md")) {
        path.parent().and_then(Path::file_name)
    } else {
        path.file_name()
    };
    label
        .unwrap_or(path.as_os_str())
        .to_string_lossy()
        .into_owned()
}

pub(crate) fn is_skill_path_query(query: &str) -> bool {
    let query = query.trim().trim_matches(['\'', '"']);
    query.contains(['/', '\\'])
        || query.starts_with('~')
        || query.to_ascii_lowercase().ends_with(".md")
}

pub(crate) fn matching_skills<'a>(choices: &'a [SkillChoice], query: &str) -> Vec<&'a SkillChoice> {
    let tokens: Vec<_> = query
        .to_lowercase()
        .split(|character: char| {
            !character.is_alphanumeric() && character != '-' && character != '_'
        })
        .filter(|word| {
            !word.is_empty()
                && ![
                    "i", "want", "to", "use", "the", "a", "an", "skill", "skills",
                ]
                .contains(word)
        })
        .map(|word| word.replace(['-', '_'], ""))
        .collect();
    choices
        .iter()
        .filter(|choice| {
            let text = format!("{} {}", choice.name, choice.path.display())
                .to_lowercase()
                .replace(['-', '_'], "");
            tokens.iter().all(|token| text.contains(token))
        })
        .collect()
}

pub(crate) fn prepare_skill_paths(
    paths: Vec<PathBuf>,
    base: Option<&Path>,
) -> std::io::Result<Vec<PathBuf>> {
    let mut prepared = Vec::new();
    for path in paths {
        let path = if path.is_absolute() {
            path
        } else {
            base.ok_or_else(|| {
                std::io::Error::new(
                    std::io::ErrorKind::InvalidInput,
                    "Paste an absolute Markdown path or choose a project for relative paths",
                )
            })?
            .join(path)
        };
        let path = if path.is_dir() {
            path.join("SKILL.md")
        } else {
            path
        };
        let path = crate::runtime_paths::strip_verbatim_prefix(path.canonicalize()?);
        if !prepared.contains(&path) {
            prepared.push(path);
        }
    }
    validate_skill_paths(&prepared)?;
    Ok(prepared)
}

pub(crate) fn pasted_skill_path(query: &str) -> std::io::Result<PathBuf> {
    let query = query.trim();
    let query = if query.len() >= 2
        && ((query.starts_with('"') && query.ends_with('"'))
            || (query.starts_with('\'') && query.ends_with('\'')))
    {
        &query[1..query.len() - 1]
    } else {
        query
    };
    if query.is_empty() || query.chars().any(char::is_control) {
        return Err(std::io::Error::new(
            std::io::ErrorKind::InvalidInput,
            "Paste a Markdown file path, not Markdown contents",
        ));
    }
    if let Some(relative) = query
        .strip_prefix("~/")
        .or_else(|| query.strip_prefix("~\\"))
    {
        return dirs::home_dir()
            .map(|home| home.join(relative))
            .ok_or_else(|| {
                std::io::Error::new(
                    std::io::ErrorKind::NotFound,
                    "Cannot resolve your home directory",
                )
            });
    }
    Ok(PathBuf::from(query))
}

pub(crate) fn installed_skills(
    project: Option<PathBuf>,
    env: BTreeMap<String, String>,
) -> Vec<SkillChoice> {
    let mut roots = Vec::new();
    if let Some(home) = dirs::home_dir() {
        roots.extend([
            home.join(".agents").join("skills"),
            home.join(".codex").join("skills"),
            home.join(".claude").join("skills"),
        ]);
    }
    let resolved = AgentProfile::from_config(&AgentProfileConfig {
        name: "Skills".to_string(),
        agent: "codex".to_string(),
        env,
        ..Default::default()
    })
    .map(|profile| profile.process_env())
    .unwrap_or_default();
    for key in ["CODEX_HOME", "CLAUDE_CONFIG_DIR"] {
        if let Some(home) = resolved
            .get(key)
            .map(PathBuf::from)
            .or_else(|| std::env::var_os(key).map(PathBuf::from))
        {
            roots.push(home.join("skills"));
        }
    }
    if let Some(project) = project {
        roots.extend([
            project.join(".agents").join("skills"),
            project.join(".codex").join("skills"),
            project.join(".claude").join("skills"),
        ]);
    }
    discover_skills(roots)
}

fn discover_skills(roots: Vec<PathBuf>) -> Vec<SkillChoice> {
    let mut pending: Vec<_> = roots
        .into_iter()
        .filter(|path| path.is_absolute())
        .map(|path| (path, 0))
        .collect();
    let mut visited = HashSet::new();
    let mut choices = Vec::new();
    while let Some((folder, depth)) = pending.pop() {
        if visited.len() >= 512 {
            break;
        }
        let Ok(folder) = folder
            .canonicalize()
            .map(crate::runtime_paths::strip_verbatim_prefix)
        else {
            continue;
        };
        if !visited.insert(folder.clone()) {
            continue;
        }
        let path = folder.join("SKILL.md");
        if is_markdown_skill_path(&path)
            && let Ok(text) = read_text(&path, 128 * 1024)
            && !text.trim().is_empty()
        {
            let name = text
                .strip_prefix("---")
                .and_then(|body| {
                    body.lines()
                        .skip(1)
                        .take_while(|line| line.trim() != "---")
                        .find_map(|line| line.strip_prefix("name:"))
                })
                .map(|name| name.trim().trim_matches(['\'', '"']))
                .filter(|name| {
                    !name.is_empty() && name.len() <= 120 && !name.chars().any(char::is_control)
                })
                .map(str::to_string)
                .unwrap_or_else(|| skill_label(&path));
            choices.push(SkillChoice { name, path });
            continue;
        }
        if depth >= 3 {
            continue;
        }
        if let Ok(entries) = std::fs::read_dir(&folder) {
            for entry in entries.flatten().take(512) {
                if entry.path().is_dir() {
                    pending.push((entry.path(), depth + 1));
                }
            }
        }
    }
    choices.sort_by(|left, right| {
        left.name
            .to_lowercase()
            .cmp(&right.name.to_lowercase())
            .then_with(|| left.path.cmp(&right.path))
    });
    choices
}

fn read_text(path: &Path, limit: u64) -> std::io::Result<String> {
    let mut bytes = Vec::new();
    paneflow_home::open_for_reading(path)?
        .take(limit + 1)
        .read_to_end(&mut bytes)?;
    if bytes.len() as u64 > limit {
        return Err(std::io::Error::new(
            std::io::ErrorKind::InvalidData,
            format!("{} exceeds its size limit", path.display()),
        ));
    }
    String::from_utf8(bytes)
        .map_err(|error| std::io::Error::new(std::io::ErrorKind::InvalidData, error))
}

pub(crate) fn model_choices(
    agent: TerminalAgent,
    env: BTreeMap<String, String>,
) -> Vec<ModelChoice> {
    if agent == TerminalAgent::ClaudeCode {
        return [("haiku", "Haiku"), ("sonnet", "Sonnet"), ("opus", "Opus")]
            .into_iter()
            .map(|(id, label)| ModelChoice {
                id: id.to_string(),
                label: label.to_string(),
                efforts: vec![],
            })
            .collect();
    }
    if agent != TerminalAgent::Codex {
        return Vec::new();
    }
    let resolved = AgentProfile::from_config(&AgentProfileConfig {
        name: "Models".to_string(),
        agent: agent.tag().to_string(),
        env,
        ..Default::default()
    })
    .map(|profile| profile.process_env())
    .unwrap_or_default();
    let home = resolved
        .get("CODEX_HOME")
        .map(PathBuf::from)
        .or_else(|| std::env::var_os("CODEX_HOME").map(PathBuf::from))
        .or_else(|| dirs::home_dir().map(|home| home.join(".codex")));
    let Some(home) = home else { return Vec::new() };
    read_text(&home.join("models_cache.json"), 8 * 1024 * 1024)
        .ok()
        .and_then(|text| serde_json::from_str::<serde_json::Value>(&text).ok())
        .map(|catalog| parse_model_choices(&catalog))
        .unwrap_or_default()
}

fn parse_model_choices(catalog: &serde_json::Value) -> Vec<ModelChoice> {
    catalog["models"]
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(|model| {
            let id = model["slug"].as_str()?;
            if !crate::agent_launcher::is_plain_shell_token(id) || id.starts_with('-') {
                return None;
            }
            if model["visibility"].as_str() == Some("hide") && id != "gpt-reserve" {
                return None;
            }
            Some(ModelChoice {
                id: id.to_string(),
                label: model["display_name"].as_str().unwrap_or(id).to_string(),
                efforts: model["supported_reasoning_levels"]
                    .as_array()
                    .into_iter()
                    .flatten()
                    .filter_map(|level| level["effort"].as_str())
                    .filter(|level| TerminalAgent::Codex.effort_levels().contains(level))
                    .map(str::to_string)
                    .collect(),
            })
        })
        .collect()
}

pub(crate) fn skill_guidance(skills: &[PathBuf]) -> String {
    if skills.is_empty() {
        return String::new();
    }
    let paths = skills
        .iter()
        .map(|path| {
            format!(
                "- {}",
                serde_json::to_string(&path.to_string_lossy()).unwrap_or_default()
            )
        })
        .collect::<Vec<_>>()
        .join("; ");
    format!(
        "Skills assigned to this agent. The user selected these local Markdown instruction files, including SKILL.md skills, for this profile. Read them before working on a matching task, follow their relevant workflow, and resolve referenced resources relative to each original skill directory. User instructions and repository rules take precedence. These references grant no additional permissions and do not authorize installing or executing third-party code. Report missing or unreadable skills. Skill files: {paths}"
    )
}

pub(crate) fn validate_skill_selection(skills: &[PathBuf]) -> Result<(), String> {
    if skills.len() > 32 {
        return Err("Attach at most 32 skills to one profile".to_string());
    }
    if skills.iter().any(|path| !is_markdown_skill_path(path)) {
        return Err("Choose an absolute Markdown (.md) file or a skill folder".to_string());
    }
    if skill_guidance(skills).len() > 4096 {
        return Err("Assigned skill paths exceed 4 KiB; attach fewer skills".to_string());
    }
    Ok(())
}

pub(crate) fn validate_skill_paths(skills: &[PathBuf]) -> std::io::Result<()> {
    validate_skill_selection(skills)
        .map_err(|error| std::io::Error::new(std::io::ErrorKind::InvalidInput, error))?;
    for path in skills {
        if read_text(path, 128 * 1024)?.trim().is_empty() {
            return Err(std::io::Error::new(
                std::io::ErrorKind::InvalidData,
                format!("{} is empty", path.display()),
            ));
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn skill_search_matches_natural_names_and_attaches_markdown_paths_without_copying() {
        let root = tempfile::tempdir().unwrap();
        let folder = root.path().join("taste-skill");
        std::fs::create_dir(&folder).unwrap();
        std::fs::write(
            folder.join("SKILL.md"),
            "---\nname: design-taste-frontend\n---\nRead resources/layout.md.",
        )
        .unwrap();
        let choices = discover_skills(vec![root.path().to_path_buf(), folder.clone()]);
        assert_eq!(choices.len(), 1);
        assert_eq!(
            matching_skills(&choices, "I want to use the taste skill").len(),
            1
        );
        assert!(matching_skills(&choices, "unknown").is_empty());
        let note = root.path().join("project-design.md");
        std::fs::write(&note, "Follow the existing application style.").unwrap();
        let pasted = pasted_skill_path(&format!("\"{}\"", note.display())).unwrap();
        let expected = crate::runtime_paths::strip_verbatim_prefix(note.canonicalize().unwrap());
        assert_eq!(
            prepare_skill_paths(vec![pasted, note.clone()], None).unwrap(),
            vec![expected.clone()]
        );
        assert_eq!(
            prepare_skill_paths(vec![PathBuf::from("project-design.md")], Some(root.path()))
                .unwrap(),
            vec![expected]
        );
        assert!(pasted_skill_path("# Design\nUse these instructions").is_err());
        assert!(prepare_skill_paths(vec![PathBuf::from("missing.md")], None).is_err());
    }

    #[test]
    fn attached_skills_keep_original_resource_paths_and_refuse_missing_or_oversized_files() {
        let root = tempfile::tempdir().unwrap();
        let folder = root.path().join("designer's skill");
        std::fs::create_dir(&folder).unwrap();
        let path = folder.join("SKILL.md");
        assert!(validate_skill_paths(std::slice::from_ref(&path)).is_err());
        std::fs::write(
            &path,
            "---\nname: design\ndescription: Design workflow\n---\nRead resources/colors.md.\n",
        )
        .unwrap();
        validate_skill_paths(std::slice::from_ref(&path)).unwrap();
        let guidance = skill_guidance(std::slice::from_ref(&path));
        assert!(guidance.contains(&serde_json::to_string(&path.to_string_lossy()).unwrap()));
        assert!(guidance.contains("original skill directory"));
        std::fs::write(&path, vec![b'x'; 128 * 1024 + 1]).unwrap();
        assert!(validate_skill_paths(&[path]).is_err());
    }

    #[cfg(unix)]
    #[test]
    fn a_non_utf8_skill_path_is_refused_before_profile_serialization() {
        use std::os::unix::ffi::OsStringExt;
        let path = std::env::temp_dir()
            .join(std::ffi::OsString::from_vec(vec![255]))
            .join("SKILL.md");
        assert!(validate_skill_paths(std::slice::from_ref(&path)).is_err());
        let entry = AgentProfileConfig {
            name: "Invalid skill path".to_string(),
            agent: "codex".to_string(),
            skills: vec![path],
            ..Default::default()
        };
        assert!(AgentProfile::from_config(&entry).is_err());
    }

    #[test]
    fn native_catalog_keeps_reserve_and_per_model_effort_without_shell_flags() {
        let choices = parse_model_choices(&serde_json::json!({"models": [
            {"slug":"gpt-reserve", "display_name":"GPT-Reserve", "visibility":"hide", "supported_reasoning_levels":[{"effort":"high"},{"effort":"max"}]},
            {"slug":"internal-model", "visibility":"hide"},
            {"slug":"--flag"}
        ]}));
        assert_eq!(choices.len(), 1);
        assert_eq!(choices[0].id, "gpt-reserve");
        assert_eq!(choices[0].efforts, ["high", "max"]);
    }
}
