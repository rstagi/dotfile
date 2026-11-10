# Dotfile

## Automatic install

Run the following command to install everything:
```bash
/bin/bash -c "$(curl -fsSL https://raw.githubusercontent.com/rstagi/dotfile/master/install.sh)"
```

You can run the installer again. A second run changes no file. Before the installer links a dotfile config (`.gitconfig`, `.zshrc`, `~/.config/nvim`, Claude and Codex config), it moves a different file or folder to a new `<path>.bak.<timestamp>` backup. For other links (`ws`, `docker-buildx`, `.tmux.conf`), it keeps a different file and prints a warning.

## ws — terminal workspaces

Bare-minimum Conductor in tmux: a git worktree + tmux session per branch, one tab per Claude/Codex/Grok session, hook-driven status, notifications, and a session bus (`ws send`, `ws done`, `ws on`). `./install.sh ws`, then `ws`. Docs: [ws/README.md](ws/README.md).

## Agent checkpoints

Invoke `/checkpoint-30` in Claude or `$checkpoint-30` in Codex to request a status report and guidance after 30 minutes on one interactive task. Interactive timing is advisory.

## Loop Do

Invoke `/loop-do <request>` in Claude or `$loop-do <request>` in Codex. The skill plans the request in chat, does the work, checks the plan against the code, opens one draft PR with proof and reviews it. It uses no scripts.

## Manual configurations

There are some packages to be configured manually.

### .zshrc extensions

The `.zshrc` file after the installation checks if a `~/.zshrc_ext` file exists and, if so, it sources it. This is useful to add some custom configurations that are not present in the main file.

### Raycast

Replace the Spotlight shortcut with Raycast:
- Remove Spotlight shortcut `CMD + space` from Keyboard Settings > Keyboard Shortcuts > Spotlight
- Set Raycast Hotkey to `CMD + space` in Raycast Settings > General

#### Extensions list
TODO: add the missing hot keys
- **Brew**
    - **Search** Hotkey: `Option + B`
- **Clipboard History**
- **Code Stash**
- **Coffee**
- **Color picker**
- **Define Word**
- **Floating Notes**
    - **Toggle Floating Notes Focus** Hotkey: `Option + .`
- **Format JSON**
- **GitHub**
- **Google Search**
- **Google Translate**
- **Google Workspace**
- **My Password** (1password)
- **Navigation**
- **Notion**
- **Search Emoji**
- **Set Audio Device**
- **Show Cheatsheets**
- **Snippets**
- **Speedtest**
- **Window Management**
- **iTerm**

### 1Password

Login with my account (or accounts) and sync everything. Then, configure the SSH Agent to use 1Password as a source for SSH keys. (TODO: explain how)

Finally, configure the browser extension to use the 1Password app instead of the web interface.

### RunCat

This package needs to be installed manually from the AppStore.

### Launch Apps at startup

Go to `Settings > General > Login items` and set the following apps to start when logging in:
- Raycast
- Rectangle
- RunCat
