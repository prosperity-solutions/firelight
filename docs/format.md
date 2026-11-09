# The `.firelight/` format, version 1

This is the normative reference. Firelight checks every rule here, and
`firelight check` reports each broken one as `path:line:col: error: message`.
JSON Schemas for every YAML file are in [`schema/v1/`](../schema/v1/) (also at
`https://firelight.oss.life.li/schema/v1/<file>.json`).

A repo describes itself in a `.firelight/` folder at its root:

```
.firelight/
  repo.yaml           required: who this repo is, how it relates to others
  about.md            optional: the long version of "what this repo is"
  links.yaml          optional: links this repo owns
  islands/*.yaml      optional: blocks on the page this repo owns or adds to
  concepts/*.md       optional: "what is X, what it is not"
  guides/*.md         optional: "how to do X"
  golden.yaml         optional: questions search must answer
  spaces.yaml         optional: spaces this repo declares
```

Any other file is ignored, with a warning. YAML files end in `.yaml`; a
`repo.yml` or `islands/*.yml` is an error that says so. Files must be regular
files: symlinks, FIFOs and devices are errors.

## General rules

- **Every YAML file, and every entry's front matter, starts with `format: 1`.**
  A file with a version this Firelight doesn't read makes it skip the whole
  repo, with an error that says so.
- **Unknown keys are errors**, so a typo never passes silently. Duplicate keys
  are errors too.
- **Ids** match `^[a-z0-9][a-z0-9-]{0,62}$`: lowercase letters, digits and
  dashes. Ids are unique per repo across concepts, guides, links and islands.
- **Refs** point at things: `argocd` (this repo) or `infra-repo/argocd` (another
  repo). A ref to something missing **in the same repo is an error**, except a
  link's `at` target: a link is never dropped, so a missing island or card puts
  it in the Unsorted card with a warning (see `links.yaml`). A ref
  into another repo that doesn't resolve is a warning (an error with
  `firelight check --strict`), because only the hub sees every repo; it renders
  as plain text, never as a dead link.
- **Only `.firelight/` is read.** Symlinks inside it are errors; they are never
  followed.
- **Limits** (defaults; the operator can change them): 1,000 files and 8 MiB
  per `.firelight/`, 256 KiB per file. YAML is parsed under a tight budget
  (depth, nodes, anchors and aliases are capped), so an exploding document is
  refused instead of expanded.
- A value that contains `: ` needs quotes: `summary: "Day one: accounts"`.

## `repo.yaml`

```yaml
format: 1
id: infra-repo                      # the repo id; must equal the source id Firelight is configured with
name: Infra
summary: Terraform and Helm for the clusters every app runs on.
owner: Platform team                # optional, free text
spaces: [platform]                  # optional: default space(s) for this repo's islands and entries
relations:                          # optional
  - repo: app-repo
    when: A new app is added there
    then: Add its Helm release in `releases/` and its DNS name in `dns/`
```

| key | | |
|---|---|---|
| `id` | required | The repo id. `firelight` is reserved for Firelight's own docs. |
| `name` | required | Display name. |
| `summary` | required | One sentence, at most 200 characters. |
| `owner` | | Free text. |
| `spaces` | | Space ids; default `default`. Not `all`, not `firelight`. |
| `relations` | | Cross-repo rules: `repo` (an id), `when`, `then`. A relation to a repo Firelight doesn't know is kept and shown as "not connected". |

`relations` is how an agent working in `app-repo` learns that a new app also
needs work in `infra-repo`. Write them honestly: only what really has to happen.

## `about.md`

Plain markdown, no front matter: the long version of the summary. Its first
section feeds search. The markdown rules below apply.

## Entries: `concepts/*.md` and `guides/*.md`

A **concept** says what something is and what it is not. A **guide** says how
to do one thing, step by step. One idea per entry; keep it short.

```markdown
---
format: 1
title: Request access rights
summary: Get time-limited admin rights for a cloud account, with a reason.
aliases: [just-in-time access, elevate, admin rights]
tags: [access, cloud]
links: [access-portal]              # links this entry is about (its "Show me" targets)
related: [access-model, app-repo/onboarding]
covers: [terraform/access/**]       # paths this entry describes
spaces: [platform, support]         # optional; overrides repo.yaml's spaces
---
1. Open [the access portal](link:access-portal).
2. Pick the role and give a reason.

## What it does not do
It never grants rights to other people. See [the access model](entry:access-model).
```

| key | | |
|---|---|---|
| `id` | | Defaults to the file name without `.md`. |
| `title` | required | Not empty. |
| `summary` | required | At most 200 characters: search results and agents show it. |
| `aliases` | | Other words people use for it. An exact alias match goes to the top of search. |
| `tags` | | Free words. |
| `links` | | Refs to links. |
| `related` | | Refs to entries. |
| `covers` | | Repo-relative paths or globs (`terraform/access/**`), no `..`, no leading `/`. When those paths change in git after the entry last did, the entry is marked *maybe stale*. |
| `spaces` | | Space ids; default: the repo's spaces. |

The front matter sits between two `---` lines at the very top of the file.

## Markdown

- CommonMark, plus tables and task lists.
- **Raw HTML is escaped, never rendered.** It is not an error; it shows as text.
- **Images are not supported** in v1 (the page fetches nothing remote). An
  image is an error; link to it instead.
- Link targets may be:
  - `link:<ref>`: renders the link's URL with a "Show me" glow.
  - `entry:<ref>` or `entry:<ref>#<heading>`: a permalink to an entry, or to one
    of its headings.
  - `https:`, `http:` or `mailto:` URLs.
  - `#<heading>`: a heading on the same page.

  Anything else (`javascript:`, `data:`, `file:`, relative paths) is an error.
- Headings get stable anchors: lowercased; letters (of any language), digits,
  `_` and `-` kept; spaces become `-`; everything else dropped
  (`What it does not do` → `what-it-does-not-do`). A heading with nothing left
  is `section`. A repeated heading gets `-2`, `-3`. An `entry:<ref>#<anchor>`
  anchor uses the same characters.
- An email autolink like `<ops@acme.example>` is a `mailto:` link.

## `links.yaml`

```yaml
format: 1
links:
  - id: argo-staging
    label: Argo CD
    url: https://argo.staging.acme.example
    note: argo.staging.acme.example   # optional: small grey text after the label
    icon: refresh                     # optional: a built-in icon (below)
    at: { island: environments, card: staging, section: tools }
    order: 20                         # optional: lower first; ties sort by label
    entry: argocd                     # optional: "What is this?", a ref to a concept
  - id: chat
    label: Team chat
    url: https://chat.acme.example
    icon: message
    at: { place: top }
```

`url` is `https:`, `http:` or `mailto:`. `at` is one target or a list of them:

| `at` | puts the link |
|---|---|
| `{ place: top }` / `{ place: footer }` | in the top or footer row of every space of the link's repo |
| `{ place: top, space: support }` | in the top or footer row of that space only |
| `{ island: <ref> }` | in a `links` island |
| `{ island: <ref>, card: <id> }` | in a card of a `cards` island |
| `{ island: <ref>, card: <id>, section: <name> }` | under one of the card's sections |

A link whose island or card doesn't exist, or isn't the right type, goes into
an **Unsorted** card at the end of the page in its repo's first space, with a
warning. It is never dropped. An unknown section puts it outside the sections,
with a warning.

## `islands/*.yaml`

An island is a block on the page. Every island has `format`, `type`, `id`
(default: the file name), `title`, `space` (default: the repo's first space,
else `default`), `place` (`main` by default, or `top`, `footer`) and `order`.
An island lives in exactly one space.

### `type: cards`

A topic made of cards. The repo that defines it owns its title, order, sections
and its own cards. Other repos may add links to its cards and add cards to it.

```yaml
format: 1
type: cards
title: Environments
order: 10
sections: [apps, tools, uses]        # optional sub-headings inside every card, in this order
cards:
  - { id: production, title: Production, order: 10, tone: danger }   # tone: danger | warn | none
  - { id: staging,    title: Staging,    order: 20 }
```

### `type: cards-contribution`

Cards added to another repo's (or this repo's) cards island:

```yaml
format: 1
type: cards-contribution
island: infra-repo/environments
cards: [{ id: demo, title: Demo, order: 30 }]
```

A contributed card whose id is already taken in that island is an error in the
contributing repo.

### `type: links`

A flat row of links. Any repo adds links to it with `at: { island: <ref> }`.
Every space also has the built-in `top` and `footer` rows (`at: { place: … }`).

### `type: template`

Inputs that turn into links, like "type a PR number, get its preview links".

```yaml
format: 1
type: template
title: PR previews
inputs:
  - { name: pr, label: "PR #", pattern: "^[0-9]{1,6}$", placeholder: "1234" }
links:
  - { label: App,  url: "https://app-{pr}.preview.acme.example", icon: phone }
  - { label: Logs, url: "https://ci.acme.example/previews/{pr}/logs", icon: file }
help: Only services the PR deployed exist.
```

- An input `name` is lowercase letters, digits and `_`, starting with a letter. `s` and `format` are reserved: the `/i/<repo>/<id>` route reads them itself.
  Its `pattern` must be anchored (`^…$`) and is matched against the whole
  value; every value must match it before anything renders.
- A URL starts with `https://` or `http://` and may use a `{name}` placeholder
  only:
  1. inside the host's **first label** (`app-{pr}.preview.acme.example`), and the
     host then has at least three labels;
  2. as **one whole path segment** (`/previews/{pr}/logs`);
  3. as **one whole query value** (`?id={pr}`).

  So a value can never change the scheme, never reach past the host's first
  label, and never turn the host into an IP address (the host may only use
  letters, digits, dots and dashes around the placeholder, and may not end in
  a number). One thing the three-label rule can't know: on a public suffix
  with two labels, like `{pr}.co.uk`, the first label *is* the registered
  domain. Don't put a placeholder there.
- Filling in: a value in the host must also be `a-z`, `0-9` and `-` only, at
  most 63 characters; a value in the path is percent-encoded as one segment and
  `.` and `..` are refused; a value in the query is percent-encoded.
- On the page the island works without JavaScript, as a form; `#<repo>/<id>:<name>=<value>`
  in the page URL pre-fills it.

### Unknown types

An island `type` this Firelight doesn't know is a **warning**: the island is
skipped and the rest of the repo works. That is how new island types arrive
without breaking older installs.

## `golden.yaml`

Questions search must answer, with the entries that must come back.

```yaml
format: 1
questions:
  - query: how do I get admin rights
    expect: [request-access]          # refs to entries or links; each must be in the top `within`
    within: 3                         # 1–25, default 5
  - query: wie bekomme ich adminrechte
    expect: [request-access]
    needs: meaning                    # skipped (and reported as skipped) when the embedder is off
```

`needs: meaning` marks questions that word search can't answer: other
languages, paraphrases. A failing question is a warning on the hub, and makes
`firelight preview --check` fail in the repo's own CI.

An expect may point into another repo (`infra-repo/request-access`): that is
how you test that a question asked about this repo finds the answer where it
really lives. Such an expect only runs where that repo is loaded too. Where it
isn't (this repo's own CI, by default), it is **skipped, not failed**, and
reported as `repo infra-repo not loaded`; a question whose expects are all in
other repos is skipped as a whole, and one with some expects here is judged on
those. Skips never fail a check, so read them: an id you don't recognise is a
typo. To run them in CI, load the other repos next to this one (`firelight
preview --check . ../infra-repo`, or the Action's `also` input; see
`firelight skills ci`).

## `spaces.yaml`

```yaml
format: 1
spaces:
  - id: platform
    title: Platform
    summary: Environments, clusters, access, deploys.   # optional
    icon: server                                        # optional
    order: 10                                           # optional
```

A **space** is a page of its own for one audience or topic.

- Space ids are global: any repo may declare one, and repos that declare the
  same id share it. Check the spaces that exist (MCP `list_spaces`) before
  inventing a new one.
- When two repos declare the same space differently, the first in the hub's
  source order wins, with a warning for the others.
- A space that is used but declared nowhere still exists, titled from its id,
  with a warning.
- Reserved ids: `all` (generated: everything on one page, and the only search
  across spaces), `default` (content that names no space), `firelight`
  (Firelight's own docs). Declaring any of them is an error; using `all` or
  `firelight` is an error.
- Membership: an island belongs to its `space`; an entry to its own `spaces`,
  else its repo's, else `default`; a link to the spaces of the islands it sits in.

## Built-in icons

`activity`, `alert`, `archive`, `bell`, `book`, `box`, `bug`, `calendar`,
`chart`, `check`, `clock`, `cloud`, `code`, `compass`, `cpu`, `database`,
`dollar`, `file`, `flag`, `folder`, `git`, `globe`, `grid`, `heart`, `help`,
`home`, `key`, `layers`, `link`, `list`, `lock`, `mail`, `map`, `message`,
`monitor`, `package`, `phone`, `refresh`, `rocket`, `search`, `server`,
`settings`, `shield`, `star`, `terminal`, `ticket`, `tool`, `users`, `video`,
`zap`.

## Checking

```sh
firelight check                 # this repo's .firelight/
firelight check ../infra-repo . # several together: refs between them resolve
firelight check --strict        # unresolved cross-repo refs are errors
firelight check --json          # for tools
```

Exit code 0 when there are no errors (warnings are fine), 1 otherwise.
