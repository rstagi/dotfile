
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
)
[ -f ~/.zshrc_additional_plugins_list ] && source ~/.zshrc_additional_plugins_list
export DISABLE_MAGIC_FUNCTIONS=true
source $HOME/.oh-my-zsh/oh-my-zsh.sh

# Key bindings
bindkey "[D" backward-word
bindkey "[C" forward-word
bindkey '^[^?' backward-kill-word

# Aliases
alias tmux="tmux -u"
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
[ -f ~/.fzf.zsh ] && source ~/.fzf.zsh
[ -f ~/.zshrc_ext ] && source ~/.zshrc_ext

autoload -U +X bashcompinit && bashcompinit

# Enable autocompletion for brew installed software
if type brew &>/dev/null
then
  FPATH="$(brew --prefix)/share/zsh/site-functions:${FPATH}"

  autoload -Uz compinit
  compinit
fi

