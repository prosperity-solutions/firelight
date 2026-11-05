//! `firelight skills [TOPIC]` and `firelight skills install [--project]`
//! (§12.4): one installable skill whose topics live in the binary, so they
//! always match the installed version.

use std::path::{Path, PathBuf};
use std::process::ExitCode;

/// The pointer skill, as `npx skills add prosperity-solutions/firelight`
/// installs it.
pub const SKILL: &str = include_str!("../../../skills/firelight/SKILL.md");

/// (name, one line, text).
pub fn topics() -> Vec<(&'static str, &'static str, &'static str)> {
    vec![
        (
            "format",
            "the .firelight/ format: every file and field",
            include_str!("../../../docs/format.md"),
        ),
        (
            "writing",
            "what good concepts, guides, links and relations look like",
            include_str!("../skills/writing.md"),
        ),
        (
            "onboarding",
            "set a repo up for Firelight (the onboarding prompt)",
            firelight_server::onboarding::PROMPT,
        ),
        (
            "ci",
            "the GitHub Action and the local checks",
            include_str!("../skills/ci.md"),
        ),
        (
            "mcp",
            "the MCP tools and how to use them",
            include_str!("../skills/mcp.md"),
        ),
        (
            "config",
            "every server setting and its environment variable",
            include_str!("../../../docs/config.md"),
        ),
    ]
}

pub fn show(topic: Option<&str>, json: bool) -> ExitCode {
    let all = topics();
    if json {
        let v: Vec<_> = all
            .iter()
            .map(|(n, d, _)| serde_json::json!({ "topic": n, "about": d }))
            .collect();
        println!("{}", serde_json::Value::Array(v));
        return ExitCode::SUCCESS;
    }
    match topic {
        None => {
            println!("Firelight skill topics (firelight skills <topic>):\n");
            for (n, d, _) in &all {
                println!("  {n:<11} {d}");
            }
            println!("\nInstall the skill: firelight skills install --project");
            ExitCode::SUCCESS
        }
        Some(t) => match all.iter().find(|(n, _, _)| *n == t) {
            Some((_, _, text)) => {
                println!("{text}");
                ExitCode::SUCCESS
            }
            None => {
                eprintln!(
                    "firelight skills: no topic {t:?}; topics: {}",
                    all.iter()
                        .map(|(n, _, _)| *n)
                        .collect::<Vec<_>>()
                        .join(", ")
                );
                ExitCode::from(2)
            }
        },
    }
}

/// Marks a SKILL.md this command wrote, so it may update it later.
const MARK: &str = "<!-- installed by `firelight skills install`; edits are overwritten -->";

/// Where the skill goes: `(skill dir, symlinks to it)`.
fn targets(project: bool, home: Option<&Path>) -> Result<Vec<(PathBuf, Vec<PathBuf>)>, String> {
    if project {
        // The repo's top folder, so a run from a subfolder lands in the right place.
        let root = std::process::Command::new("git")
            .args(["rev-parse", "--show-toplevel"])
            .env_remove("GIT_DIR")
            .env_remove("GIT_WORK_TREE")
            .output()
            .ok()
            .filter(|o| o.status.success())
            .map(|o| PathBuf::from(String::from_utf8_lossy(&o.stdout).trim()))
            .ok_or("--project needs a git repo (run it inside one)")?;
        return Ok(vec![(
            root.join(".agents/skills/firelight"),
            vec![root.join(".claude/skills/firelight")],
        )]);
    }
    let home = home.ok_or("HOME is not set; use --project")?;
    Ok(vec![
        (home.join(".agents/skills/firelight"), vec![]),
        (home.join(".claude/skills/firelight"), vec![]),
    ])
}

/// Write (or update) the pointer skill. Never overwrites a file it didn't
/// write; running it again changes nothing.
pub fn install(project: bool, home: Option<&Path>) -> Result<Vec<String>, String> {
    let body = format!("{SKILL}\n{MARK}\n");
    let mut done = Vec::new();
    for (dir, links) in targets(project, home)? {
        // Never write through a symlink: a cloned repo could point
        // `.agents` or `.agents/skills/firelight` anywhere on disk.
        let base = if project {
            dir.ancestors().nth(3)
        } else {
            home
        };
        if let Some(base) = base {
            let mut p = base.to_path_buf();
            for c in dir.strip_prefix(base).unwrap_or(&dir).components() {
                p.push(c);
                if std::fs::symlink_metadata(&p).is_ok_and(|m| m.file_type().is_symlink()) {
                    return Err(format!(
                        "{} is a symlink; not writing through it",
                        p.display()
                    ));
                }
            }
        }
        let file = dir.join("SKILL.md");
        let meta = std::fs::symlink_metadata(&file);
        match meta {
            Ok(m) if m.file_type().is_symlink() => {
                return Err(format!("{} is a symlink; not touching it", file.display()));
            }
            Ok(_) => {
                let cur = std::fs::read_to_string(&file).unwrap_or_default();
                if cur == body {
                    done.push(format!("{} is up to date", file.display()));
                    continue;
                }
                if !cur.contains(MARK) {
                    return Err(format!(
                        "{} exists and wasn't written by firelight; not overwriting it",
                        file.display()
                    ));
                }
            }
            Err(_) => {}
        }
        std::fs::create_dir_all(&dir).map_err(|e| format!("{}: {e}", dir.display()))?;
        std::fs::write(&file, &body).map_err(|e| format!("{}: {e}", file.display()))?;
        done.push(format!("wrote {}", file.display()));
        for link in links {
            if std::fs::symlink_metadata(&link).is_ok() {
                done.push(format!("{} exists; left as it is", link.display()));
                continue;
            }
            if let Some(p) = link.parent() {
                std::fs::create_dir_all(p).map_err(|e| format!("{}: {e}", p.display()))?;
            }
            // .claude/skills/firelight → ../../.agents/skills/firelight
            #[cfg(unix)]
            std::os::unix::fs::symlink("../../.agents/skills/firelight", &link)
                .map_err(|e| format!("{}: {e}", link.display()))?;
            done.push(format!("linked {}", link.display()));
        }
    }
    Ok(done)
}

pub fn run_install(project: bool) -> ExitCode {
    let home = std::env::var_os("HOME").map(PathBuf::from);
    match install(project, home.as_deref()) {
        Ok(lines) => {
            for l in lines {
                println!("firelight skills install: {l}");
            }
            ExitCode::SUCCESS
        }
        Err(e) => {
            eprintln!("firelight skills install: {e}");
            ExitCode::from(1)
        }
    }
}

/// `site/llms.txt` (llmstxt.org): what Firelight is and where its docs are.
/// Generated by the drift test below, committed, and deployed as is.
#[cfg(test)]
pub fn llms_txt() -> String {
    let mut s = String::from(
        "# Firelight\n\n> The docs your repos write about themselves: each repo describes itself in a small `.firelight/` folder, and Firelight serves all of them as a link page, a help center, a search by words and meaning, and a read-only MCP server.\n\nInstall the CLI: `curl -fsSL https://firelight.oss.life.li/install.sh | sh`. Every topic below is also built into the binary: `firelight skills <topic>`.\n\n## Docs\n\n",
    );
    for (n, d, _) in topics() {
        s.push_str(&format!(
            "- [{n}](https://firelight.oss.life.li/llms-full.txt#{n}): {d}\n"
        ));
    }
    s.push_str("- [Self-hosting](https://github.com/prosperity-solutions/firelight/blob/main/docs/self-hosting.md): run a Firelight server with Docker, Compose or Helm\n");
    s.push_str("- [Install](https://github.com/prosperity-solutions/firelight/blob/main/docs/install.md): the CLI installer and how it verifies downloads\n");
    s.push_str("\n## Optional\n\n- [Everything above in one file](https://firelight.oss.life.li/llms-full.txt)\n- [Source](https://github.com/prosperity-solutions/firelight)\n");
    s
}

/// `site/llms-full.txt`: every topic in full.
#[cfg(test)]
pub fn llms_full() -> String {
    let mut s = String::from(
        "# Firelight: the full reference\n\nGenerated from the topics built into the firelight binary (`firelight skills <topic>`).\n",
    );
    for (n, d, text) in topics() {
        s.push_str(&format!(
            "\n\n<a id=\"{n}\"></a>\n\n# Topic: {n} ({d})\n\n{}\n",
            text.trim()
        ));
    }
    s
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_skill_is_small_and_runs_nothing() {
        assert!(SKILL.len() <= 6 * 1024, "§12.4: at most 6 KB");
        assert!(!SKILL.contains("!`"), "no inline shell execution");
        assert!(SKILL.starts_with("---\nname: firelight\n"));
        for (n, _, text) in topics() {
            assert!(!text.trim().is_empty(), "{n}");
            assert!(
                SKILL.contains(&format!("firelight skills {n}")),
                "the skill points at {n}"
            );
        }
    }

    /// The site's llms files are generated from the binary's topics; a
    /// change to a topic must regenerate them (UPDATE_SNAPSHOTS=1).
    #[test]
    fn llms_files_match_the_topics() {
        let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../site");
        for (name, want) in [("llms.txt", llms_txt()), ("llms-full.txt", llms_full())] {
            let p = root.join(name);
            if std::env::var_os("UPDATE_SNAPSHOTS").is_some() {
                let _ = std::fs::create_dir_all(&root);
                let _ = std::fs::write(&p, &want);
            }
            let got = std::fs::read_to_string(&p).unwrap_or_default();
            assert!(
                got == want,
                "site/{name} is stale. Run: UPDATE_SNAPSHOTS=1 cargo test -p firelight llms"
            );
        }
    }

    #[test]
    fn install_is_idempotent_and_never_clobbers() {
        let home = std::env::temp_dir().join(format!("fl-skills-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&home);
        let first = install(false, Some(&home)).unwrap_or_default();
        assert!(first.iter().all(|l| l.starts_with("wrote")), "{first:?}");
        let again = install(false, Some(&home)).unwrap_or_default();
        assert!(again.iter().all(|l| l.contains("up to date")), "{again:?}");
        // A SKILL.md someone else wrote stays.
        let theirs = home.join(".claude/skills/firelight/SKILL.md");
        let _ = std::fs::write(&theirs, "mine");
        assert!(install(false, Some(&home)).is_err());
        assert_eq!(std::fs::read_to_string(&theirs).unwrap_or_default(), "mine");
        let _ = std::fs::remove_dir_all(&home);
    }

    #[test]
    fn install_refuses_symlinked_folders() {
        let home = std::env::temp_dir().join(format!("fl-skills-ln-{}", std::process::id()));
        let elsewhere = home.join("elsewhere");
        let _ = std::fs::create_dir_all(&elsewhere);
        #[cfg(unix)]
        {
            let _ = std::os::unix::fs::symlink(&elsewhere, home.join(".agents"));
            let r = install(false, Some(&home));
            assert!(r.is_err(), "{r:?}");
            assert!(!elsewhere.join("skills/firelight/SKILL.md").exists());
        }
        let _ = std::fs::remove_dir_all(&home);
    }
}
