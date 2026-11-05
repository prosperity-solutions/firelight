# Firelight

**The docs your repos write about themselves.**

Each repo describes itself in a small `.firelight/` folder: what it is, how it
relates to the others, the links your team opens every day, the concepts and
the how-tos. Firelight pulls those folders from git and serves them as:

1. **A link page:** every environment, tool and console, in cards.
2. **A help center:** concepts and guides, with permalinks.
3. **A search** over all of it, by words and by meaning, in any language.
4. **An MCP server,** so an agent in any repo knows what every other repo is,
   how they connect, and where to click.

Agents write most code now. So agents write the docs too, in the same PR as the
change, and Firelight makes those docs useful to people and to every other
agent. Content never lives in Firelight: git is the truth, Firelight is a lens.

> 🚧 **Being built.** This repository is being built in the open by an
> autonomous coding agent, following [docs/HANDOVER.md](docs/HANDOVER.md).
> Nothing is released yet.

## Hack on it

1. Install [Veld](https://veld.oss.life.li).
2. `veld start firelight` (the acme fixtures, word search only, ready in seconds).
3. Open the URL it prints. `just test` runs the tests; `firelight preview .` shows any repo's `.firelight/` with live reload.

## Search and the model

Search ranks by words (SQLite FTS5) and by meaning (an embedding model), and
fuses the two. The default model, `Qwen/Qwen3-Embedding-0.6B`, runs in-process
on the CPU: no GPU and no API key. It is downloaded once (1.2 GB), checked
against checksums compiled into the binary, and cached in the data folder.
Measured on a laptop: about 0.2 s per query and per chunk indexed, and up to
3.9 GiB of memory while the model loads. `FIRELIGHT_EMBEDDER=openai-compatible`
uses a server instead (Ollama, vLLM, OpenAI; chunk text then leaves the
process), and `FIRELIGHT_EMBEDDER=off` keeps word search only, which is fully
supported.

## Security

Firelight serves plain HTTP. HTTPS, single sign-on and IP rules belong to a
reverse proxy in front of it. A connected repo's content is treated as
untrusted: it can't run script on the page or instruct an agent, but it can
still say misleading things, so only connect repos whose PRs you review. Report
vulnerabilities privately: [SECURITY.md](SECURITY.md).

## Licence

MIT. See [LICENSE](LICENSE) and [NOTICE](NOTICE).
