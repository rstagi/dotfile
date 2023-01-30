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

Plugins in `.zshrc`:

```bash
plugins=(
     git
     macos
     npm
     node
     jsontools
     docker-compose
     docker
     common-aliases
     dotenv
     emoji
     extract
     gcloud
     gitignore
     z
     zsh-syntax-highlighting
 )
```

Per abilitare zsh-syntax-highlighting devi anche lanciare il seguente comando:

```bash
git clone https://github.com/zsh-users/zsh-syntax-highlighting.git $ZSH_CUSTOM/plugins/zsh-syntax-highlighting
```

### Altri tool

```bash
brew install tree ack fzf shellcheck trash-cli
$(brew --prefix)/opt/fzf/install
```

### Useful aliases & functions

Aggiornata January 30, 2023 

```bash
# Git aliases
alias glog="git log --color --graph --pretty=format:'%Cred%h%Creset -%C(yellow)%d%Creset %s %Cgreen(%cr) %C(bold blue)<%an>%Creset' --abbrev-commit --branches"
alias gac="git add . && git commit -a -m"
alias gam="git commit --amend"
alias gsc="gac '' --allow-empty-message"
alias gscp="gac '' --allow-empty-message && git push"
alias gpf="git push --force-with-lease"
alias gsw="git switch"
alias gswc="git switch --create"
alias gst="git status"
alias gpsu="git push --set-upstream origin"

function gacp() { gac "$@" && git push }

alias gfetch="git fetch origin"
alias greb="git rebase"
alias grebc="git rebase --continue"

alias gbparent='git show-branch \
    | sed -E "s/([\^~]+.+)*].*//" \
    | grep "\*" \
    | grep -v "$(git rev-parse --abbrev-ref HEAD)" \
    | head -n1 \
    | sed "s/^.*\[//"'

alias gsq="git reset --soft \$(git merge-base HEAD \$(gbparent)) && git commit"
alias gremreb="gfetch \$(gbparent) && greb \$(gbparent)"
alias gsqreb="gfetch \$(gbparent) && gsq && greb \$(gbparent)"

alias gresempty="git reset --soft \$(git log -1 --grep='.' --pretty=format:'%h')"
alias gsempty="gresempty && gsc"
alias gamempty="gresempty && gam"

# Open the Pull Request URL for your current directory's branch (base branch defaults to main)
alias giturl="git remote -v | awk '/fetch/{print \$2}' | sed -Ee 's#(git@|git://)#https://#' -e 's@com:@com/@' -e 's%\.git$%%' | awk '/github/'"
function gitprs() {
  github_url=`giturl`;
  prs_page=$github_url"/pulls"
  open $prs_page
}
function openpr() {
  github_url=`giturl`;
  branch_name=`git symbolic-ref HEAD | cut -d"/" -f 3,4`;
  pr_url=$github_url"/compare/$(gbparent)..."$branch_name
  open $pr_url;
}

# Run git push and then immediately open the Pull Request URL
function gpr() {
  git push origin HEAD
 
  if [ $? -eq 0 ]; then
    openpr
  else
    echo 'failed to push commits and open a pull request.';
  fi
}

# Debug shell scripts
alias shcheck="shellcheck"
alias traceon="set -x"
alias traceoff="set +x"

# Npm
alias ni="npm i"
alias nid="npm i -D"
alias nr="npm run"
alias nrs="npm run start"
alias nver="npm version"

# Yarn
alias ya="yarn add"
alias yad="yarn add --dev"
alias yr="yarn run"
alias yrs="yarn start"
alias yver="yarn version"

# Docker
alias dockup="docker-compose up -d"
alias dockdown="docker-compose down"

# Other aliases
alias usage="du -h -d1"
alias runport="lsof -i"
alias srczsh="source ~/.zshrc"
alias vizsh="vim ~/.zshrc"
alias ..="cd .."
alias ...="cd ../.."
```

In ogni caso, dare un’occhiata [al repository su GitHub](https://github.com/rstagi/dotfile) dedicato a queste configurazioni, che dovrebbe già comprendere tutte quelle di qui sopra. Ci sono sia degli script per configurare tutto in automatico, sia l’estensione del `.zshrc` con tutti gli alias e le function di sopra (aggiornate).

---

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

```bash
brew install tmux
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

---
