// Generated from the Zig multiline strings of extra/vim.zig. Do not edit.
pub const C0: &str = r####"" Vim filetype detect file
" Language: Ghostty config file
" Maintainer: Ghostty <https://github.com/ghostty-org/ghostty>
"
" THIS FILE IS AUTO-GENERATED

au BufRead,BufNewFile */ghostty/config,*/*.ghostty/config,*/ghostty/themes/*,*.ghostty setf ghostty
"####;
pub const C1: &str = r####"" Vim filetype plugin file
" Language: Ghostty config file
" Maintainer: Ghostty <https://github.com/ghostty-org/ghostty>
"
" THIS FILE IS AUTO-GENERATED

if exists('b:did_ftplugin')
  finish
endif
let b:did_ftplugin = 1

setlocal commentstring=#\ %s
setlocal iskeyword+=-

" Use syntax keywords for completion
setlocal omnifunc=syntaxcomplete#Complete

let b:undo_ftplugin = 'setl cms< isk< ofu<'

if !exists('current_compiler')
  compiler ghostty
  let b:undo_ftplugin .= " makeprg< errorformat<"
endif
"####;
pub const C2: &str = r####"" Vim compiler file
" Language: Ghostty config file
" Maintainer: Ghostty <https://github.com/ghostty-org/ghostty>
"
" THIS FILE IS AUTO-GENERATED

if exists("current_compiler")
  finish
endif
let current_compiler = "ghostty"

CompilerSet makeprg=ghostty\ +validate-config\ --config-file=%:S
CompilerSet errorformat=%f:%l:%m,%m
"####;
pub const C3: &str = r####"" Vim syntax file
" Language: Ghostty config file
" Maintainer: Ghostty <https://github.com/ghostty-org/ghostty>
"
" THIS FILE IS AUTO-GENERATED

if exists('b:current_syntax')
  finish
endif

let b:current_syntax = 'ghostty'

let s:cpo_save = &cpo
set cpo&vim

syn iskeyword @,48-57,-
syn keyword ghosttyConfigKeyword"####;
pub const C4: &str = r####"

syn match ghosttyConfigComment /^\s*#.*/ contains=@Spell

hi def link ghosttyConfigComment Comment
hi def link ghosttyConfigKeyword Keyword

let &cpo = s:cpo_save
unlet s:cpo_save
"####;
