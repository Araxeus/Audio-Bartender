//! Repository automation. Run with `cargo xtask <command>`.

use std::io::{self, BufRead, Write};
use std::path::{Path, PathBuf};
use std::process::Command;

type Result<T = ()> = std::result::Result<T, String>;

const USAGE: &str = "\
Usage: cargo xtask release [patch|minor|major|X.Y.Z] [--push | --no-push]

Bumps the version in Cargo.toml and apps/tauri/tauri.conf.json, commits
`chore(release): vX.Y.Z`, tags it and pushes. The pushed tag starts the Release
workflow, which builds the installer and portable exes and publishes the release.

Without a level it asks for one. Without --push or --no-push it asks before pushing.";

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let result = match args.first().map(String::as_str) {
        Some("release") => release(&args[1..]),
        _ => Err(USAGE.to_owned()),
    };
    if let Err(error) = result {
        eprintln!("{error}");
        std::process::exit(1);
    }
}

fn release(args: &[String]) -> Result {
    let mut level = None;
    let mut push = None;
    for arg in args {
        match arg.as_str() {
            "--push" => push = Some(true),
            "--no-push" => push = Some(false),
            "-h" | "--help" => return Err(USAGE.to_owned()),
            other if level.is_none() && !other.starts_with('-') => level = Some(other.to_owned()),
            other => return Err(format!("Unknown argument {other}.\n\n{USAGE}")),
        }
    }

    let root = Path::new(env!("CARGO_MANIFEST_DIR")).parent().ok_or("xtask has no parent directory")?.to_path_buf();
    if !git(&root, &["status", "--porcelain"])?.trim().is_empty() {
        return Err("The working tree has changes. Commit or stash them first.".to_owned());
    }
    let branch = git(&root, &["rev-parse", "--abbrev-ref", "HEAD"])?;
    if branch.trim() != "main" {
        return Err(format!("Releases come from main, but this is {}.", branch.trim()));
    }

    let manifest_path = root.join("Cargo.toml");
    let manifest = read(&manifest_path)?;
    let current = workspace_version(&manifest)?;
    let next = match level {
        Some(level) => next_version(current, &level)?,
        None => ask_version(current)?,
    };
    let tag = format!("v{}", show(next));
    if git(&root, &["tag", "--list", &tag])?.trim() == tag {
        return Err(format!("Tag {tag} already exists."));
    }
    let push = match push {
        Some(push) => push,
        None => ask(&format!("Push {tag} to origin and start the release? [Y/n] "))?,
    };

    write(&manifest_path, &set_workspace_version(&manifest, current, next)?)?;
    let conf_path = root.join("apps/tauri/tauri.conf.json");
    write(&conf_path, &set_tauri_version(&read(&conf_path)?, current, next)?)?;
    println!("Bumped {} to {}", show(current), show(next));

    // Puts the new version into the lockfile's entries for this workspace's crates.
    run(&root, "cargo", &["update", "--workspace", "--offline"])?;
    run(&root, "git", &["add", "Cargo.toml", "Cargo.lock", "apps/tauri/tauri.conf.json"])?;
    run(&root, "git", &["commit", "-m", &format!("chore(release): {tag}")])?;
    run(&root, "git", &["tag", "-a", &tag, "-m", &tag])?;

    if push {
        // Atomic, so the tag never lands without the commit it points at.
        run(&root, "git", &["push", "--atomic", "origin", "main", &tag])?;
        println!("Pushed {tag}. The Release workflow builds and publishes it:");
        println!("https://github.com/Araxeus/Audio-Bartender/actions/workflows/release.yml");
    } else {
        println!("Committed and tagged {tag}. Run `git push --atomic origin main {tag}` to release it.");
    }
    Ok(())
}

type Version = (u64, u64, u64);

fn show((major, minor, patch): Version) -> String {
    format!("{major}.{minor}.{patch}")
}

fn parse(text: &str) -> Result<Version> {
    let parts: Vec<u64> = text
        .split('.')
        .map(|part| part.parse().map_err(|_| format!("{text} is not a version like 1.2.3")))
        .collect::<Result<_>>()?;
    match parts[..] {
        [major, minor, patch] => Ok((major, minor, patch)),
        _ => Err(format!("{text} is not a version like 1.2.3")),
    }
}

fn next_version((major, minor, patch): Version, level: &str) -> Result<Version> {
    let next = match level {
        "patch" => (major, minor, patch + 1),
        "minor" => (major, minor + 1, 0),
        "major" => (major + 1, 0, 0),
        exact => parse(exact.trim_start_matches('v'))?,
    };
    if next <= (major, minor, patch) {
        return Err(format!("{} is not newer than {}.", show(next), show((major, minor, patch))));
    }
    Ok(next)
}

/// The `version` under `[workspace.package]`.
fn workspace_version(manifest: &str) -> Result<Version> {
    let line = version_line(manifest)?;
    parse(line.trim_start_matches("version = \"").trim_end_matches('"'))
}

fn version_line(manifest: &str) -> Result<&str> {
    manifest
        .lines()
        .skip_while(|line| line.trim() != "[workspace.package]")
        .skip(1)
        .take_while(|line| !line.trim_start().starts_with('['))
        .find(|line| line.starts_with("version = \""))
        .ok_or_else(|| "Cargo.toml has no version under [workspace.package]".to_owned())
}

fn set_workspace_version(manifest: &str, current: Version, next: Version) -> Result<String> {
    let line = version_line(manifest)?;
    let updated = format!("version = \"{}\"", show(next));
    debug_assert_eq!(line, format!("version = \"{}\"", show(current)));
    Ok(manifest.replacen(line, &updated, 1))
}

fn set_tauri_version(conf: &str, current: Version, next: Version) -> Result<String> {
    let old = format!("\"version\": \"{}\"", show(current));
    if conf.matches(&old).count() != 1 {
        return Err(format!("tauri.conf.json should hold {old} exactly once"));
    }
    Ok(conf.replacen(&old, &format!("\"version\": \"{}\"", show(next)), 1))
}

fn ask_version(current: Version) -> Result<Version> {
    let choices = ["patch", "minor", "major"];
    println!("Current version: {}", show(current));
    for (i, level) in choices.iter().enumerate() {
        println!("  {}) {level:<5}  {}", i + 1, show(next_version(current, level)?));
    }
    let answer = prompt("Bump to [1]: ")?;
    match answer.as_str() {
        "" | "1" => next_version(current, "patch"),
        "2" => next_version(current, "minor"),
        "3" => next_version(current, "major"),
        other => next_version(current, other),
    }
}

fn ask(question: &str) -> Result<bool> {
    match prompt(question)?.to_lowercase().as_str() {
        "" | "y" | "yes" => Ok(true),
        "n" | "no" => Ok(false),
        other => Err(format!("Expected y or n, got {other}.")),
    }
}

fn prompt(question: &str) -> Result<String> {
    print!("{question}");
    io::stdout().flush().map_err(|e| e.to_string())?;
    let mut line = String::new();
    io::stdin().lock().read_line(&mut line).map_err(|e| e.to_string())?;
    Ok(line.trim().to_owned())
}

fn read(path: &PathBuf) -> Result<String> {
    std::fs::read_to_string(path).map_err(|e| format!("Reading {}: {e}", path.display()))
}

fn write(path: &PathBuf, text: &str) -> Result {
    std::fs::write(path, text).map_err(|e| format!("Writing {}: {e}", path.display()))
}

fn git(root: &Path, args: &[&str]) -> Result<String> {
    let output = Command::new("git").args(args).current_dir(root).output().map_err(|e| e.to_string())?;
    if !output.status.success() {
        return Err(format!("git {} failed: {}", args.join(" "), String::from_utf8_lossy(&output.stderr)));
    }
    Ok(String::from_utf8_lossy(&output.stdout).into_owned())
}

fn run(root: &Path, program: &str, args: &[&str]) -> Result {
    let status = Command::new(program).args(args).current_dir(root).status().map_err(|e| e.to_string())?;
    if !status.success() {
        return Err(format!("{program} {} failed", args.join(" ")));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    const MANIFEST: &str = "\
[workspace]
members = [\"a\"]

[workspace.package]
version = \"0.4.2\"
edition = \"2024\"

[profile.release]
opt-level = \"s\"
";

    #[test]
    fn levels_bump_the_right_part() {
        assert_eq!(next_version((0, 4, 2), "patch"), Ok((0, 4, 3)));
        assert_eq!(next_version((0, 4, 2), "minor"), Ok((0, 5, 0)));
        assert_eq!(next_version((0, 4, 2), "major"), Ok((1, 0, 0)));
        assert_eq!(next_version((0, 4, 2), "v0.6.0"), Ok((0, 6, 0)));
    }

    #[test]
    fn an_exact_version_must_be_newer() {
        assert!(next_version((0, 4, 2), "0.4.2").is_err());
        assert!(next_version((0, 4, 2), "0.3.9").is_err());
        assert!(next_version((0, 4, 2), "1.0").is_err());
    }

    #[test]
    fn only_the_workspace_version_changes() {
        assert_eq!(workspace_version(MANIFEST), Ok((0, 4, 2)));
        let out = set_workspace_version(MANIFEST, (0, 4, 2), (0, 5, 0)).unwrap();
        assert_eq!(out, MANIFEST.replace("version = \"0.4.2\"", "version = \"0.5.0\""));
    }

    #[test]
    fn the_tauri_version_follows() {
        let conf = "{\n  \"productName\": \"X\",\n  \"version\": \"0.4.2\",\n}";
        assert_eq!(set_tauri_version(conf, (0, 4, 2), (0, 5, 0)).unwrap(), conf.replace("0.4.2", "0.5.0"));
        assert!(set_tauri_version(conf, (0, 4, 1), (0, 5, 0)).is_err());
    }

    #[test]
    fn the_repository_versions_agree() {
        let root = Path::new(env!("CARGO_MANIFEST_DIR")).parent().unwrap();
        let version = workspace_version(&std::fs::read_to_string(root.join("Cargo.toml")).unwrap()).unwrap();
        let conf = std::fs::read_to_string(root.join("apps/tauri/tauri.conf.json")).unwrap();
        assert!(
            conf.contains(&format!("\"version\": \"{}\"", show(version))),
            "tauri.conf.json is not on {}",
            show(version)
        );
    }
}
