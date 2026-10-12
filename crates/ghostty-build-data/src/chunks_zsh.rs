// Generated from the Zig multiline strings of extra/zsh.zig. Do not edit.
pub const C0: &str = r####"#compdef ghostty

_fonts () {
  local font_list=$(ghostty +list-fonts | grep -Z '^[A-Z]')
  local fonts=(${(f)font_list})
  _describe -t fonts 'fonts' fonts
}

_themes() {
  local theme_list=$(ghostty +list-themes | sed -E 's/^(.*) \(.*$/\1/')
  local themes=(${(f)theme_list})
  _describe -t themes 'themes' themes
}
"####;
pub const C1: &str = r####"_ghostty() {
  typeset -A opt_args
  local context state line
  local opt=('-e' '--help' '--version')

  _arguments -C \
    '1:actions:->actions' \
    '*:: :->rest' \

  if [[ "$line[1]" == "--help" || "$line[1]" == "--version" || "$line[1]" == "-e" ]]; then
    return
  fi

  if [[ "$line[1]" == -* ]]; then
    _config
    return
  fi

  case "$state" in
    (actions)
      local actions; actions=(
"####;
pub const C2: &str = r####"      )
      _describe '' opt
      _describe -t action 'action' actions
    ;;
    (rest)
      if [[ "$line[2]" == "--help" ]]; then
        return
      fi

      local help=('--help')
      _describe '' help

      case $line[1] in
"####;
pub const C3: &str = r####"      esac
    ;;
  esac
}

_ghostty "$@"
"####;
