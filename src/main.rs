//! claudin: a small wrapper around Claude Code to switch between accounts.
//!
//! Each account lives in its own config directory `$HOME/.claude_<name>`.
//! `claudin` lists those directories in a menu, lets the user pick one (or
//! create a new one) and then launches `claude` with `CLAUDE_CONFIG_DIR`
//! pointing to the selected directory.

use std::env;
use std::ffi::OsString;
use std::fs;
use std::io::{self, BufRead, Write};
use std::path::{Path, PathBuf};
use std::process::{self, Command};

const PREFIX: &str = ".claude_";
const CONFIG_ENV: &str = "CLAUDE_CONFIG_DIR";

/// Returns the sorted list of account names found in `home`
/// (directories named `.claude_<name>`).
fn list_accounts(home: &Path) -> Vec<String> {
    let mut accounts: Vec<String> = fs::read_dir(home)
        .map(|entries| {
            entries
                .filter_map(Result::ok)
                .filter(|e| e.path().is_dir())
                .filter_map(|e| e.file_name().into_string().ok())
                .filter_map(|n| n.strip_prefix(PREFIX).map(str::to_owned))
                .filter(|n| is_valid_name(n))
                .collect()
        })
        .unwrap_or_default();
    accounts.sort();
    accounts
}

/// A project name may only contain ASCII letters, digits, `-`, `_` and `.`,
/// and must not be empty or consist only of dots.
fn is_valid_name(name: &str) -> bool {
    !name.is_empty()
        && !name.chars().all(|c| c == '.')
        && name
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || matches!(c, '-' | '_' | '.'))
}

fn config_dir(home: &Path, name: &str) -> PathBuf {
    home.join(format!("{PREFIX}{name}"))
}

enum Choice {
    Existing(String),
    New,
    Quit,
}

/// Parses the user's menu input. Options are numbered `1..=accounts.len()`
/// for existing accounts, followed by "New account"; `q` quits.
fn parse_choice(input: &str, accounts: &[String]) -> Option<Choice> {
    let input = input.trim();
    if input.eq_ignore_ascii_case("q") {
        return Some(Choice::Quit);
    }
    if input.eq_ignore_ascii_case("n") {
        return Some(Choice::New);
    }
    let n: usize = input.parse().ok()?;
    match n {
        0 => None,
        n if n <= accounts.len() => Some(Choice::Existing(accounts[n - 1].clone())),
        n if n == accounts.len() + 1 => Some(Choice::New),
        _ => None,
    }
}

/// Reads one line from `input`. Returns `None` on EOF.
fn read_line(input: &mut impl BufRead) -> io::Result<Option<String>> {
    let mut line = String::new();
    if input.read_line(&mut line)? == 0 {
        return Ok(None);
    }
    Ok(Some(line.trim().to_owned()))
}

fn prompt_new_name(
    home: &Path,
    input: &mut impl BufRead,
    out: &mut impl Write,
) -> io::Result<Option<String>> {
    loop {
        write!(out, "New project name: ")?;
        out.flush()?;
        let Some(name) = read_line(input)? else {
            return Ok(None);
        };
        if !is_valid_name(&name) {
            writeln!(
                out,
                "Invalid name. Use only letters, digits, '-', '_' or '.'."
            )?;
            continue;
        }
        if config_dir(home, &name).exists() {
            writeln!(out, "Account '{name}' already exists.")?;
            continue;
        }
        return Ok(Some(name));
    }
}

/// Shows the menu and returns the selected account name, or `None` if the
/// user quit. When there are no accounts, goes straight to creating one.
fn select_account(
    home: &Path,
    input: &mut impl BufRead,
    out: &mut impl Write,
) -> io::Result<Option<String>> {
    let accounts = list_accounts(home);
    if accounts.is_empty() {
        writeln!(out, "No Claude accounts configured yet. Let's create one.")?;
        return prompt_new_name(home, input, out);
    }
    loop {
        writeln!(out, "Select a Claude account:")?;
        for (i, name) in accounts.iter().enumerate() {
            writeln!(out, "  {}) {name}", i + 1)?;
        }
        writeln!(out, "  {}) New account", accounts.len() + 1)?;
        writeln!(out, "  q) Quit")?;
        write!(out, "> ")?;
        out.flush()?;
        let Some(line) = read_line(input)? else {
            return Ok(None);
        };
        match parse_choice(&line, &accounts) {
            Some(Choice::Existing(name)) => return Ok(Some(name)),
            Some(Choice::New) => return prompt_new_name(home, input, out),
            Some(Choice::Quit) => return Ok(None),
            None => writeln!(out, "Invalid choice.\n")?,
        }
    }
}

fn run_claude(dir: &Path, args: Vec<OsString>) -> io::Result<i32> {
    let mut cmd = Command::new("claude");
    cmd.args(args).env(CONFIG_ENV, dir);
    #[cfg(unix)]
    {
        use std::os::unix::process::CommandExt;
        // Only returns on failure.
        Err(cmd.exec())
    }
    #[cfg(not(unix))]
    {
        Ok(cmd.status()?.code().unwrap_or(1))
    }
}

fn main() {
    let Some(home) = env::var_os("HOME").map(PathBuf::from) else {
        eprintln!("claudin: HOME is not set");
        process::exit(1);
    };

    let stdin = io::stdin();
    let mut input = stdin.lock();
    let mut out = io::stdout();
    let name = match select_account(&home, &mut input, &mut out) {
        Ok(Some(name)) => name,
        Ok(None) => return,
        Err(e) => {
            eprintln!("claudin: {e}");
            process::exit(1);
        }
    };

    let dir = config_dir(&home, &name);
    if let Err(e) = fs::create_dir_all(&dir) {
        eprintln!("claudin: cannot create {}: {e}", dir.display());
        process::exit(1);
    }

    println!("Using {CONFIG_ENV}={}", dir.display());
    match run_claude(&dir, env::args_os().skip(1).collect()) {
        Ok(code) => process::exit(code),
        Err(e) => {
            eprintln!("claudin: failed to run 'claude': {e}");
            process::exit(1);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Cursor;

    fn temp_home(tag: &str) -> PathBuf {
        let dir = env::temp_dir().join(format!("claudin-test-{tag}-{}", process::id()));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();
        dir
    }

    #[test]
    fn validates_names() {
        assert!(is_valid_name("work"));
        assert!(is_valid_name("my-proj_1.2"));
        assert!(!is_valid_name(""));
        assert!(!is_valid_name(".."));
        assert!(!is_valid_name("a/b"));
        assert!(!is_valid_name("a b"));
    }

    #[test]
    fn lists_only_claude_account_dirs_sorted() {
        let home = temp_home("list");
        fs::create_dir(home.join(".claude_work")).unwrap();
        fs::create_dir(home.join(".claude_personal")).unwrap();
        fs::create_dir(home.join(".claude")).unwrap();
        fs::create_dir(home.join(".claude_")).unwrap();
        fs::write(home.join(".claude_file"), "").unwrap();
        assert_eq!(list_accounts(&home), vec!["personal", "work"]);
        fs::remove_dir_all(&home).unwrap();
    }

    #[test]
    fn parses_choices() {
        let accounts = vec!["a".to_string(), "b".to_string()];
        assert!(matches!(parse_choice("1", &accounts), Some(Choice::Existing(n)) if n == "a"));
        assert!(matches!(parse_choice(" 2 ", &accounts), Some(Choice::Existing(n)) if n == "b"));
        assert!(matches!(parse_choice("3", &accounts), Some(Choice::New)));
        assert!(matches!(parse_choice("n", &accounts), Some(Choice::New)));
        assert!(matches!(parse_choice("q", &accounts), Some(Choice::Quit)));
        assert!(parse_choice("0", &accounts).is_none());
        assert!(parse_choice("4", &accounts).is_none());
        assert!(parse_choice("x", &accounts).is_none());
    }

    #[test]
    fn no_accounts_prompts_for_new_name() {
        let home = temp_home("empty");
        let mut input = Cursor::new("bad name\nwork\n");
        let mut out = Vec::new();
        let name = select_account(&home, &mut input, &mut out).unwrap();
        assert_eq!(name.as_deref(), Some("work"));
        assert!(String::from_utf8(out).unwrap().contains("Invalid name"));
        fs::remove_dir_all(&home).unwrap();
    }

    #[test]
    fn selects_existing_or_new_account() {
        let home = temp_home("select");
        fs::create_dir(home.join(".claude_work")).unwrap();

        let mut out = Vec::new();
        let name = select_account(&home, &mut Cursor::new("1\n"), &mut out).unwrap();
        assert_eq!(name.as_deref(), Some("work"));

        let name = select_account(&home, &mut Cursor::new("9\n2\nwork\nhome\n"), &mut out).unwrap();
        assert_eq!(name.as_deref(), Some("home"));
        let text = String::from_utf8(out).unwrap();
        assert!(text.contains("Invalid choice"));
        assert!(text.contains("already exists"));

        let name = select_account(&home, &mut Cursor::new("q\n"), &mut Vec::new()).unwrap();
        assert!(name.is_none());
        fs::remove_dir_all(&home).unwrap();
    }
}
