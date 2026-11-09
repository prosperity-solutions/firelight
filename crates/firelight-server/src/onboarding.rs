//! The onboarding prompt (§12.4): what a coding agent does to add a repo to
//! Firelight. Shown on the page ("Add a repo"), served as the MCP prompt
//! `onboard_repo` and the resource `firelight://onboarding`, and on the website.

pub const PROMPT: &str = "Set this repository up for Firelight, the docs your repos write about themselves.

1. Install the Firelight skill: `npx skills add prosperity-solutions/firelight` (or `firelight skills install --project`), then read it: `firelight skills format` and `firelight skills writing`.
2. Read this repository: what it is, what it is for, how it relates to the other repos it talks to, the links the team opens every day, and the questions people ask about it.
3. If the Firelight MCP server is connected, call `list_spaces` and `list_repos` first. Contribute to the spaces that already exist before declaring a new one, and use the other repos' ids in refs and relations.
4. Write `.firelight/`:
   - `repo.yaml`: id, name, a one-sentence summary, the owner, its spaces, and honest `relations` (\"when you do X here, also do Y there\"), only the ones that really hold.
   - `about.md`: the longer version of the summary.
   - `links.yaml`: the links the team actually uses (environments, consoles, dashboards), placed on existing islands where they fit.
   - 3 to 10 `concepts/`: what each important thing is, and what it is not.
   - `guides/` for the tasks people ask about, step by step.
   - `golden.yaml`: questions people would type, with the entries that must answer them.
   Keep entries short, one idea each. Write only what you verified in this repository. Leave out anything secret or internal-only: no tokens, no internal hostnames you were not asked to publish, no personal data.
5. Run `firelight check` and `firelight preview --check` until both pass.
6. Wire this repo's agents to Firelight. Add to AGENTS.md (and CLAUDE.md, if the repo has one) under a \"Firelight\" heading:
   \"The Firelight MCP server is the team's shared knowledge of all our repos; it must be connected. Search it first (`search`) for any discovery question: environments, access, deploys, incidents, links, how-tos, cross-repo rules. Keep `.firelight/` true in the same PR as the change; run `firelight check` and `firelight preview --check`. When a Firelight answer is stale or missing, fix it in the owning repo's `.firelight/`, in any repo.\"
   Commit the skill (`firelight skills install --project`), and say in the README how to connect the MCP, e.g. for Claude Code: `claude mcp add --transport http firelight https://firelight.acme.example/mcp --header \"Authorization: Bearer <token>\"`.
7. Add the Firelight GitHub Action to CI (see `firelight skills ci`).
8. Open a pull request with all of it.";
