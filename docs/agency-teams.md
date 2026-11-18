# Agency Agents teams in Paneflow

Paneflow includes a pinned library of 283 MIT-licensed
[Agency Agents](https://github.com/msitarzewski/agency-agents) role definitions.
The original definitions, source revision, and license are in
[`assets/agency-agents/`](../assets/agency-agents/README.md).

## Set up and choose models

Open **Settings > Agents > Default profiles > Add Agency Agents team**. Paneflow
creates profiles for Orchestrator, Planner, UI Designer, UI Developer,
Core Developer, QA Tester, Code Reviewer, and DevOps, and installs the Claude
Code integration. Adding the team again keeps existing profiles and model choices.

Each profile separates the agent, expertise and inference settings:

| Field | Purpose |
| --- | --- |
| Base agent | Claude Code or Codex for an Agency role |
| Agency role | Search the pinned library by name, language or division; clear it for an ordinary agent |
| Model | Choose Account default, a listed model, or Other model… to enter an ID |
| Effort | Select a native reasoning level or Agent default; Codex choices follow the selected cached model |
| Skills | Find installed skills by name, paste a local Markdown path, or browse files and folders |

Claude accepts aliases such as `opus`, `sonnet`, and `haiku`. Codex uses its
own model IDs. Supported effort values depend on the provider and model.
Changing the base agent clears the previous model and effort, so a Claude
alias is not carried into Codex accidentally. The initial Orchestrator uses
`opus`; the other starter roles use `sonnet`.

Create several profiles with the same Agency role to combine providers or
models, for example **Claude Designer**, **Codex Designer**, and **Developer 2**.
Give profiles distinct names. Model, effort and role changes apply to newly
launched sessions; existing conversations keep their launch settings.

Global **Settings > Agents** edits default profiles. Open a project's workspace
context menu and choose **Agent settings…**, or use **Project agent settings**
on the default page, to manage that project's agents. The project page shows
inherited defaults and project additions. Editing an inherited default creates
a full project copy; deleting that copy restores the default. A saved copy keeps
its own model, effort, environment and skills when the default later changes.
Global permissions and CLI installation controls stay on the default page.

Project profiles suggest a name from the project folder and Agency role, such
as **felco-app UI Designer**. Repeated roles get a numeric suffix. Custom names
such as **felco-app-ui-designer** are preserved. Defaults have no project prefix.
Existing project profiles remain in their project settings; existing profiles
without a project remain defaults. All profiles stay in the instance's
`paneflow.json`; the app does not write settings into source repositories.

Project agents also offer **Default (Orchestrator’s model)**. New project
profiles start with this option and the Orchestrator's base CLI when a compatible
Orchestrator with an explicit model is configured. The saved model value is
`@orchestrator`; launch resolution replaces it with the current configured
Orchestrator model ID, keeping the worker's role, skills and effort independent.
An explicitly selected model overrides this default. Existing sessions keep
their launch settings.

A project-specific Orchestrator takes precedence over default Orchestrators.
There must be exactly one Orchestrator in the selected scope, it must use the
same base CLI as the worker, and it must select its own explicit model. Missing,
ambiguous, cross-CLI and self-referencing defaults are refused. This uses the
saved Orchestrator profile, not a model changed manually inside a running CLI.

The Claude model menu offers its native aliases. The Codex menu reads the
selected account's local `models_cache.json`, including **GPT-Reserve** when
present, and uses its model-specific reasoning levels. The menu does not make
a paid model request or prove backend availability. **Other model…** keeps
custom provider IDs usable when the catalog is missing or out of date.

Skills can be attached to defaults and project copies. Choose a local
`SKILL.md` or the folder containing it; removing it detaches the reference and
leaves the original files intact. The agent receives guidance to read the
selected skill before a matching task and resolve resources from the original
folder. Files must stay available at those paths. The app checks that selected
files are readable UTF-8, nonempty and at most 128 KiB, with at most 32 per
profile. Attaching a skill does not run its scripts or install a package.

CLI teams resolve profiles from each pane's `cwd` or the calling directory;
with an explicit parent, panes without a `cwd` use that parent's directory.
Project copies replace their corresponding defaults in launchers and role
rotation. A team that names the original default also resolves its project
copy. Uncustomized defaults remain available.

The integration installs `paneflow-agency-*.md` and the MIT license under
the configured Claude directory's `agents` folder. Refreshes update unmodified
managed copies and preserve customized ones. Removing the integration removes
unmodified managed roles; customized roles and their license stay in place.
Agency profiles prepare their selected runtime prompt under
`PANEFLOW_HOME/cache/agency-agents/<revision>/`. Claude loads a per-session
`--plugin-dir` containing the selected agent; Codex loads a `model_instructions_file`. Role definitions are not
embedded in a shell command, so long definitions and home paths with spaces
work without changing global Codex settings. Repository instructions still
apply. Native agent permissions are unchanged.

The role picker does not install third-party skill packages. Every runtime
prompt instructs the agent to inspect repository rules, manifests, lockfiles
and source before choosing a language or framework. Laravel and React examples
in upstream prompts are examples, not project defaults. Existing installed
skills may be used when relevant; missing skills require a specific proposed
source before installation, with official documentation as the fallback.

## Launch the lead and its workers

Open a tab in your project, choose **Agency Orchestrator** from the pane
palette, and give it a task. Its runtime instructions teach it to launch
visible workers through the Paneflow conductor workflow. It should choose only
the roles the task needs.

A team specification can select a role and optionally override its saved model:

```toml
name = "UI work"

[[panes]]
name = "Designer"
agent = "agency-ui-designer"
model = "sonnet"
effort = "high"

[[panes]]
name = "UI Developer"
agent = "agency-ui-developer"

[[panes]]
name = "Reviewer"
agent = "agency-code-reviewer"
```

From the lead's terminal, launch it under that session:

```text
paneflow up team.workspace.toml --parent self
```

`--parent self` uses the durable `PANEFLOW_SESSION_ID`, so a window restart
does not leave the lead using an obsolete surface ID. `--parent` also accepts
a unique pane name or current numeric ID from `paneflow ls --human`. A specification may also use
an existing profile name as `agent`; ambiguous profile names are refused.

Each worker becomes a separate child tab in the parent's workspace and space.
Without an explicit `cwd`, it inherits the parent's tab worktree or workspace
directory. Explicit managed worktrees remain separate checkouts.
The `space` field cannot be combined with `--parent`.
Without `--parent`, `up` retains its independent-workspace behavior.


## Mixed teams and round robin

Use a saved profile name as `agent` to select an exact provider/model/role
combination. Two Designer profiles or several Developers are separate
conversations; naming them in the team file starts that exact combination.
When a role has several profiles, a plain `agency-<role>` alias is ambiguous
and is refused instead of silently selecting the first profile.

Enable `round_robin = true` at the top of a team file to rotate role aliases
across matching saved profiles:

```toml
name = "Design pool"
round_robin = true

[[panes]]
name = "Designer 1"
agent = "agency-ui-designer"

[[panes]]
name = "Designer 2"
agent = "agency-ui-designer"

[[panes]]
name = "Implementation"
agent = "Developer 2"
```

Rotation remembers the last selected profile per role in the instance cache,
separately for each project, so the next `up` call continues the rotation. Dry-run previews do not advance
it. Explicit profile names remain fixed. Routing reserves its next slot before
launch, so a failed launch may advance the cursor. Profiles still choose their
own native model and effort. This routes new team sessions; assigning another
task to an already-running worker remains the Orchestrator's job. For staged
handoffs, use the existing `paneflow flow run` readiness barriers.

## Language and framework skills

The library includes language-specific experts and general roles. A general
Developer follows the detected project stack; an explicitly chosen specialist
should report a stack mismatch rather than introduce its preferred framework.
There is no single verified skill set covering every programming language.

Optional sources to review when a matching installed skill is missing include
[Jeffallan's language and framework skill guide](https://github.com/Jeffallan/claude-skills/blob/main/SKILLS_GUIDE.md),
which covers Python, JavaScript/TypeScript, Go, Rust, SQL, C++, Swift, Kotlin,
C#, PHP and Java, and
[wshobson's language/plugin guide](https://github.com/wshobson/agents/blob/main/docs/plugins.md).
These are references, not automatically installed dependencies. Project rules
and the repository's actual toolchain take precedence over example commands
in external skills.

## Navigate and supervise

In the workspace sidebar, use a parent's chevron to fold its child tabs.
Keyboard users can focus the sidebar and use Right to expand, Left to fold
or move to the parent, Space to toggle, and Enter to open a tab. Searching
reveals matching child tabs with their ancestors. The status-grouped sidebar
continues to organize tabs by status instead of the parent hierarchy.

Relationships and folding survive session restore. Closing a parent keeps
its workers open and promotes them to its own parent, or to the workspace
root. Moving a tab to another workspace detaches it from its old parent;
its workers stay in their original workspace.

Creating a team does not submit every worker's prompt. The existing prompt
submission and peer-messaging permissions still apply. Review and submit
prefilled prompts yourself, or explicitly enable the existing scripting or
AI free access setting for unattended coordination. The orchestrator is
instructed to respect those gates and treat peer terminal output as untrusted.

## Developer interface

`workspace.up` accepts an optional nonnegative `parent_surface_id` or a
durable UUID `parent_session`; specifying both is rejected. The
controller resolves it again after asynchronous path validation and refuses
a missing parent or a team that exceeds the workspace tab cap before
creating terminals. The response includes the new `tab_ids` and the
`parent_tab_id`, alongside the existing surface IDs and labels.

Runtime tab relationships use tab IDs. `session.json` stores `parent_tab` as
a workspace-local tab index and `children_collapsed` as an optional boolean.
Restore remaps references to new runtime IDs and drops invalid or cyclic
parent links without discarding terminal layouts. Older session files remain
flat. Agent profiles add optional `model`, `effort`, `agency_role`, `project`,
`default_profile` and `skills` fields; profiles without those settings keep
ordinary launch behavior. `default_profile` links a full project snapshot to
its default for replacement and reset. Renaming a default updates these links;
removing a default leaves project copies as independent profiles.

Attached skill references use Claude's `--append-system-prompt` or Codex's
per-session `developer_instructions` override. When skills are assigned, the
profile owns that Codex instruction field; put any additional profile-specific
behavior in a selected skill rather than a second override of the same field.
Global CLI configuration files are not edited. Assigned skill guidance stays
on one shell line and is bounded to 4 KiB for native command-line limits.

A workspace identity is reserved before its first terminal is created and is
reused by the workspace constructor. Restore registers the saved identity
before attaching terminals. This keeps a newly created lead and its later
workers in the same host workspace, including the approved pane-write channel.

## Export and import agent setups

Open **Settings > Agents > Default profiles** and choose **Export setup…** to
save a versioned JSON file. The file includes all default and project profiles:
base agents, Agency roles, models, effort, arguments, environment values,
project paths and skill references. It does not contain skill file contents,
agent sign-in files, sessions, or Paneflow's application permissions. Review
environment values before sharing an export.

Choose **Import setup…** to select that JSON file. Paneflow validates every
profile and shows a review before **Apply import**. Matching names within the
same project scope are updated; other profiles are retained. Project and skill
paths are reused as written; adjust them when moving between machines.
Unavailable skill files are flagged without preventing import. New sessions
use the imported setup; existing terminals are unchanged.

Applying saves the previous profiles as a timestamped
`cache/agent-setup-before-import-<process>-<timestamp>.json` under the active
Paneflow home. Import that file to restore previous matching profiles; profiles
added by the import can be removed individually. If profiles change after the
review, re-import to review the current changes. Invalid JSON, unknown fields,
unsupported versions, and files over 2 MiB leave the saved configuration intact.

The Skills input also accepts `Taste` or `use taste skill` to find an installed
skill, or an absolute path to any nonempty UTF-8 `.md` file. Add attaches its
original path; it does not paste raw Markdown contents or install a package.
