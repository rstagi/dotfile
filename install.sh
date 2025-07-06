#!/bin/bash

# Check if brew is already installed
if ! command -v brew &> /dev/null
then
  echo "brew could not be found"
  echo "Installing brew..."
  /bin/bash -c "$(curl -fsSL https://raw.githubusercontent.com/Homebrew/install/HEAD/install.sh)"
  echo >> $HOME/.zprofile
  echo 'eval "$(/opt/homebrew/bin/brew shellenv)"' >> $HOME/.zprofile
  eval "$(/opt/homebrew/bin/brew shellenv)"
else
  echo "brew is already installed"
  brew update && brew upgrade
fi

# Install necessary dependencies
echo "Installing dependencies..."
brew install openssl readline sqlite3 xz zlib graphviz jq git tree ack fzf shellcheck trash-cli font-jetbrains-mono-nerd-font

# Cloning dotfile repo
if [ -d "$HOME/dotfile" ]; then
  echo "dotfile repo already exists"
else
  echo "Cloning dotfile repo..."
  git clone https://github.com/rstagi/dotfile.git $HOME/dotfile
fi

# Configuring git
link_gitconfig() {
  [ -f "$HOME/.gitconfig" ] && mv $HOME/.gitconfig $HOME/.gitconfig.bak
  ln -s $HOME/dotfile/.gitconfig $HOME/.gitconfig
}
read -p "Do you want to configure git with the proper dotfile? (y/n) " choice
case "$choice" in
  y|Y|yes|YES ) link_gitconfig;;
  * ) echo "ok, skipping git configuration";;
esac


is_already_installed() {
  brew list | grep ^$1$ > /dev/null
}

install_pkg() {
  brew update && brew install $2 $1
}

read_yes() {
  read -p "$1 (y/n) " choice
  case "$choice" in
    y|Y|yes|YES ) return 0;;
    * ) return 1;;
  esac
}

install_pkg_if_needed() {
  if is_already_installed $1; then
    echo "$1 is already installed"
    return 0
  else
    if read_yes "$1 is not installed. Do you want to install it?"; then
      install_pkg $1 $2
      return 0
    else
      return 1
    fi
  fi
}

## Important packages ##
install_pkg_if_needed "arc" "--cask"
install_pkg_if_needed "warp" "--cask"
install_pkg_if_needed "cursor" "--cask"
install_pkg_if_needed "rectangle" "--cask"

if read_yes "Important packages have been installed. Would you like to continue with the optional stuff?"; then
  echo "Alright, let's go!"
else
  exit
fi

## Optional stuff"

# Configure fzf
if read_yes "Do you want to setup fzf?"; then
  $(brew --prefix)/opt/fzf/install
fi

# Install zsh
install_zsh() {
  brew update && brew install zsh && chsh zsh
}
install_pkg_if_needed "zsh"

# Install python with pyenv
configure_python() {
  brew update && brew upgrade pyenv
  echo "source $HOME/dotfile/.zshrc_python_ext" >> $HOME/.zshrc_ext
  export PYENV_ROOT="$HOME/.pyenv"
  command -v pyenv >/dev/null || export PATH="$PYENV_ROOT/bin:$PATH"
  eval "$(pyenv init -)"
  CFLAGS="-I$(brew --prefix xz)/include -I$(brew --prefix openssl)/include" LDFLAGS="-L$(brew --prefix xz)/lib -L$(brew --prefix openssl)/lib" pyenv install -s 3
}
if install_pkg_if_needed "pyenv" && read_yes "Do you want to configure python?"; then
  install_pkg_if_needed "pipx"
  configure_python
fi

# Install oh-my-zsh
install_oh_my_zsh() {
  sh -c "$(curl -fsSL https://raw.githubusercontent.com/ohmyzsh/ohmyzsh/master/tools/install.sh)"
  brew install qlcolorcode qlstephen qlmarkdown quicklook-json suspicious-package apparency quicklookase qlvideo
  xattr -cr ~/Library/QuickLook/*.qlgenerator
  pipx install pygments
}
install_zsh_syntax_highlighting() {
  git clone https://github.com/zsh-users/zsh-syntax-highlighting.git $ZSH_CUSTOM/plugins/zsh-syntax-highlighting
}
link_zshrc() {
  [ -f "$HOME/.zshrc" ] && mv $HOME/.zshrc $HOME/.zshrc.bak
  ln -s $HOME/dotfile/.zshrc $HOME/.zshrc
}
if [ -d "$HOME/.oh-my-zsh" ]; then
  echo "oh-my-zsh is already installed"
else
  if ! read_yes "oh-my-zsh is not installed, but it's required. Do you want to install it?"; then
    exit
  fi
  install_oh_my_zsh
fi
if [ -f "$HOME/.zshrc" ]; then
  if read_yes ".zshrc already exists. Do you want to override it?"; then
    if [ -d "$HOME/.oh-my-zsh/custom/plugins/zsh-syntax-highlighting" ]; then
      echo "zsh-syntax-highlighting is already installed"
    else
      echo "zsh-syntax-highlighting is not installed. Installing it..."
      install_zsh_syntax_highlighting
    fi
    link_zshrc
  else
    echo "ok, skipping .zshrc override"
  fi
else
  link_zshrc
fi

# Install gh
configure_gh() {
  TARGET_PATH="$(brew --prefix)/share/zsh/site-functions"
  [ ! -f "$TARGET_PATH/_gh" ] && mkdir -p $TARGET_PATH && gh completion -s zsh > "$TARGET_PATH/_gh" && chmod +x "$TARGET_PATH/_gh"
}
if install_pkg_if_needed "gh" && read_yes "Do you want to configure gh?"; then
  configure_gh
fi

# Install nvm and node
install_node() {
  nvm install --latest 
  echo "plugins+=(npm)" >> $HOME/.zshrc_additional_plugins
  echo "plugins+=(node)" >> $HOME/.zshrc_additional_plugins
  echo "source $HOME/dotfile/.zshrc_node_ext" >> $HOME/.zshrc_ext
  npm install --global yarn
}
if install_pkg_if_needed "nvm" && read_yes "Do you want to install node?"; then
  install_node
fi

# Install terraform
configure_terraform() {
  tfenv install latest
  terraform -install-autocomplete
  echo "source $HOME/dotfile/.zshrc_terraform_ext" >> $HOME/.zshrc_ext
}
if install_pkg_if_needed "tfenv" && read_yes "Do you want to configure terraform?"; then
  configure_terraform
fi

# Install gcloud
configure_gcloud() {
  echo "plugins+=(gcloud)" >> $HOME/.zshrc_additional_plugins
  echo "source $HOME/dotfile/.zshrc_gcloud_ext" >> $HOME/.zshrc_ext
  gcloud auth login --update-adc --enable-gdrive-access
  gcloud auth application-default login
  gcloud_installed=true
}
if install_pkg_if_needed "google-cloud-sdk" && read_yes "Do you want to configure gcloud?"; then
  install_gcloud
fi

# Install kubectl
configure_kubectl() {
  brew update && brew install krew
  kubectl krew install ctx
  kubectl krew install ns
  echo "source $HOME/dotfile/.zshrc_k8s_ext" >> $HOME/.zshrc_ext
}
if install_pkg_if_needed "kubernetes-cli" && read_yes "Do you want to configure kubectl?"; then
  configure_kubectl
fi

# Install helm
install_pkg_if_needed "helm"

# Install docker
configure_docker() {
  # Install docker-buildx
  brew install docker-buildx
  mkdir -p ~/.docker/cli-plugins
  ln -sfn $(which docker-buildx) ~/.docker/cli-plugins/docker-buildx

  # Extend zshrc
  echo "source $HOME/dotfile/.zshrc_docker_ext" >> $HOME/.zshrc_ext

  if [ "$gcloud_installed" = "true" ]; then
    gcloud auth configure-docker
  fi
}
if install_pkg_if_needed "docker" "--cask" && read_yes "Do you want to configure docker?"; then
  configure_docker
fi

# Install tmux
configure_tmux() {
  git clone https://github.com/tmux-plugins/tpm ~/.tmux/plugins/tpm
  ln -s $HOME/dotfile/.tmux.conf $HOME/.tmux.conf
}
if install_pkg_if_needed "tmux" && read_yes "Do you want to configure tmux?"; then
  configure_tmux
fi

# Install neovim
configure_neovim() {
  echo "source $HOME/dotfile/.zshrc_vim_ext" >> $HOME/.zshrc_ext
}
if install_pkg_if_needed "neovim" && read_yes "Do you want to configure neovim?"; then
  configure_neovim
fi

# Install raycast
install_pkg_if_needed "raycast" "--cask"

# Install ghostty
install_pkg_if_needed "ghostty"

# Install Slack
install_pkg_if_needed "slack" "--cask"

# Install 1password
configure_1password() {
  brew update && brew install --cask 1password/tap/1password-cli
}
if install_pkg_if_needed "1password" && read_yes "Do you want to configure 1password?"; then
  configure_1password
fi

# Install appcleaner
install_pkg_if_needed "appcleaner"

# Install Google Chrome
install_pkg_if_needed "google-chrome"

# Install ripgrep
install_pkg_if_needed "ripgrep"

# Install workon
configure_workon() {
  mkdir -p $HOME/bin
  ln -s $HOME/dotfile/workon.sh $HOME/bin/workon
  chmod +x $HOME/dotfile/workon.sh
}
if read_yes "Do you want to install workon.sh?"; then
  configure_workon
fi

echo "Done!"
