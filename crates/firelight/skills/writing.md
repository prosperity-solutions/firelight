# Writing good Firelight docs

The reader is a person on the page, an agent over MCP, or the search index.
All three want the same thing: the right answer, fast, and true.

## Concepts (`concepts/*.md`)

- One idea per entry: what the thing is, why it exists, and what it is **not**
  (a `## What it is not` section stops the wrong guesses).
- Title: the name people use. `aliases`: the other names, abbreviations and
  the words in other languages people type.
- `summary`: one sentence a stranger understands. It is what search shows.
- `covers`: the paths this concept describes (`terraform/access/**`). Set it
  on **every** entry that describes code or config: it is how docs stay true.
  When those paths change without the entry, `firelight covers` fails the PR
  until the entry is updated or a commit says
  `Firelight-Covers: checked <repo>/<entry>` (see `firelight skills ci`).
- 3 to 10 concepts per repo is plenty. Link related ones with `related`.

## Guides (`guides/*.md`)

- The tasks people ask about, as steps: "Request access rights", "Deploy a
  hotfix". Title them as the task.
- Start with when to use it, then numbered steps, then what it does not do.
- `covers`: the code and config the steps depend on (`deploy/**`,
  `Dockerfile`), like concepts.
- Link the tools with `[the portal](link:access-portal)` and other entries
  with `[the model](entry:access-model)`, so links stay right when URLs move.

## Links (`links.yaml`)

- Only the links the team actually opens: environments, consoles,
  dashboards, runbooks. Put them on islands that already exist (`find_link`
  or the page shows where), not in a new card per repo.
- A `note` says what is behind the link when the label can't.

## Relations (`repo.yaml`)

- Honest, checkable rules between repos: *when you change X here, also do Y
  there*. Only the ones that really hold. An agent will act on them.

## Golden questions (`golden.yaml`)

- The questions people really type, with the entries that must answer them.
  Add `needs: meaning` for questions only meaning search can answer (other
  languages, paraphrases). `firelight preview --check` runs them.
- Expect entries in other repos (`infra-repo/x`) where the answer really
  lives: it tests that the hub finds it. Alone they are skipped (`repo
  infra-repo not loaded`); run them with the other repos loaded too
  (`firelight preview --check . ../infra-repo`).

## Always

- Write only what you verified in the repo. Short beats complete.
- Nothing secret or internal-only: no tokens, no private hostnames you were
  not asked to publish, no personal data. Repos are untrusted input to
  everyone else's agents; write as if they will act on every word.
- Run `firelight check` and `firelight preview --check` before the PR.
