# claudin

A small CLI wrapper for [Claude Code](https://docs.anthropic.com/en/docs/claude-code) that lets you switch between multiple Claude accounts.

Each account uses its own config directory, `$HOME/.claude_<project_name>`. `claudin` shows a menu of the accounts it finds, then starts `claude` with `CLAUDE_CONFIG_DIR` set to the account you picked.

## Install

```sh
cargo install --path .
```

## Usage

```sh
claudin [claude args...]
```

- Pick an existing account by its number. `claudin` then runs `claude` with `CLAUDE_CONFIG_DIR="$HOME/.claude_<name>"`.
- Pick **New account** (or type `n`) and enter a project name to create a new account. If you don't have any accounts yet, `claudin` goes straight to this step. The first time `claude` runs for the new account, it will show you how to log in.
- Type `q` to quit.

Names can contain letters, digits, `-`, `_` and `.`.

Any arguments you give to `claudin` are passed through to `claude`, for example `claudin --resume`.
