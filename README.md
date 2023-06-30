# Dotfile

## Warning

The following guide has been exported from a Notion page and thus still needs to be translated in english.
Some part might be redundant with the code, or might not be updated.

## XCode (solo per build iOS)

- Installare XCode da AppStore
- Installare i tool da linea di comando con `xcode-select --install`

---

## Homebrew

Per installarlo:

```bash
/bin/bash -c "$(curl -fsSL https://raw.githubusercontent.com/Homebrew/install/master/install.sh)"
```

### Quicklook plugins (???)

```bash
brew install qlcolorcode qlstephen qlmarkdown quicklook-json qlimagesize suspicious-package apparency quicklookase qlvideo
xattr -cr ~/Library/QuickLook/*.qlgenerator
```

### Altri software utili

```bash
brew install --cask \
		aldente \
    appcleaner \
    bitwarden \  
    cheatsheet \
    google-chrome \
    flux \
    rectangle \ 
    ripgrep \
		spotify \
    transmission \
    vlc
```

---

## RunCat

Da AppStore (purtroppo).

---

## ITerm2

```bash
brew install --cask iterm2
```

### Zsh

```bash
brew install zsh
```

### Oh My Zsh

Istruzioni su https://github.com/ohmyzsh/ohmyzsh

```bash
sh -c "$(curl -fsSL https://raw.githubusercontent.com/ohmyzsh/ohmyzsh/master/tools/install.sh)"
```

Now, we'll install some plugins and setup some new tools. To do so, run the following command to enable the `zsh-syntax-highlighting` plugin:
```bash
git clone https://github.com/zsh-users/zsh-syntax-highlighting.git $ZSH_CUSTOM/plugins/zsh-syntax-highlighting
```

And the following one to install some tools we use:

```bash
brew install tree ack fzf shellcheck trash-cli
$(brew --prefix)/opt/fzf/install
```

After that, everyhing else you might need for oh-my-zsh and other tools is already present in an `.zshrc` extension in this repository! So make sure to clone this repository in your home:
```bash
git clone git@github.com:rstagi/dotfile.git ~/dotfile
```

And make your `.zshrc` look like the following:
```
export ZSH="$HOME/.oh-my-zsh"
source ~/dotfile/.zshrc-ext
```

Such extension contains the following:
- oh-my-zsh plugins and themes
- fzf configuration
- useful aliases for tools like git, npm, yarn, docker and others!

## Git

```bash
brew install git
git config --global user.name "rstagi"
git config --global user.email "r.stagi96@gmail.com"
```

Per usare il clone via HTTPS, ed evitare il prompt di username e password ogni volta, lanciare il seguente comando:

```bash
git config --global credential.helper osxkeychain
```

---

## Vim

```bash
brew install vim
```

Per installare ****Ultimate vimrc****, che è un set di plugin e configurazioni per vim, lanciare:

```bash
git clone https://github.com/amix/vimrc.git ~/.vim_runtime
sh ~/.vim_runtime/install_awesome_vimrc.sh
```

Infine, per installare ******************************Maximum awesome******************************, che è un plugin manager per vim, lanciare in punto definito:

```bash
git clone https://github.com/square/maximum-awesome.git
cd maximum-awesome
rake
```

---

## Visual Studio Code

```bash
brew install --cask visual-studio-code
```

Poi:

- abilitare l’integrazione con la command line: [https://code.visualstudio.com/docs/setup/mac#_launching-from-the-command-line](https://code.visualstudio.com/docs/setup/mac#_launching-from-the-command-line)
- fare accesso per il sync con il mio account.

---

## Tmux

To install it run the following:

```bash
brew install tmux
```

Then, intstall the Tmux Package Manager (TPM) by cloning its repository:

```
git clone https://github.com/tmux-plugins/tpm ~/.tmux/plugins/tpm
```

And finally source the `.tmux.conf` extension that is present in this repository! You can do so by creating a new file `~/.tmux.conf` and write the following line:
```
source-file ~/dotfile/.tmux-ext.conf
```

---

## Raycast

```bash
brew install raycast  
```

- Remove Spotlight shortcut `CMD + space` from Keyboard Settings > Keyboard Shortcuts > Spotlight
- Set Raycast Hotkey to `CMD + space` in Raycast Settings > General

### Extensions list

- **Brew**
    - **Search** Hotkey: `Option + B`
- **Clipboard History**
- **Code Stash**
- **Coffee**
- **Define Word**
- **Floating Notes**
    - **Toggle Floating Notes Focus** Hotkey: `Option + .`
- **************Format JSON**************
- **GitHub**
- **Google Search**
- **Google Translate**
- **Google Workspace**
- **Jira**
- **Navigation**
- **Search Emoji**
- **Search Project Manager**
- **Set Audio Device**
- **Show Cheatsheets**
- **Snippets**
- **Speedtest**
- **Window Management**
- **iTerm**
- **Notion**
---
