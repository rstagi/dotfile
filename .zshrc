
ZSH_THEME="robbyrussell"
plugins=(
    git
    macos
    gitignore
    common-aliases
    emoji
    extract
    z
    zsh-syntax-highlighting
    jsontools

    # # Docker
    # docker-compose
    # docker
)
[ -f ~/.zshrc_additional_plugins_list ] && source ~/.zshrc_additional_plugins_list
source $ZSH/oh-my-zsh.sh

autoload -U +X bashcompinit && bashcompinit

# Enable autocompletion for brew installed software
if type brew &>/dev/null
then
  FPATH="$(brew --prefix)/share/zsh/site-functions:${FPATH}"

  autoload -Uz compinit
  compinit
fi

# Aliases
alias shcheck="shellcheck"
alias traceon="set -x"
alias traceoff="set +x"
alias usage="du -h -d1"
alias runport="lsof -i"
alias srczsh="source ~/.zshrc"
alias vizsh="vim ~/.zshrc"
alias ..="cd .."
alias ...="cd ../.."

source ~/dotfile/.zshrc_git_ext

# Fzf 
[ -f ~/.fzf.zsh ] && source ~/.fzf.zsh

# source ~/dotfile/.zshrc_docker_ext
