# claudin

A small CLI wrapper for [Claude Code](https://docs.anthropic.com/en/docs/claude-code) that lets you switch between multiple Claude accounts.

Each account uses its own config directory, `$HOME/.claude_<project_name>`. `claudin` shows a menu of the accounts it finds, then starts `claude` with `CLAUDE_CONFIG_DIR` set to the account you picked.

## Install

```sh
cargo install --path .
```

## Usage

```sh
claudin [--account name] [claude args...]
```

- In a terminal, move with the up and down arrows and press Enter. Then choose **Terminal** or **Desktop app**. `claudin` runs that Claude with `CLAUDE_CONFIG_DIR="$HOME/.claude_<name>"`.
- If the terminal app is missing, claudin offers to install it with `curl -fsSL https://claude.ai/install.sh | bash`.
- If the desktop app is missing, install it manually from <https://claude.com/product/claude-code>.
- Choose **New account** and enter a project name to create a new account. If you don't have any accounts yet, `claudin` goes straight to this step. The first time `claude` runs for the new account, it will show you how to log in.
- Choose **Remove account**, pick the account, and confirm to delete its config directory. In the numbered menu, `r` does the same.
- Choose **Quit**, or press `q` or Esc, to leave without starting `claude`.
- When input is not a terminal, type the account number instead. `n` creates an account and `q` quits.

Names can contain letters, digits, `-`, `_` and `.`.

Any other arguments are passed through to `claude`, for example `claudin --resume`. `claudin --account work` skips the menu and opens that account.

Inside [Herdr](https://herdr.dev) (`HERDR_ENV=1`), a bare terminal launch starts Claude with a prompt asking it to configure Herdr for that account: name the workspace so it mentions the account, and match the tab, pane, and notification. Arguments you pass through, such as `claudin --resume`, are left unchanged.
