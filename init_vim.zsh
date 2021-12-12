#!/bin/zsh

VIMRC_PATH=~/.vimrc

CUSTOM_VIMRC_CONTENT="
\" Disable compatibility with vi which can cause unexpected issues
set nocompatible

\" Enable type file detection
filetype off 

\" Enable plugins and load plugin for the detected file type
filetype plugin indent on

\" Turn syntax hightlighting on
syntax on
colorscheme desert
set hlsearch
hi Search ctermbg=LightYellow
hi Search ctermfg=Red

\" Add line number on the left
set number

\" Set tab to 2 spaces 
set tabstop=2
set shiftwidth=2
set expandtab

\" Do not save backup files
set nobackup

\" Do not let cursor scroll below or above 10 lines
set scrolloff=10

\" Do not wrap lines
set nowrap

\" Highlight matching characters while searching
set incsearch
set showmatch

\" Ignore capital letters during search
set ignorecase
set smartcase

\" Show partial command you type in the last line of the screen
set showcmd

\" Set commands history to save 1000
set history=1000

\" Enable auto completion in menu with TAB
set wildmenu
set wildmode=list:longest
set wildignore=*.docx,*.jpg,*.png,*.gif,*.pdf,*.pyc,*.exe,*.flv,*.img,*.xlsx
"
	
if test -f $VIMRC_PATH; then
	mv $VIMRC_PATH $VIMRC_PATH.old
fi
echo "$CUSTOM_VIMRC_CONTENT" > $VIMRC_PATH

