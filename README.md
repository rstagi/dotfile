# Dotfile

## Automatic install

Run the following command to install everything:
```bash
/bin/bash -c "$(curl -fsSL https://raw.githubusercontent.com/rstagi/dotfile/master/install.sh)"
```

## Manual configurations

There are some packages to be configured manually.

### .zshrc extension

The `.zshrc` file after the installation checks if a `~/.zshrc_ext` file exists and, if so, it sources it. This is useful to add some custom configurations that are not present in the main file.

### Raycast

Replace the Spotlight shortcut with Raycast:
- Remove Spotlight shortcut `CMD + space` from Keyboard Settings > Keyboard Shortcuts > Spotlight
- Set Raycast Hotkey to `CMD + space` in Raycast Settings > General

### Extensions list
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

### Visual Studio Code

Enable the cli integration following the instructions at [https://code.visualstudio.com/docs/setup/mac#_launching-from-the-command-line](https://code.visualstudio.com/docs/setup/mac#_launching-from-the-command-line):
- `Cmd + Shift + P`
- Type `shell command`
- Select `Shell Command: Install 'code' command in PATH`

Then login with my GitHub account and sync everything else.

### 1Password

Login with my account (or accounts) and sync everything. Then, configure the SSH Agent to use 1Password as a source for SSH keys. (TODO: explain how)

Finally, configure the browser extension to use the 1Password app instead of the web interface.

### Obsidian

Clone the `obsidian-work` repository in the `~/Documents` folder:
```
cd ~/Documents
git clone git@github.com:rstagi/obsidian-work.git
```

Then run Obsidian and select the open an existing folder as workspace option, selecting the `~/Documents/obsidian-work` folder.


### iTerm2
TODO: ???


### RunCat

This package needs to be installed manually from the AppStore.

---

# Deprecated documentation

## NeoVim

TODO.

DEPRECATED:
```bash
brew install vim
```

Per installare ****Ultimate vimrc****, che è un set di plugin e configurazioni per vim, lanciare:

```bash
git clone https://github.com/amix/vimrc.git ~/.vim_runtime
sh ~/.vim_runtime/install_awesome_vimrc.sh
```

Infine, per installare **Maximum awesome**, che è un plugin manager per vim, lanciare in punto definito:

```bash
git clone https://github.com/square/maximum-awesome.git
cd maximum-awesome
rake
```

## Tmux

TODO: review

To install it run the following:

```bash
brew install tmux
```

Then, install the Tmux Package Manager (TPM) by cloning its repository:

```
git clone https://github.com/tmux-plugins/tpm ~/.tmux/plugins/tpm
```

And finally source the `.tmux.conf` extension that is present in this repository! You can do so by creating a new file `~/.tmux.conf` and write the following line:
```
source-file ~/dotfile/.tmux-ext.conf
```

