# Enable persistent history
HISTFILE=~/.zsh_history
HISTSIZE=100000
SAVEHIST=100000

setopt HIST_SAVE_NO_DUPS
setopt INC_APPEND_HISTORY

# Configure the push directory stack (most people don't need this)
setopt AUTO_PUSHD
setopt PUSHD_IGNORE_DUPS
setopt PUSHD_SILENT

# Emacs keybindings
bindkey -e
# Use the up and down keys to navigate the history
bindkey "\e[A" history-beginning-search-backward
bindkey "\e[B" history-beginning-search-forward

# Move to directories without cd
setopt autocd

# The most important aliases ever (the only thing I borrowed from OMZ)
alias l='ls -lah'
alias la='ls -lAh'
alias ll='ls -lh'
alias ls='ls -G'
alias lsa='ls -lah'

# Set up fzf key bindings and fuzzy completion
# If ~/.fzf.zsh exists, source it, otherwise use the dynamic command.
if [ -f ~/.fzf.zsh ]; then
  source ~/.fzf.zsh
else
  source <(fzf --zsh)
fi

# Set up zoxide to move between folders efficiently
eval "$(zoxide init zsh)"

# Set up the Starship prompt
eval "$(starship init zsh)"

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

# Add ~/bin to PATH
export PATH="$HOME/bin:$PATH"

# Source external configuration
source ~/dotfile/.zshrc_git_ext
[ -f ~/.zshrc_ext ] && source ~/.zshrc_ext

# --- Completion System ---
# Enable autocompletion for brew installed software
if type brew &>/dev/null
then
  FPATH="$(brew --prefix)/share/zsh/site-functions:${FPATH}"
fi

# Initialize bash completion compatibility first
autoload -U +X bashcompinit && bashcompinit

# Initialize zsh completion system
autoload -U compinit; compinit

### Custom configs
export GOOGLE_CLOUD_PROJECT="rstagi"

# Source zsh-syntax-highlighting
source $(brew --prefix)/share/zsh-syntax-highlighting/zsh-syntax-highlighting.zsh

export PATH="/Users/rstagi/.local/bin:$PATH"
export PATH="/opt/homebrew/opt/libpq/bin:$PATH"

# OpenClaw Completion
source <(openclaw completion --shell zsh)

# pnpm
export PNPM_HOME="/Users/rstagi/Library/pnpm"
case ":$PATH:" in
  *":$PNPM_HOME:"*) ;;
  *) export PATH="$PNPM_HOME:$PATH" ;;
esac
# pnpm end
