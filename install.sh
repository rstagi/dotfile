#!/bin/bash

packages_to_be_configured=()

# Check if brew is already installed
if ! command -v brew &> /dev/null
then
  echo "brew could not be found"
  echo "Installing brew..."
  /bin/bash -c "$(curl -fsSL https://raw.githubusercontent.com/Homebrew/install/HEAD/install.sh)"
else
  echo "brew is already installed"
  brew update && brew upgrade
fi

# Install necessary dependencies
echo "Installing dependencies..."
brew install openssl readline sqlite3 xz zlib graphviz jq git tree ack fzf shellcheck trash-cli
$(brew --prefix)/opt/fzf/install
softwareupdate --install-rosetta

# Cloning dotfile repo
if [ -d "$HOME/dotfile" ]; then
  echo "dotfile repo already exists"
else
  echo "Cloning dotfile repo..."
  git clone github.com:rstagi/dotfile.git $HOME/dotfile
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

# Install zsh
install_zsh() {
  brew update && brew install zsh && chsh zsh
}
if is_already_installed "zsh"; then
  echo "zsh is already installed"
else
  read -p "zsh is not installed but it's required. Do you want to install it? (y/n) " choice
  case "$choice" in
    y|Y|yes|YES ) install_zsh;;
    * ) echo "Exiting" && exit;;
  esac
fi

# Install python with pyenv
install_pyenv() {
  brew update && brew install pyenv
  echo "source $HOME/dotfile/.zshrc_python_ext" >> $HOME/.zshrc_ext
}
upgrade_pyenv() {
  brew update && brew upgrade pyenv
}
install_python3() {
  export PYENV_ROOT="$HOME/.pyenv"
  command -v pyenv >/dev/null || export PATH="$PYENV_ROOT/bin:$PATH"
  eval "$(pyenv init -)"
  CFLAGS="-I$(brew --prefix xz)/include -I$(brew --prefix openssl)/include" LDFLAGS="-L$(brew --prefix xz)/lib -L$(brew --prefix openssl)/lib" pyenv install -s 3
}
if is_already_installed "pyenv"; then
  echo "pyenv is already installed"
else
  read -p "pyenv is not installed but it's required. Do you want to install it? (y/n) " choice
  case "$choice" in
    y|Y|yes|YES )
      install_pyenv  
    ;;
    * ) echo "Exiting" && exit;;
  esac
fi
install_python3

# Install oh-my-zsh
install_oh_my_zsh() {
  sh -c "$(curl -fsSL https://raw.githubusercontent.com/ohmyzsh/ohmyzsh/master/tools/install.sh)"
  brew install qlcolorcode qlstephen qlmarkdown quicklook-json qlimagesize suspicious-package apparency quicklookase qlvideo
  xattr -cr ~/Library/QuickLook/*.qlgenerator
  pip3 install pygments
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
  read -p "oh-my-zsh is not installed, but it's required. Do you want to install it? (y/n) " choice
  case "$choice" in
    y|Y|yes|YES )
      install_oh_my_zsh
    ;;
    * ) echo "Exiting" && exit;;
  esac
fi
if [ -f "$HOME/.zshrc" ]; then
  echo ".zshrc already exists"
  read -p "Do you want to override it? (y/n) " choice
  case "$choice" in
    y|Y|yes|YES )
      
      if [ -d "$HOME/.oh-my-zsh/custom/plugins/zsh-syntax-highlighting" ]; then
        echo "zsh-syntax-highlighting is already installed"
      else
        echo "zsh-syntax-highlighting is not installed. Installing it..."
        install_zsh_syntax_highlighting
      fi
      link_zshrc
    ;;
    * ) echo "ok, skipping .zshrc override";;
  esac
else
  link_zshrc
fi

# Install gh
install_gh() {
  brew update && brew install gh
  TARGET_PATH="$(brew --prefix)/share/zsh/site-functions"
  [ ! -f "$TARGET_PATH/_gh" ] && mkdir -p $TARGET_PATH && gh completion -s zsh > "$TARGET_PATH/_gh" && chmod +x "$TARGET_PATH/_gh"
  packages_to_be_configured+=("gh")
}
if is_already_installed "gh"; then
  echo "gh is already installed"
else
  read -p "gh is not installed. Do you want to install it? (y/n) " choice
  case "$choice" in
    y|Y|yes|YES )
      install_gh
    ;;
    * ) echo "ok, skipping gh";;
  esac
fi


# Install nvm and node
install_nvm() {
  brew update && brew install nvm
  echo "plugins+=(npm)" >> $HOME/.zshrc_additional_plugins
  echo "plugins+=(node)" >> $HOME/.zshrc_additional_plugins
  echo "source $HOME/dotfile/.zshrc_node_ext" >> $HOME/.zshrc_ext
}
install_node() {
  nvm install node
}
if is_already_installed "nvm"; then
  read -p "nvm is already installed. Do you want to install the latest node version? (y/n) " choice
  case "$choice" in
    y|Y|yes|YES )
      install_node
    ;;
    * ) echo "ok, skipping nvm";;
  esac
else
  read -p "nvm is not installed. Do you want to install it? (y/n) " choice
  case "$choice" in
    y|Y|yes|YES )
      install_nvm
      install_node
    ;;
    * ) echo "ok, skipping nvm";;
  esac
fi

# Install terraform
install_terraform() {
  brew update && brew install tfenv && tfenv install latest
  echo "source $HOME/dotfile/.zshrc_terraform_ext" >> $HOME/.zshrc_ext
}
if is_already_installed "tfenv"; then
  echo "terraform is already installed"
else
  read -p "terraform is not installed. Do you want to install it? (y/n) " choice
  case "$choice" in
    y|Y|yes|YES ) install_terraform;;
    * ) echo "ok, skipping terraform";;
  esac
fi

# Install gcloud
install_gcloud() {
  brew update && brew install --cask google-cloud-sdk
  echo "plugins+=(gcloud)" >> $HOME/.zshrc_additional_plugins
  echo "source $HOME/dotfile/.zshrc_gcloud_ext" >> $HOME/.zshrc_ext
}
if is_already_installed "google-cloud-sdk"; then
  echo "google-cloud-sdk is already installed"
else
  read -p "google-cloud-sdk is not installed. Do you want to install it? (y/n) " choice
  case "$choice" in
    y|Y|yes|YES ) install_gcloud;;
    * ) echo "ok, skipping google-cloud-sdk";;
  esac
fi

# Install kubectl
install_kubectl() {
  brew update && brew install kubernetes-cli krew
  kubectl krew install ctx
  kubectl krew install ns
  echo "source $HOME/dotfile/.zshrc_k8s_ext" >> $HOME/.zshrc_ext
}
if is_already_installed "kubernetes-cli"; then
  echo "kubectl is already installed"
else
  read -p "kubectl is not installed. Do you want to install it? (y/n) " choice
  case "$choice" in
    y|Y|yes|YES ) install_kubectl;;
    * ) echo "ok, skipping kubectl";;
  esac
fi

# Install helm
install_helm() {
  brew update && brew install helm
}
if is_already_installed "helm"; then
  echo "helm is already installed"
else
  read -p "helm is not installed. Do you want to install it? (y/n) " choice
  case "$choice" in
    y|Y|yes|YES ) install_helm;;
    * ) echo "ok, skipping helm";;
  esac
fi

# Install docker
install_docker_desktop() {
  brew update && brew install --cask docker
}
install_docker_lima() {
  brew update && brew install colima docker docker-credential-helper
}
extend_docker() {
  # Install docker-buildx
  brew install docker-buildx
  mkdir -p ~/.docker/cli-plugins
  ln -sfn $(which docker-buildx) ~/.docker/cli-plugins/docker-buildx

  # Extend zshrc
  echo "source $HOME/dotfile/.zshrc_docker_ext" >> $HOME/.zshrc_ext
}
install_doker() {
  read -p "Do you want to install docker using docker desktop or lima? ([d]esktop/[l]ima) " choice
  case "$choice" in
    d|desktop ) install_docker_desktop;;
    l|lima ) install_docker_lima;;
    * ) echo "Unrecognized option, exiting" && exit;;
  esac
  extend_docker
  packages_to_be_configured+=("docker")
}
if is_already_installed "docker"; then
  echo "docker is already installed"
else
  read -p "docker is not installed. Do you want to install it? (y/n) " choice
  case "$choice" in
    y|Y|yes|YES ) install_docker;;
    * ) echo "ok, skipping docker";;
  esac
fi

# Install neovim
# TODO: explore the different configurations

# Install tmux
# TODO: explore the different configurations

# Install raycast
install_raycast() {
  brew update && brew install --cask raycast
  packages_to_be_configured+=("raycast")
}
if is_already_installed "raycast"; then
  echo "raycast is already installed"
else
  read -p "raycast is not installed. Do you want to install it? (y/n) " choice
  case "$choice" in
    y|Y|yes|YES ) install_raycast;;
    * ) echo "ok, skipping raycast";;
  esac
fi

# Install iterm2
install_iterm2() {
  brew update && brew install --cask iterm2
  packages_to_be_configured+=("iterm2")
}
if is_already_installed "iterm2"; then
  echo "iTerm2 is already installed"
else
  read -p "iTerm2 is not installed. Do you want to install it? (y/n) " choice
  case "$choice" in
    y|Y|yes|YES ) install_iterm2;;
    * ) echo "ok, skipping iTerm2";;
  esac
fi

# Install visual-studio-code
install_vscode() {
  brew update && brew install --cask visual-studio-code
  packages_to_be_configured+=("visual-studio-code")
}
if is_already_installed "visual-studio-code"; then
  echo "visual-studio-code is already installed"
else
  read -p "visual-studio-code is not installed. Do you want to install it? (y/n) " choice
  case "$choice" in
    y|Y|yes|YES ) install_vscode;;
    * ) echo "ok, skipping visual-studio-code";;
  esac
fi

# Install Slack
install_slack() {
  brew update && brew install --cask slack
  packages_to_be_configured+=("slack")
}
if is_already_installed "slack"; then
  echo "slack is already installed"
else
  read -p "slack is not installed. Do you want to install it? (y/n) " choice
  case "$choice" in
    y|Y|yes|YES ) install_slack;;
    * ) echo "ok, skipping slack";;
  esac
fi

# Install 1password
install_1password() {
  brew update && brew install --cask 1password 1password/tap/1password-cli
  packages_to_be_configured+=("1password")
}
if is_already_installed "1password"; then
  echo "1password is already installed"
else
  read -p "1password is not installed. Do you want to install it? (y/n) " choice
  case "$choice" in
    y|Y|yes|YES ) install_1password;;
    * ) echo "ok, skipping 1password";;
  esac
fi

# Install appcleaner
install_appcleaner() {
  brew update && brew install --cask appcleaner
}
if is_already_installed "appcleaner"; then
  echo "appcleaner is already installed"
else
  read -p "appcleaner is not installed. Do you want to install it? (y/n) " choice
  case "$choice" in
    y|Y|yes|YES ) install_appcleaner;;
    * ) echo "ok, skipping appcleaner";;
  esac
fi

# Install Google Chrome
install_google_chrome() {
  brew update && brew install --cask google-chrome
  packages_to_be_configured+=("google-chrome")
}
if is_already_installed "google-chrome"; then
  echo "google-chrome is already installed"
else
  read -p "google-chrome is not installed. Do you want to install it? (y/n) " choice
  case "$choice" in
    y|Y|yes|YES ) install_google_chrome;;
    * ) echo "ok, skipping google-chrome";;
  esac
fi

# Install flux -> TODO: run at startup
install_flux() {
  brew update && brew install --cask flux
  packages_to_be_configured+=("flux")
}
if is_already_installed "flux"; then
  echo "flux is already installed"
else
  read -p "flux is not installed. Do you want to install it? (y/n) " choice
  case "$choice" in
    y|Y|yes|YES ) install_flux;;
    * ) echo "ok, skipping flux";;
  esac
fi


# Install rectangle -> TODO: run at startup
install_rectangle() {
  brew update && brew install --cask rectangle
  packages_to_be_configured+=("rectangle")
}
if is_already_installed "rectangle"; then
  echo "rectangle is already installed"
else
  read -p "rectangle is not installed. Do you want to install it? (y/n) " choice
  case "$choice" in
    y|Y|yes|YES ) install_rectangle;;
    * ) echo "ok, skipping rectangle";;
  esac
fi

# Install ripgrep
install_ripgrep() {
  brew update && brew install ripgrep
}
if is_already_installed "ripgrep"; then
  echo "ripgrep is already installed"
else
  read -p "ripgrep is not installed. Do you want to install it? (y/n) " choice
  case "$choice" in
    y|Y|yes|YES )
      install_ripgrep
      echo "source $HOME/dotfile/.zshrc_ripgrep_ext" >> $HOME/.zshrc_ext
    ;;
    * ) echo "ok, skipping ripgrep";;
  esac
fi

# Install spotify
install_spotify() {
  brew update && brew install --cask spotify
  packages_to_be_configured+=("spotify")
}
if is_already_installed "spotify"; then
  echo "spotify is already installed"
else
  read -p "spotify is not installed. Do you want to install it? (y/n) " choice
  case "$choice" in
    y|Y|yes|YES )
      install_spotify
    ;;
    * ) echo "ok, skipping spotify";;
  esac
fi

# Install transmission
install_transmission() {
  brew update && brew install --cask transmission
}
if is_already_installed "transmission"; then
  echo "transmission is already installed"
else
  read -p "transmission is not installed. Do you want to install it? (y/n) " choice
  case "$choice" in
    y|Y|yes|YES )
      install_transmission
    ;;
    * ) echo "ok, skipping transmission";;
  esac
fi

# Install vlc
install_vlc() {
  brew update && brew install --cask vlc
}
if is_already_installed "vlc"; then
  echo "vlc is already installed"
else
  read -p "vlc is not installed. Do you want to install it? (y/n) " choice
  case "$choice" in
    y|Y|yes|YES )
      install_vlc
    ;;
    * ) echo "ok, skipping vlc";;
  esac
fi

# Install obsidian
install_obsidian() {
  brew update && brew install --cask obsidian
  packages_to_be_configured+=("obsidian")
}
if is_already_installed "obsidian"; then
  echo "obsidian is already installed"
else
  read -p "obsidian is not installed. Do you want to install it? (y/n) " choice
  case "$choice" in
    y|Y|yes|YES )
      install_obsidian
    ;;
    * ) echo "ok, skipping obsidian";;
  esac
fi

echo "Done! You might want to configure the following packages: ${packages_to_be_configured[@]}"
