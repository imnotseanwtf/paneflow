# Agency Agents role sources

Source: https://github.com/msitarzewski/agency-agents

Pinned revision: `8ae6164602c95ca78828b148b9052460ee426885`.

These files are unmodified upstream instructions under the accompanying MIT license.
Paneflow adds its runtime instructions when installing them for Claude Code.

| Paneflow role | Upstream file |
| --- | --- |
| `orchestrator` | `specialized/agents-orchestrator.md` |
| `planner` | `project-management/project-manager-senior.md` |
| `ui-designer` | `design/design-ui-designer.md` |
| `ui-developer` | `engineering/engineering-frontend-developer.md` |
| `core-developer` | `engineering/engineering-backend-architect.md` |
| `qa-tester` | `testing/testing-evidence-collector.md` |
| `code-reviewer` | `engineering/engineering-code-reviewer.md` |
| `devops` | `engineering/engineering-devops-automator.md` |

The full pinned library contains 283 definitions across the divisions in the
upstream `divisions.json`. The eight entries above keep their short IDs for
compatibility; additional unmodified source files are under `catalog/`.
[`catalog.json`](catalog.json) maps each selectable ID to its upstream path,
display name and division. Only selected profiles' runtime prompts are prepared
in the Paneflow home's cache. No network download is needed to choose a role.
