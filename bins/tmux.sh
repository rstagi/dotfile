#!/bin/zsh

which -s brew
if [[ $? != 0 ]]; then
  echo "Brew is not installed but it is required to install tmux."
  if read -q "choice?Press Y/y to continue with installing the MacOS dependencies: "; then
    echo
    ./install_dependencies.sh
  else
    echo
    echo "Cannot continue without brew."
    exit 1
  fi
fi
brew update

if [[ "$(uname -m)" == "arm64" ]]; then
  arch -arm64 brew install tmux
else
  brew install tmux
fi

