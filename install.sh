#!/bin/bash

# Global variables
AVAILABLE_PACKAGES=("arc" "warp" "cursor" "rectangle" "fzf" "zsh" "python" "gh" "node" "terraform" "gcloud" "kubectl" "helm" "docker" "tmux" "neovim" "raycast" "ghostty" "slack" "1password" "appcleaner" "google-chrome" "ripgrep" "claude-code" "ralph")
REQUESTED_PACKAGES=()
INTERACTIVE_MODE=true
DRY_RUN=false
gcloud_installed=false

# Dependency mapping
get_dependencies() {
  case $1 in
    "kubectl") echo "gcloud" ;;
    "docker") echo "gcloud" ;;
    "zsh") echo "" ;;
    "python") echo "pyenv pipx" ;;
    "ralph") echo "node" ;;
    *) echo "" ;;
  esac
}

# Parse command line arguments
parse_arguments() {
  if [ $# -eq 0 ]; then
    INTERACTIVE_MODE=true
    return
  fi
  
  INTERACTIVE_MODE=false
  
  for arg in "$@"; do
    case $arg in
      --list)
        echo "Available packages:"
        printf '%s\n' "${AVAILABLE_PACKAGES[@]}"
        exit 0
        ;;
      --help|-h)
        echo "Usage: $0 [package1] [package2] ... [packageN]"
        echo "       $0 --list"
        echo "       $0 --dry-run [package1] [package2] ... [packageN]"
        echo ""
        echo "Available packages:"
        printf '%s\n' "${AVAILABLE_PACKAGES[@]}"
        exit 0
        ;;
      --dry-run)
        DRY_RUN=true
        ;;
      *)
        REQUESTED_PACKAGES+=("$arg")
        ;;
    esac
  done
  
  # Validate requested packages
  for package in "${REQUESTED_PACKAGES[@]}"; do
    if [[ ! " ${AVAILABLE_PACKAGES[@]} " =~ " ${package} " ]]; then
      echo "Error: Unknown package '$package'"
      echo "Use --list to see available packages"
      exit 1
    fi
  done
}

# Install prerequisites (always run)
install_prerequisites() {
  echo "=== Installing Prerequisites ==="
  
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
}

# Setup core configuration (always run)
setup_core() {
  echo "=== Setting up Core Configuration ==="
  
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
  
  if [ "$INTERACTIVE_MODE" = true ]; then
    read -p "Do you want to configure git with the proper dotfile? (y/n) " choice
    case "$choice" in
      y|Y|yes|YES ) link_gitconfig;;
      * ) echo "ok, skipping git configuration";;
    esac
  else
    echo "Configuring git automatically..."
    link_gitconfig
  fi
}


# Utility functions
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
    if [ "$INTERACTIVE_MODE" = true ]; then
      if read_yes "$1 is not installed. Do you want to install it?"; then
        install_pkg $1 $2
        return 0
      else
        return 1
      fi
    else
      echo "Installing $1..."
      install_pkg $1 $2
      return 0
    fi
  fi
}

# Dependency resolution
resolve_dependencies() {
  local package=$1
  local deps=$(get_dependencies "$package")
  
  if [ -n "$deps" ]; then
    echo "Installing dependencies for $package: $deps"
    for dep in $deps; do
      if [[ " ${AVAILABLE_PACKAGES[@]} " =~ " ${dep} " ]]; then
        install_package $dep
      else
        echo "Installing special dependency: $dep"
        case $dep in
          "pyenv") install_pkg_if_needed "pyenv" ;;
          "pipx") install_pkg_if_needed "pipx" ;;
        esac
      fi
    done
  fi
}

# Package installation functions
install_package() {
  local package=$1
  echo "=== Installing $package ==="
  
  # Resolve dependencies first
  resolve_dependencies $package
  
  case $package in
    "arc") install_pkg_if_needed "arc" "--cask" ;;
    "warp") install_pkg_if_needed "warp" "--cask" ;;
    "cursor") install_pkg_if_needed "cursor" "--cask" ;;
    "rectangle") install_pkg_if_needed "rectangle" "--cask" ;;
    "fzf") install_fzf ;;
    "zsh") install_zsh ;;
    "python") install_python ;;
    "gh") install_gh ;;
    "node") install_node ;;
    "terraform") install_terraform ;;
    "gcloud") install_gcloud ;;
    "kubectl") install_kubectl ;;
    "helm") install_pkg_if_needed "helm" ;;
    "docker") install_docker ;;
    "tmux") install_tmux ;;
    "neovim") install_neovim ;;
    "raycast") install_pkg_if_needed "raycast" "--cask" ;;
    "ghostty") install_pkg_if_needed "ghostty" ;;
    "slack") install_pkg_if_needed "slack" "--cask" ;;
    "1password") install_1password ;;
    "appcleaner") install_pkg_if_needed "appcleaner" ;;
    "google-chrome") install_pkg_if_needed "google-chrome" ;;
    "ripgrep") install_pkg_if_needed "ripgrep" ;;
    "claude-code") install_claude_code ;;
    "ralph") install_ralph ;;
    *) echo "Unknown package: $package" ;;
  esac
}

# Individual package installation functions
install_fzf() {
  if [ "$INTERACTIVE_MODE" = true ]; then
    if read_yes "Do you want to setup fzf?"; then
      $(brew --prefix)/opt/fzf/install
    fi
  else
    echo "Configuring fzf..."
    $(brew --prefix)/opt/fzf/install
  fi
}

install_zsh() {
  if install_pkg_if_needed "zsh"; then
    brew update && brew install zsh && chsh zsh
    
    # Install zsh-syntax-highlighting via brew
    install_pkg_if_needed "zsh-syntax-highlighting"
    
    # Link .zshrc
    link_zshrc() {
      [ -f "$HOME/.zshrc" ] && mv $HOME/.zshrc $HOME/.zshrc.bak
      ln -s $HOME/dotfile/.zshrc $HOME/.zshrc
    }
    
    if [ -f "$HOME/.zshrc" ]; then
      if [ "$INTERACTIVE_MODE" = true ]; then
        if read_yes ".zshrc already exists. Do you want to override it?"; then
          link_zshrc
        else
          echo "ok, skipping .zshrc override"
        fi
      else
        echo "Overriding .zshrc..."
        link_zshrc
      fi
    else
      link_zshrc
    fi
  fi
}


install_python() {
  if install_pkg_if_needed "pyenv"; then
    install_pkg_if_needed "pipx"
    configure_python() {
      brew update && brew upgrade pyenv
      echo "source $HOME/dotfile/.zshrc_python_ext" >> $HOME/.zshrc_ext
      export PYENV_ROOT="$HOME/.pyenv"
      command -v pyenv >/dev/null || export PATH="$PYENV_ROOT/bin:$PATH"
      eval "$(pyenv init -)"
      CFLAGS="-I$(brew --prefix xz)/include -I$(brew --prefix openssl)/include" LDFLAGS="-L$(brew --prefix xz)/lib -L$(brew --prefix openssl)/lib" pyenv install -s 3
    }
    if [ "$INTERACTIVE_MODE" = true ]; then
      if read_yes "Do you want to configure python?"; then
        configure_python
      fi
    else
      echo "Configuring python..."
      configure_python
    fi
  fi
}

install_gh() {
  if install_pkg_if_needed "gh"; then
    configure_gh() {
      TARGET_PATH="$(brew --prefix)/share/zsh/site-functions"
      [ ! -f "$TARGET_PATH/_gh" ] && mkdir -p $TARGET_PATH && gh completion -s zsh > "$TARGET_PATH/_gh" && chmod +x "$TARGET_PATH/_gh"
    }
    if [ "$INTERACTIVE_MODE" = true ]; then
      if read_yes "Do you want to configure gh?"; then
        configure_gh
      fi
    else
      echo "Configuring gh..."
      configure_gh
    fi
  fi
}

install_node() {
  if install_pkg_if_needed "nvm"; then
    configure_node() {
      nvm install --latest 
      echo "plugins+=(npm)" >> $HOME/.zshrc_additional_plugins
      echo "plugins+=(node)" >> $HOME/.zshrc_additional_plugins
      echo "source $HOME/dotfile/.zshrc_node_ext" >> $HOME/.zshrc_ext
      npm install --global yarn
    }
    if [ "$INTERACTIVE_MODE" = true ]; then
      if read_yes "Do you want to install node?"; then
        configure_node
      fi
    else
      echo "Installing node..."
      configure_node
    fi
  fi
}

install_terraform() {
  if install_pkg_if_needed "tfenv"; then
    configure_terraform() {
      tfenv install latest
      terraform -install-autocomplete
      echo "source $HOME/dotfile/.zshrc_terraform_ext" >> $HOME/.zshrc_ext
    }
    if [ "$INTERACTIVE_MODE" = true ]; then
      if read_yes "Do you want to configure terraform?"; then
        configure_terraform
      fi
    else
      echo "Configuring terraform..."
      configure_terraform
    fi
  fi
}

install_gcloud() {
  if install_pkg_if_needed "google-cloud-sdk"; then
    configure_gcloud() {
      echo "plugins+=(gcloud)" >> $HOME/.zshrc_additional_plugins
      echo "source $HOME/dotfile/.zshrc_gcloud_ext" >> $HOME/.zshrc_ext
      gcloud auth login --update-adc --enable-gdrive-access
      gcloud auth application-default login
      gcloud_installed=true
    }
    if [ "$INTERACTIVE_MODE" = true ]; then
      if read_yes "Do you want to configure gcloud?"; then
        configure_gcloud
      fi
    else
      echo "Configuring gcloud..."
      configure_gcloud
    fi
  fi
}

install_kubectl() {
  if install_pkg_if_needed "kubernetes-cli"; then
    configure_kubectl() {
      brew update && brew install krew
      kubectl krew install ctx
      kubectl krew install ns
      echo "source $HOME/dotfile/.zshrc_k8s_ext" >> $HOME/.zshrc_ext
    }
    if [ "$INTERACTIVE_MODE" = true ]; then
      if read_yes "Do you want to configure kubectl?"; then
        configure_kubectl
      fi
    else
      echo "Configuring kubectl..."
      configure_kubectl
    fi
  fi
}

install_docker() {
  if install_pkg_if_needed "docker" "--cask"; then
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
    if [ "$INTERACTIVE_MODE" = true ]; then
      if read_yes "Do you want to configure docker?"; then
        configure_docker
      fi
    else
      echo "Configuring docker..."
      configure_docker
    fi
  fi
}

install_tmux() {
  if install_pkg_if_needed "tmux"; then
    configure_tmux() {
      git clone https://github.com/tmux-plugins/tpm ~/.tmux/plugins/tpm
      ln -s $HOME/dotfile/.tmux.conf $HOME/.tmux.conf
    }
    if [ "$INTERACTIVE_MODE" = true ]; then
      if read_yes "Do you want to configure tmux?"; then
        configure_tmux
      fi
    else
      echo "Configuring tmux..."
      configure_tmux
    fi
  fi
}

install_neovim() {
  if install_pkg_if_needed "neovim"; then
    configure_neovim() {
      echo "source $HOME/dotfile/.zshrc_vim_ext" >> $HOME/.zshrc_ext
    }
    if [ "$INTERACTIVE_MODE" = true ]; then
      if read_yes "Do you want to configure neovim?"; then
        configure_neovim
      fi
    else
      echo "Configuring neovim..."
      configure_neovim
    fi
  fi
}

install_1password() {
  if install_pkg_if_needed "1password"; then
    configure_1password() {
      brew update && brew install --cask 1password/tap/1password-cli
    }
    if [ "$INTERACTIVE_MODE" = true ]; then
      if read_yes "Do you want to configure 1password?"; then
        configure_1password
      fi
    else
      echo "Configuring 1password..."
      configure_1password
    fi
  fi
}

install_claude_code() {
  if ! command -v claude &> /dev/null; then
    if [ "$INTERACTIVE_MODE" = true ]; then
      if read_yes "claude-code is not installed. Do you want to install it?"; then
        curl -fsSL https://claude.ai/install.sh | bash
      fi
    else
      echo "Installing claude-code..."
      curl -fsSL https://claude.ai/install.sh | bash
    fi
  else
    echo "claude-code is already installed"
  fi
}

install_ralph() {
  if ! command -v ralph &> /dev/null; then
    if [ "$INTERACTIVE_MODE" = true ]; then
      if read_yes "ralph is not installed. Do you want to install it?"; then
        npm install -g @anthropic-ai/ralph
        echo "source $HOME/dotfile/.zshrc_ralph_ext" >> "$HOME/.zshrc_ext"
      fi
    else
      echo "Installing ralph..."
      npm install -g @anthropic-ai/ralph
      echo "source $HOME/dotfile/.zshrc_ralph_ext" >> "$HOME/.zshrc_ext"
    fi
  else
    echo "ralph is already installed"
  fi
}

# Main execution logic
main() {
  parse_arguments "$@"
  
  # Always install prerequisites and core setup (unless dry-run)
  if [ "$DRY_RUN" = true ]; then
    echo "=== DRY RUN: Would install prerequisites and core setup ==="
  else
    install_prerequisites
    setup_core
  fi
  
  if [ "$INTERACTIVE_MODE" = true ]; then
    # Original interactive mode
    echo "=== Important packages ==="
    install_pkg_if_needed "arc" "--cask"
    install_pkg_if_needed "warp" "--cask"
    install_pkg_if_needed "cursor" "--cask"
    install_pkg_if_needed "rectangle" "--cask"

    if read_yes "Important packages have been installed. Would you like to continue with the optional stuff?"; then
      echo "Alright, let's go!"
    else
      exit
    fi

    # Continue with all optional packages in interactive mode
    install_fzf
    install_zsh
    install_python
    install_gh
    install_node
    install_terraform
    install_gcloud
    install_kubectl
    install_pkg_if_needed "helm"
    install_docker
    install_tmux
    install_neovim
    install_pkg_if_needed "raycast" "--cask"
    install_pkg_if_needed "ghostty"
    install_pkg_if_needed "slack" "--cask"
    install_1password
    install_pkg_if_needed "appcleaner"
    install_pkg_if_needed "google-chrome"
    install_pkg_if_needed "ripgrep"
    install_claude_code
    install_ralph
  else
    # Selective installation mode
    if [ "$DRY_RUN" = true ]; then
      echo "=== DRY RUN: Would install requested packages ==="
      for package in "${REQUESTED_PACKAGES[@]}"; do
        echo "Would install: $package"
        deps=$(get_dependencies "$package")
        if [ -n "$deps" ]; then
          echo "  Dependencies: $deps"
        fi
      done
    else
      echo "=== Installing requested packages ==="
      for package in "${REQUESTED_PACKAGES[@]}"; do
        install_package "$package"
      done
    fi
  fi
  
  echo "Done!"
}

# Run main function with all arguments
main "$@"
