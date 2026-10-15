//! claudin: a small wrapper around Claude Code to switch between accounts.
//!
//! Each account lives in its own config directory `$HOME/.claude_<name>`.
//! `claudin` lists those directories in a menu. In a terminal, the user moves
//! with the arrow keys and confirms with Enter (or creates a new account).
//! `claudin` then launches `claude` with `CLAUDE_CONFIG_DIR` pointing to the
//! selected directory. `claudin --gui` opens a window for the same setup and
//! starts the Claude desktop app. Inside Herdr (`HERDR_ENV=1`), the terminal
//! app starts with a short prompt asking Claude to configure the Herdr
//! workspace for that account. After the account is chosen, claudin asks
//! whether to open the terminal app or the desktop app. A missing terminal
//! app can be installed with the official script. A missing desktop app is
//! installed by hand from the Claude Code download page. The desktop binary
//! is `/Applications/Claude.app/Contents/MacOS/Claude` on macOS, or
//! `%LOCALAPPDATA%\Anthropic\Claude\claude.exe` on Windows.

use std::env;
use std::ffi::OsString;
use std::fs;
use std::io::{self, BufRead, IsTerminal, Write};
use std::path::{Path, PathBuf};
use std::process::{self, Command, Stdio};

mod gui;

const PREFIX: &str = ".claude_";
const CONFIG_ENV: &str = "CLAUDE_CONFIG_DIR";
const TUI_INSTALL_COMMAND: &str = "curl -fsSL https://claude.ai/install.sh | bash";
const GUI_INSTALL_URL: &str = "https://claude.com/product/claude-code";

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Surface {
    Tui,
    Gui,
}

fn surface_label(surface: Surface) -> &'static str {
    match surface {
        Surface::Tui => "Terminal",
        Surface::Gui => "Desktop app",
    }
}

fn accepts_install(answer: &str) -> bool {
    matches!(answer.trim().to_ascii_lowercase().as_str(), "y" | "yes")
}

fn tui_missing_message() -> String {
    format!(
        "The Claude Code terminal app is not installed.\nInstall it by running:\n  {TUI_INSTALL_COMMAND}"
    )
}

fn gui_missing_message() -> String {
    format!(
        "The Claude desktop app is not installed.\nInstall it manually from {GUI_INSTALL_URL}\nthen run claudin again."
    )
}

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

#[derive(Clone)]
enum Choice {
    Existing(String),
    New,
    Remove,
    Quit,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum MenuKey {
    Up,
    Down,
    Enter,
    Quit,
}

#[derive(Debug, PartialEq, Eq)]
enum MenuStep {
    Move(usize),
    Choose(usize),
    Quit,
}

fn menu_entries(accounts: &[String]) -> Vec<Choice> {
    accounts
        .iter()
        .cloned()
        .map(Choice::Existing)
        .chain([Choice::New, Choice::Remove, Choice::Quit])
        .collect()
}

fn entry_label(entry: &Choice) -> &str {
    match entry {
        Choice::Existing(name) => name,
        Choice::New => "New account",
        Choice::Remove => "Remove account",
        Choice::Quit => "Quit",
    }
}

/// Moves the highlight, or confirms the highlighted row.
/// Up and down stop at the ends of the list.
fn step_menu(cursor: usize, len: usize, key: MenuKey) -> MenuStep {
    match key {
        MenuKey::Up => MenuStep::Move(cursor.saturating_sub(1)),
        MenuKey::Down => MenuStep::Move(if cursor + 1 < len { cursor + 1 } else { cursor }),
        MenuKey::Enter => MenuStep::Choose(cursor),
        MenuKey::Quit => MenuStep::Quit,
    }
}

fn decode_key(key: crossterm::event::KeyEvent) -> Option<MenuKey> {
    if key.kind == crossterm::event::KeyEventKind::Release {
        return None;
    }
    match key.code {
        crossterm::event::KeyCode::Up => Some(MenuKey::Up),
        crossterm::event::KeyCode::Down => Some(MenuKey::Down),
        crossterm::event::KeyCode::Enter => Some(MenuKey::Enter),
        crossterm::event::KeyCode::Esc => Some(MenuKey::Quit),
        crossterm::event::KeyCode::Char('q' | 'Q') => Some(MenuKey::Quit),
        crossterm::event::KeyCode::Char('c' | 'd')
            if key
                .modifiers
                .contains(crossterm::event::KeyModifiers::CONTROL) =>
        {
            Some(MenuKey::Quit)
        }
        _ => None,
    }
}

/// Draws the menu and returns how many rows it occupied.
fn paint_menu(
    out: &mut impl Write,
    title: &str,
    labels: &[&str],
    cursor: usize,
) -> io::Result<u16> {
    let mut lines = 0u16;
    write!(out, "\r\x1b[2K{title}\r\n")?;
    lines += 1;
    for (i, label) in labels.iter().enumerate() {
        if i == cursor {
            write!(out, "\r\x1b[2K\x1b[7m> {label}\x1b[0m\r\n")?;
        } else {
            write!(out, "\r\x1b[2K  {label}\r\n")?;
        }
        lines += 1;
    }
    out.flush()?;
    Ok(lines)
}

fn choose_from_labels(title: &str, labels: &[&str]) -> io::Result<Option<usize>> {
    if labels.is_empty() {
        return Ok(None);
    }
    let mut cursor = 0usize;
    crossterm::terminal::enable_raw_mode()?;
    let _guard = RawTermGuard;
    let mut out = io::stderr();
    let _ = crossterm::execute!(out, crossterm::cursor::Hide);
    let mut lines = paint_menu(&mut out, title, labels, cursor)?;
    loop {
        match step_menu(cursor, labels.len(), next_menu_key()?) {
            MenuStep::Move(next) if next != cursor => {
                cursor = next;
                let _ = crossterm::execute!(out, crossterm::cursor::MoveUp(lines));
                lines = paint_menu(&mut out, title, labels, cursor)?;
            }
            MenuStep::Move(_) => {}
            MenuStep::Choose(index) => return Ok(Some(index)),
            MenuStep::Quit => return Ok(None),
        }
    }
}

struct RawTermGuard;

impl Drop for RawTermGuard {
    fn drop(&mut self) {
        let mut err = io::stderr();
        let _ = write!(err, "\x1b[0m\x1b[?25h");
        let _ = err.flush();
        let _ = crossterm::terminal::disable_raw_mode();
    }
}

fn next_menu_key() -> io::Result<MenuKey> {
    loop {
        let event = match crossterm::event::read() {
            Ok(event) => event,
            Err(e) if e.kind() == io::ErrorKind::UnexpectedEof => return Ok(MenuKey::Quit),
            Err(e) => return Err(e),
        };
        if let crossterm::event::Event::Key(key) = event
            && let Some(menu_key) = decode_key(key)
        {
            return Ok(menu_key);
        }
    }
}

fn choose_interactive(accounts: &[String]) -> io::Result<Choice> {
    let entries = menu_entries(accounts);
    let labels: Vec<&str> = entries.iter().map(entry_label).collect();
    match choose_from_labels("Select a Claude account", &labels)? {
        Some(index) => Ok(entries[index].clone()),
        None => Ok(Choice::Quit),
    }
}

fn finish_choice(
    home: &Path,
    choice: Choice,
    input: &mut impl BufRead,
    out: &mut impl Write,
) -> io::Result<Option<String>> {
    match choice {
        Choice::Existing(name) => Ok(Some(name)),
        Choice::New => prompt_new_name(home, input, out),
        Choice::Remove => Ok(None),
        Choice::Quit => Ok(None),
    }
}

fn delete_account(home: &Path, name: &str) -> io::Result<()> {
    let dir = config_dir(home, name);
    if dir.exists() {
        fs::remove_dir_all(&dir)?;
    }
    Ok(())
}

fn confirm_remove(name: &str, input: &mut impl BufRead, out: &mut impl Write) -> io::Result<bool> {
    writeln!(
        out,
        "Remove account '{name}' and delete its config directory? [y/N]"
    )?;
    write!(out, "> ")?;
    out.flush()?;
    let Some(answer) = read_line(input)? else {
        return Ok(false);
    };
    Ok(accepts_install(&answer))
}

fn remove_account_lines(
    home: &Path,
    accounts: &[String],
    input: &mut impl BufRead,
    out: &mut impl Write,
) -> io::Result<()> {
    loop {
        writeln!(out, "Remove an account:")?;
        for (i, name) in accounts.iter().enumerate() {
            writeln!(out, "  {}) {name}", i + 1)?;
        }
        writeln!(out, "  b) Back")?;
        write!(out, "> ")?;
        out.flush()?;
        let Some(line) = read_line(input)? else {
            return Ok(());
        };
        let line = line.trim();
        if line.eq_ignore_ascii_case("b") || line.eq_ignore_ascii_case("q") {
            return Ok(());
        }
        let Ok(n) = line.parse::<usize>() else {
            writeln!(out, "Invalid choice.\n")?;
            continue;
        };
        if n == 0 || n > accounts.len() {
            writeln!(out, "Invalid choice.\n")?;
            continue;
        }
        let name = &accounts[n - 1];
        if confirm_remove(name, input, out)? {
            delete_account(home, name)?;
            writeln!(out, "Removed account '{name}'.")?;
        }
        return Ok(());
    }
}

fn remove_account_tty(
    home: &Path,
    accounts: &[String],
    input: &mut impl BufRead,
    out: &mut impl Write,
) -> io::Result<()> {
    let mut labels: Vec<&str> = accounts.iter().map(String::as_str).collect();
    labels.push("Back");
    let Some(index) = choose_from_labels("Remove an account", &labels)? else {
        return Ok(());
    };
    if index >= accounts.len() {
        return Ok(());
    }
    let name = &accounts[index];
    if confirm_remove(name, input, out)? {
        delete_account(home, name)?;
        writeln!(out, "Removed account '{name}'.")?;
    }
    Ok(())
}

fn create_first_account(
    home: &Path,
    input: &mut impl BufRead,
    out: &mut impl Write,
) -> io::Result<Option<String>> {
    writeln!(out, "No Claude accounts configured yet. Let's create one.")?;
    prompt_new_name(home, input, out)
}

/// Parses a typed menu line. Options are numbered `1..=accounts.len()`
/// for existing accounts, then "New account" and "Remove account"; `q` quits.
fn parse_choice(input: &str, accounts: &[String]) -> Option<Choice> {
    let input = input.trim();
    if input.eq_ignore_ascii_case("q") {
        return Some(Choice::Quit);
    }
    if input.eq_ignore_ascii_case("n") {
        return Some(Choice::New);
    }
    if input.eq_ignore_ascii_case("r") {
        return Some(Choice::Remove);
    }
    let n: usize = input.parse().ok()?;
    match n {
        0 => None,
        n if n <= accounts.len() => Some(Choice::Existing(accounts[n - 1].clone())),
        n if n == accounts.len() + 1 => Some(Choice::New),
        n if n == accounts.len() + 2 => Some(Choice::Remove),
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

/// Numbered menu used when stdin is not a terminal.
fn select_account_lines(
    home: &Path,
    input: &mut impl BufRead,
    out: &mut impl Write,
) -> io::Result<Option<String>> {
    let mut accounts = list_accounts(home);
    if accounts.is_empty() {
        return create_first_account(home, input, out);
    }
    loop {
        writeln!(out, "Select a Claude account:")?;
        for (i, name) in accounts.iter().enumerate() {
            writeln!(out, "  {}) {name}", i + 1)?;
        }
        writeln!(out, "  {}) New account", accounts.len() + 1)?;
        writeln!(out, "  {}) Remove account", accounts.len() + 2)?;
        writeln!(out, "  q) Quit")?;
        write!(out, "> ")?;
        out.flush()?;
        let Some(line) = read_line(input)? else {
            return Ok(None);
        };
        match parse_choice(&line, &accounts) {
            Some(Choice::Remove) => {
                remove_account_lines(home, &accounts, input, out)?;
                accounts = list_accounts(home);
                if accounts.is_empty() {
                    return create_first_account(home, input, out);
                }
            }
            Some(choice) => return finish_choice(home, choice, input, out),
            None => writeln!(out, "Invalid choice.\n")?,
        }
    }
}

/// Arrow-key menu. When there are no accounts, asks for a new name.
fn select_account_tty(home: &Path) -> io::Result<Option<String>> {
    let mut out = io::stderr();
    loop {
        let accounts = list_accounts(home);
        let stdin = io::stdin();
        if accounts.is_empty() {
            let mut input = stdin.lock();
            return create_first_account(home, &mut input, &mut out);
        }
        // Read the menu before locking stdin. The arrow-key reader uses the
        // terminal directly, and later prompts need the line reader.
        match choose_interactive(&accounts)? {
            Choice::Remove => {
                let mut input = stdin.lock();
                remove_account_tty(home, &accounts, &mut input, &mut out)?;
            }
            choice => {
                let mut input = stdin.lock();
                return finish_choice(home, choice, &mut input, &mut out);
            }
        }
    }
}

/// Desktop install locations used when `claude` is not on `PATH`.
#[cfg(target_os = "macos")]
fn fallback_claude_paths(_home: &Path) -> Vec<PathBuf> {
    vec![PathBuf::from(
        "/Applications/Claude.app/Contents/MacOS/Claude",
    )]
}

#[cfg(windows)]
fn fallback_claude_paths(home: &Path) -> Vec<PathBuf> {
    let local_app_data = env::var_os("LOCALAPPDATA").map(PathBuf::from);
    windows_claude_paths(home, local_app_data.as_deref())
}

#[cfg(not(any(target_os = "macos", windows)))]
fn fallback_claude_paths(_home: &Path) -> Vec<PathBuf> {
    Vec::new()
}

/// `%LOCALAPPDATA%\Anthropic\Claude\claude.exe`, which is
/// `%USERPROFILE%\AppData\Local\Anthropic\Claude\claude.exe` by default.
/// `local_app_data` wins when it points somewhere else; the profile path is
/// still tried when it differs.
#[cfg(any(windows, test))]
fn windows_claude_paths(home: &Path, local_app_data: Option<&Path>) -> Vec<PathBuf> {
    let mut paths = Vec::new();
    if let Some(local_app_data) = local_app_data {
        paths.push(
            local_app_data
                .join("Anthropic")
                .join("Claude")
                .join("claude.exe"),
        );
    }
    let from_home = home
        .join("AppData")
        .join("Local")
        .join("Anthropic")
        .join("Claude")
        .join("claude.exe");
    if paths.iter().all(|path| path != &from_home) {
        paths.push(from_home);
    }
    paths
}

fn is_runnable(path: &Path) -> bool {
    let Ok(meta) = fs::metadata(path) else {
        return false;
    };
    if !meta.is_file() {
        return false;
    }
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        meta.permissions().mode() & 0o111 != 0
    }
    #[cfg(not(unix))]
    {
        true
    }
}

fn first_runnable(paths: &[PathBuf]) -> Option<&Path> {
    paths
        .iter()
        .find(|path| is_runnable(path))
        .map(PathBuf::as_path)
}

fn in_herdr(flag: Option<&str>) -> bool {
    flag == Some("1")
}

/// Prompt Claude uses to configure Herdr. Claudin does not call `herdr` itself.
fn herdr_setup_prompt(account: &str) -> String {
    format!(
        "Configure Herdr for the logged-in Claude account \"{account}\". \
Name the workspace so it mentions this account, and set the tab, pane, and notification to match. \
Use the herdr CLI, then wait for my next request."
    )
}

/// When this terminal is inside Herdr and the user did not already pass Claude
/// arguments, the terminal app starts with `herdr_setup_prompt`.
fn claude_args_for_launch(in_herdr: bool, account: &str, args: &[OsString]) -> Vec<OsString> {
    if in_herdr && args.is_empty() {
        vec![OsString::from(herdr_setup_prompt(account))]
    } else {
        args.to_vec()
    }
}

fn program_on_path(name: &str) -> bool {
    let Some(path_var) = env::var_os("PATH") else {
        return false;
    };
    env::split_paths(&path_var).any(|dir| {
        if is_runnable(&dir.join(name)) {
            return true;
        }
        #[cfg(windows)]
        {
            if is_runnable(&dir.join(format!("{name}.exe"))) {
                return true;
            }
        }
        false
    })
}

struct LaunchRequest {
    window: bool,
    account: Option<String>,
    claude_args: Vec<OsString>,
}

/// Splits `claudin` flags from arguments that are forwarded to Claude.
/// `--` ends `claudin` flags. It is not forwarded. Everything after it is.
fn parse_launch_args(args: Vec<OsString>) -> Result<LaunchRequest, String> {
    let mut claude_args = args;
    let mut forwarded = take_passthrough(&mut claude_args);
    let window = take_gui_flag(&mut claude_args);
    let account = take_account_arg(&mut claude_args)?;
    claude_args.append(&mut forwarded);
    Ok(LaunchRequest {
        window,
        account,
        claude_args,
    })
}

/// Removes the first `--` and returns every argument after it.
fn take_passthrough(args: &mut Vec<OsString>) -> Vec<OsString> {
    let Some(index) = args.iter().position(|arg| arg == "--") else {
        return Vec::new();
    };
    let forwarded = args.split_off(index + 1);
    args.pop();
    forwarded
}

/// Removes a leading `--gui`, including `--account <name> --gui`.
/// Later arguments stay in `args` and are forwarded to the desktop app.
fn take_gui_flag(args: &mut Vec<OsString>) -> bool {
    if args.first().is_some_and(|arg| arg == "--gui") {
        args.remove(0);
        return true;
    }
    if args.len() >= 3 && args[0] == "--account" && args[2] == "--gui" {
        args.remove(2);
        return true;
    }
    false
}

/// Removes a leading `--account <name>` so the menu can be skipped.
/// Later arguments stay in `args` and are forwarded to `claude`.
fn take_account_arg(args: &mut Vec<OsString>) -> Result<Option<String>, String> {
    if args.first().and_then(|arg| arg.to_str()) != Some("--account") {
        return Ok(None);
    }
    let Some(name) = args.get(1).and_then(|arg| arg.to_str()) else {
        return Err("--account needs a name".to_owned());
    };
    let name = name.to_owned();
    if !is_valid_name(&name) {
        return Err(format!(
            "invalid account name '{name}'. Use only letters, digits, '-', '_' or '.'."
        ));
    }
    args.drain(0..2);
    Ok(Some(name))
}

fn spawn_claude(program: &Path, dir: &Path, args: &[OsString]) -> io::Result<i32> {
    let mut cmd = Command::new(program);
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

/// Starts a desktop app without keeping the terminal. The child gets its own
/// process group, its output is discarded, and this process does not wait.
fn spawn_in_background(program: &Path, dir: &Path, args: &[OsString]) -> io::Result<i32> {
    let mut cmd = Command::new(program);
    cmd.args(args)
        .env(CONFIG_ENV, dir)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null());
    #[cfg(unix)]
    {
        use std::os::unix::process::CommandExt;
        cmd.process_group(0);
    }
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        // New process group, detached from this console.
        cmd.creation_flags(0x0000_0200 | 0x0000_0008);
    }
    cmd.spawn()?;
    Ok(0)
}

fn tui_binary(home: &Path) -> Option<PathBuf> {
    if program_on_path("claude") {
        return Some(PathBuf::from("claude"));
    }
    let local = home.join(".local").join("bin").join(if cfg!(windows) {
        "claude.exe"
    } else {
        "claude"
    });
    is_runnable(&local).then_some(local)
}

fn gui_binary(home: &Path) -> Option<PathBuf> {
    first_runnable(&fallback_claude_paths(home)).map(Path::to_path_buf)
}

fn select_surface_lines(
    input: &mut impl BufRead,
    out: &mut impl Write,
) -> io::Result<Option<Surface>> {
    loop {
        writeln!(out, "Open Claude with:")?;
        writeln!(out, "  1) {}", surface_label(Surface::Tui))?;
        writeln!(out, "  2) {}", surface_label(Surface::Gui))?;
        writeln!(out, "  q) Quit")?;
        write!(out, "> ")?;
        out.flush()?;
        let Some(line) = read_line(input)? else {
            return Ok(None);
        };
        match line.trim() {
            "1" => return Ok(Some(Surface::Tui)),
            "2" => return Ok(Some(Surface::Gui)),
            "q" | "Q" => return Ok(None),
            _ => writeln!(out, "Invalid choice.\n")?,
        }
    }
}

fn select_surface_tty() -> io::Result<Option<Surface>> {
    let labels = [surface_label(Surface::Tui), surface_label(Surface::Gui)];
    Ok(
        choose_from_labels("Open Claude with", &labels)?.map(|index| match index {
            0 => Surface::Tui,
            _ => Surface::Gui,
        }),
    )
}

fn confirm_tui_install(input: &mut impl BufRead, out: &mut impl Write) -> io::Result<bool> {
    writeln!(out, "{}", tui_missing_message())?;
    write!(out, "Run that now? [y/N] ")?;
    out.flush()?;
    let Some(answer) = read_line(input)? else {
        return Ok(false);
    };
    Ok(accepts_install(&answer))
}

fn install_tui() -> io::Result<()> {
    let status = Command::new("bash")
        .arg("-c")
        .arg(TUI_INSTALL_COMMAND)
        .status()?;
    if status.success() {
        Ok(())
    } else {
        Err(io::Error::other(format!("installer exited with {status}")))
    }
}

fn launch_selected(
    home: &Path,
    account: &str,
    dir: &Path,
    args: &[OsString],
    surface: Surface,
) -> io::Result<i32> {
    match surface {
        Surface::Tui => {
            let args = claude_args_for_launch(
                in_herdr(env::var("HERDR_ENV").ok().as_deref()),
                account,
                args,
            );
            if let Some(program) = tui_binary(home) {
                return spawn_claude(&program, dir, &args);
            }
            let stdin = io::stdin();
            let mut input = stdin.lock();
            let mut out = io::stderr();
            if !confirm_tui_install(&mut input, &mut out)? {
                return Ok(0);
            }
            install_tui()?;
            let Some(program) = tui_binary(home) else {
                eprintln!(
                    "claudin: claude is still not available. Open a new terminal and run claudin again."
                );
                return Ok(1);
            };
            spawn_claude(&program, dir, &args)
        }
        Surface::Gui => {
            let Some(program) = gui_binary(home) else {
                eprintln!("{}", gui_missing_message());
                return Ok(1);
            };
            eprintln!("claudin: starting {} in the background", program.display());
            spawn_in_background(&program, dir, args)
        }
    }
}

fn main() {
    let Some(home) = env::var_os("HOME")
        .or_else(|| env::var_os("USERPROFILE"))
        .map(PathBuf::from)
    else {
        eprintln!("claudin: HOME is not set");
        process::exit(1);
    };

    let LaunchRequest {
        window: use_window,
        account: preset,
        claude_args,
    } = match parse_launch_args(env::args_os().skip(1).collect()) {
        Ok(request) => request,
        Err(message) => {
            eprintln!("claudin: {message}");
            process::exit(1);
        }
    };
    if use_window {
        if let Err(err) = gui::run(home, preset, claude_args) {
            eprintln!("claudin: {err}");
            process::exit(1);
        }
        return;
    }
    // UI goes to stderr so claude's stdout stays clean for pipelines.
    let name = if let Some(name) = preset {
        Ok(Some(name))
    } else if io::stdin().is_terminal() && io::stderr().is_terminal() {
        select_account_tty(&home)
    } else {
        let stdin = io::stdin();
        let mut input = stdin.lock();
        let mut out = io::stderr();
        select_account_lines(&home, &mut input, &mut out)
    };
    let name = match name {
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

    eprintln!("Using {CONFIG_ENV}={}", dir.display());
    let surface = if io::stdin().is_terminal() && io::stderr().is_terminal() {
        select_surface_tty()
    } else {
        let stdin = io::stdin();
        let mut input = stdin.lock();
        let mut out = io::stderr();
        select_surface_lines(&mut input, &mut out)
    };
    let surface = match surface {
        Ok(Some(surface)) => surface,
        Ok(None) => return,
        Err(e) => {
            eprintln!("claudin: {e}");
            process::exit(1);
        }
    };
    match launch_selected(&home, &name, &dir, &claude_args, surface) {
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
        assert!(matches!(parse_choice("4", &accounts), Some(Choice::Remove)));
        assert!(matches!(parse_choice("r", &accounts), Some(Choice::Remove)));
        assert!(parse_choice("0", &accounts).is_none());
        assert!(parse_choice("5", &accounts).is_none());
        assert!(parse_choice("x", &accounts).is_none());
    }

    #[test]
    fn arrows_move_the_highlight_and_enter_selects() {
        let entries = menu_entries(&["work".into(), "home".into()]);
        assert!(matches!(&entries[0], Choice::Existing(name) if name == "work"));
        assert!(matches!(entries[2], Choice::New));
        assert!(matches!(entries[3], Choice::Remove));
        assert!(matches!(entries[4], Choice::Quit));

        assert_eq!(step_menu(0, entries.len(), MenuKey::Up), MenuStep::Move(0));
        assert_eq!(
            step_menu(0, entries.len(), MenuKey::Down),
            MenuStep::Move(1)
        );
        assert_eq!(
            step_menu(entries.len() - 1, entries.len(), MenuKey::Down),
            MenuStep::Move(entries.len() - 1)
        );
        assert_eq!(
            step_menu(1, entries.len(), MenuKey::Enter),
            MenuStep::Choose(1)
        );
        assert_eq!(step_menu(0, entries.len(), MenuKey::Quit), MenuStep::Quit);

        let mut cursor = 0;
        for key in [MenuKey::Down, MenuKey::Down, MenuKey::Enter] {
            match step_menu(cursor, entries.len(), key) {
                MenuStep::Move(next) => cursor = next,
                MenuStep::Choose(index) => {
                    assert!(matches!(entries[index], Choice::New));
                    return;
                }
                MenuStep::Quit => panic!("quit"),
            }
        }
        panic!("enter did not choose");
    }

    #[test]
    fn paints_the_highlighted_row() {
        let entries = menu_entries(&["work".into()]);
        let labels: Vec<&str> = entries.iter().map(entry_label).collect();
        let mut out = Vec::new();
        paint_menu(&mut out, "Select a Claude account", &labels, 0).unwrap();
        let text = String::from_utf8(out).unwrap();
        assert!(text.contains("> work"));
        assert!(text.contains("  New account"));
        assert!(text.contains("  Remove account"));
        assert!(text.contains("  Quit"));

        let mut out = Vec::new();
        paint_menu(&mut out, "Select a Claude account", &labels, 3).unwrap();
        let text = String::from_utf8(out).unwrap();
        assert!(text.contains("  work"));
        assert!(text.contains("> Quit"));
    }

    #[test]
    fn decodes_arrows_enter_and_quit_keys() {
        use crossterm::event::{KeyCode, KeyEvent, KeyEventKind, KeyModifiers};

        assert_eq!(
            decode_key(KeyEvent::new(KeyCode::Up, KeyModifiers::NONE)),
            Some(MenuKey::Up)
        );
        assert_eq!(
            decode_key(KeyEvent::new(KeyCode::Down, KeyModifiers::NONE)),
            Some(MenuKey::Down)
        );
        assert_eq!(
            decode_key(KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE)),
            Some(MenuKey::Enter)
        );
        assert_eq!(
            decode_key(KeyEvent::new(KeyCode::Esc, KeyModifiers::NONE)),
            Some(MenuKey::Quit)
        );
        assert_eq!(
            decode_key(KeyEvent::new(KeyCode::Char('q'), KeyModifiers::NONE)),
            Some(MenuKey::Quit)
        );
        assert_eq!(
            decode_key(KeyEvent::new(KeyCode::Char('c'), KeyModifiers::CONTROL)),
            Some(MenuKey::Quit)
        );
        assert_eq!(
            decode_key(KeyEvent::new(KeyCode::Char('1'), KeyModifiers::NONE)),
            None
        );

        let release =
            KeyEvent::new_with_kind(KeyCode::Down, KeyModifiers::NONE, KeyEventKind::Release);
        assert_eq!(decode_key(release), None);
    }

    #[test]
    fn no_accounts_prompts_for_new_name() {
        let home = temp_home("empty");
        let mut input = Cursor::new("bad name\nwork\n");
        let mut out = Vec::new();
        let name = select_account_lines(&home, &mut input, &mut out).unwrap();
        assert_eq!(name.as_deref(), Some("work"));
        assert!(String::from_utf8(out).unwrap().contains("Invalid name"));
        fs::remove_dir_all(&home).unwrap();
    }

    #[test]
    fn selects_existing_or_new_account() {
        let home = temp_home("select");
        fs::create_dir(home.join(".claude_work")).unwrap();

        let mut out = Vec::new();
        let name = select_account_lines(&home, &mut Cursor::new("1\n"), &mut out).unwrap();
        assert_eq!(name.as_deref(), Some("work"));

        let name =
            select_account_lines(&home, &mut Cursor::new("9\n2\nwork\nhome\n"), &mut out).unwrap();
        assert_eq!(name.as_deref(), Some("home"));
        let text = String::from_utf8(out).unwrap();
        assert!(text.contains("Invalid choice"));
        assert!(text.contains("already exists"));

        let name = select_account_lines(&home, &mut Cursor::new("q\n"), &mut Vec::new()).unwrap();
        assert!(name.is_none());
        fs::remove_dir_all(&home).unwrap();
    }

    fn write_file(path: &Path, executable: bool) {
        fs::write(path, b"").unwrap();
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let mut perms = fs::metadata(path).unwrap().permissions();
            perms.set_mode(if executable { 0o755 } else { 0o644 });
            fs::set_permissions(path, perms).unwrap();
        }
        #[cfg(not(unix))]
        {
            let _ = executable;
        }
    }

    #[test]
    fn first_runnable_skips_missing_dirs_and_non_executables() {
        let home = temp_home("bins");
        let missing = home.join("missing");
        let dir = home.join("dir");
        fs::create_dir(&dir).unwrap();
        let plain = home.join("plain");
        write_file(&plain, false);
        let bin = home.join("claude");
        write_file(&bin, true);

        assert!(first_runnable(&[missing.clone(), dir.clone()]).is_none());
        assert_eq!(
            first_runnable(&[missing, dir, bin.clone()]),
            Some(bin.as_path())
        );
        #[cfg(unix)]
        {
            assert!(first_runnable(std::slice::from_ref(&plain)).is_none());
            assert_eq!(first_runnable(&[plain, bin.clone()]), Some(bin.as_path()));
        }
        fs::remove_dir_all(&home).unwrap();
    }

    #[cfg(unix)]
    #[test]
    fn background_spawn_returns_without_waiting() {
        let home = temp_home("detach");
        let marker = home.join("ran");
        let pidfile = home.join("pid");
        let program = home.join("fake-claude");
        let config = home.join("cfg");
        let script = format!(
            "#!/bin/sh\nprintf '%s' \"$CLAUDE_CONFIG_DIR\" > '{}'\necho $$ > '{}'\nsleep 30\n",
            marker.display(),
            pidfile.display()
        );
        fs::write(&program, script).unwrap();
        use std::os::unix::fs::PermissionsExt;
        let mut perms = fs::metadata(&program).unwrap().permissions();
        perms.set_mode(0o755);
        fs::set_permissions(&program, perms).unwrap();

        let started = std::time::Instant::now();
        let code = spawn_in_background(&program, &config, &[]).unwrap();
        assert_eq!(code, 0);
        assert!(started.elapsed() < std::time::Duration::from_secs(2));

        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(3);
        while std::time::Instant::now() < deadline && !(marker.exists() && pidfile.exists()) {
            std::thread::sleep(std::time::Duration::from_millis(20));
        }
        if let Ok(pid) = fs::read_to_string(&pidfile) {
            let _ = Command::new("kill").arg(pid.trim()).status();
        }
        assert_eq!(
            fs::read_to_string(&marker).unwrap_or_default(),
            config.display().to_string()
        );
        fs::remove_dir_all(&home).unwrap();
    }

    #[cfg(target_os = "macos")]
    #[test]
    fn macos_fallback_is_claude_app() {
        assert_eq!(
            fallback_claude_paths(Path::new("/Users/ada")),
            vec![PathBuf::from(
                "/Applications/Claude.app/Contents/MacOS/Claude"
            )]
        );
    }

    #[test]
    fn windows_fallback_uses_local_app_data_then_profile() {
        let home = Path::new(r"C:\Users\ada");
        let from_home = home
            .join("AppData")
            .join("Local")
            .join("Anthropic")
            .join("Claude")
            .join("claude.exe");
        assert_eq!(windows_claude_paths(home, None), vec![from_home.clone()]);

        let local = Path::new(r"D:\AppData\Local");
        assert_eq!(
            windows_claude_paths(home, Some(local)),
            vec![
                local.join("Anthropic").join("Claude").join("claude.exe"),
                from_home,
            ]
        );

        let local = home.join("AppData").join("Local");
        assert_eq!(
            windows_claude_paths(home, Some(&local)),
            vec![local.join("Anthropic").join("Claude").join("claude.exe")]
        );
    }

    #[test]
    fn herdr_start_prompt_is_only_added_for_a_bare_launch() {
        assert!(!in_herdr(None));
        assert!(!in_herdr(Some("0")));
        assert!(in_herdr(Some("1")));

        let prompt = herdr_setup_prompt("work");
        assert!(prompt.contains("work"));
        assert!(prompt.contains("workspace"));
        assert!(prompt.contains("herdr CLI"));

        let launched = claude_args_for_launch(true, "work", &[]);
        assert_eq!(launched, vec![OsString::from(prompt)]);

        let resume = vec![OsString::from("--resume")];
        assert_eq!(claude_args_for_launch(true, "work", &resume), resume);
        assert!(claude_args_for_launch(false, "work", &[]).is_empty());
    }

    #[test]
    fn removes_an_account_after_confirmation() {
        let home = temp_home("remove");
        fs::create_dir_all(home.join(".claude_home")).unwrap();
        fs::create_dir_all(home.join(".claude_work")).unwrap();
        fs::write(home.join(".claude_work").join("session"), "secret").unwrap();

        let mut out = Vec::new();
        let name = select_account_lines(&home, &mut Cursor::new("4\n2\ny\n1\n"), &mut out).unwrap();
        assert_eq!(name.as_deref(), Some("home"));
        assert!(!home.join(".claude_work").exists());
        assert!(home.join(".claude_home").exists());
        assert!(
            String::from_utf8(out)
                .unwrap()
                .contains("Removed account 'work'")
        );

        fs::create_dir(home.join(".claude_work")).unwrap();
        let name =
            select_account_lines(&home, &mut Cursor::new("4\n1\nn\n2\n"), &mut Vec::new()).unwrap();
        assert_eq!(name.as_deref(), Some("work"));
        assert!(home.join(".claude_home").exists());
        fs::remove_dir_all(&home).unwrap();
    }

    #[test]
    fn account_flag_skips_the_menu_and_keeps_claude_args() {
        let mut args = vec![
            OsString::from("--account"),
            OsString::from("work"),
            OsString::from("--resume"),
        ];
        assert_eq!(
            take_account_arg(&mut args).unwrap().as_deref(),
            Some("work")
        );
        assert_eq!(args, vec![OsString::from("--resume")]);

        let mut args = vec![OsString::from("--resume")];
        assert!(take_account_arg(&mut args).unwrap().is_none());
        assert_eq!(args, vec![OsString::from("--resume")]);

        let mut args = vec![OsString::from("--account")];
        assert!(take_account_arg(&mut args).is_err());
        let mut args = vec![OsString::from("--account"), OsString::from("a/b")];
        assert!(take_account_arg(&mut args).is_err());
    }

    #[test]
    fn gui_flag_opens_the_window_and_keeps_the_account() {
        let mut args = vec![OsString::from("--gui")];
        assert!(take_gui_flag(&mut args));
        assert!(args.is_empty());

        let mut args = vec![
            OsString::from("--gui"),
            OsString::from("--account"),
            OsString::from("work"),
            OsString::from("--resume"),
        ];
        assert!(take_gui_flag(&mut args));
        assert_eq!(
            take_account_arg(&mut args).unwrap().as_deref(),
            Some("work")
        );
        assert_eq!(args, vec![OsString::from("--resume")]);

        let mut args = vec![
            OsString::from("--account"),
            OsString::from("work"),
            OsString::from("--gui"),
        ];
        assert!(take_gui_flag(&mut args));
        assert_eq!(
            take_account_arg(&mut args).unwrap().as_deref(),
            Some("work")
        );
        assert!(args.is_empty());

        let mut args = vec![OsString::from("--resume"), OsString::from("--gui")];
        assert!(!take_gui_flag(&mut args));
        assert_eq!(
            args,
            vec![OsString::from("--resume"), OsString::from("--gui")]
        );
    }

    #[test]
    fn double_dash_forwards_the_rest_unchanged() {
        let request = parse_launch_args(vec![
            OsString::from("--"),
            OsString::from("--permission-mode=auto"),
        ])
        .unwrap();
        assert!(!request.window);
        assert!(request.account.is_none());
        assert_eq!(
            request.claude_args,
            vec![OsString::from("--permission-mode=auto")]
        );

        let request = parse_launch_args(vec![
            OsString::from("--account"),
            OsString::from("work"),
            OsString::from("--"),
            OsString::from("--permission-mode=auto"),
        ])
        .unwrap();
        assert_eq!(request.account.as_deref(), Some("work"));
        assert_eq!(
            request.claude_args,
            vec![OsString::from("--permission-mode=auto")]
        );

        let request = parse_launch_args(vec![
            OsString::from("--gui"),
            OsString::from("--resume"),
            OsString::from("--"),
            OsString::from("--permission-mode=auto"),
        ])
        .unwrap();
        assert!(request.window);
        assert_eq!(
            request.claude_args,
            vec![
                OsString::from("--resume"),
                OsString::from("--permission-mode=auto")
            ]
        );

        let request = parse_launch_args(vec![
            OsString::from("--"),
            OsString::from("--account"),
            OsString::from("work"),
            OsString::from("--gui"),
        ])
        .unwrap();
        assert!(!request.window);
        assert!(request.account.is_none());
        assert_eq!(
            request.claude_args,
            vec![
                OsString::from("--account"),
                OsString::from("work"),
                OsString::from("--gui"),
            ]
        );
    }

    #[test]
    fn install_offer_matches_yes_and_explains_both_apps() {
        assert!(accepts_install("y"));
        assert!(accepts_install(" YES "));
        assert!(!accepts_install(""));
        assert!(!accepts_install("n"));
        assert!(tui_missing_message().contains(TUI_INSTALL_COMMAND));
        assert!(gui_missing_message().contains(GUI_INSTALL_URL));
    }

    #[test]
    fn line_menu_chooses_terminal_or_desktop() {
        let mut out = Vec::new();
        let surface = select_surface_lines(&mut Cursor::new("9\n1\n"), &mut out).unwrap();
        assert_eq!(surface, Some(Surface::Tui));
        assert!(String::from_utf8(out).unwrap().contains("Invalid choice"));

        let surface = select_surface_lines(&mut Cursor::new("2\n"), &mut Vec::new()).unwrap();
        assert_eq!(surface, Some(Surface::Gui));
        let surface = select_surface_lines(&mut Cursor::new("q\n"), &mut Vec::new()).unwrap();
        assert!(surface.is_none());
    }
}
