#/bin/zsh

git clone https://github.com/zsh-users/zsh-syntax-highlighting.git $ZSH_CUSTOM/plugins/zsh-syntax-highlighting

# TODO ask if the user wants to override the .zshrc or not
echo "\
source ~/dotfile/.zshrc_rstagi\
" >> ~/.zshrc
