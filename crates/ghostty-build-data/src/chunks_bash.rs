// Generated from the Zig multiline strings of extra/bash.zig. Do not edit.
pub const C0: &str = r####"_ghostty() {

  # compat: mapfile -t COMPREPLY < <( "$@" )
  _compreply() {
    COMPREPLY=()
    while IFS='' read -r line; do COMPREPLY+=("$line"); done < <( "$@" )
  }

  # -o nospace requires we add back a space when a completion is finished
  # and not part of a --key= completion
  _add_spaces() {
    for idx in "${!COMPREPLY[@]}"; do
      [ -n "${COMPREPLY[idx]}" ] && COMPREPLY[idx]="${COMPREPLY[idx]} ";
    done
  }

  _fonts() {
    local IFS=$'\n'
    COMPREPLY=()
    while read -r line; do COMPREPLY+=("$line"); done < <( compgen -P '"' -S '"' -W "$($ghostty +list-fonts | grep '^[A-Z]' )" -- "$cur")
  }

  _themes() {
    local IFS=$'\n'
    COMPREPLY=()
    while read -r line; do COMPREPLY+=("$line"); done < <( compgen -P '"' -S '"' -W "$($ghostty +list-themes | sed -E 's/^(.*) \(.*$/\1/')" -- "$cur")
  }

  _files() {
    _compreply compgen -o filenames -f -- "$cur"
    for i in "${!COMPREPLY[@]}"; do
      if [[ -d "${COMPREPLY[i]}" ]]; then
        COMPREPLY[i]="${COMPREPLY[i]}/";
      fi
      if [[ -f "${COMPREPLY[i]}" ]]; then
        COMPREPLY[i]="${COMPREPLY[i]} ";
      fi
    done
  }

  _dirs() {
    _compreply compgen -o dirnames -d -- "$cur"
    for i in "${!COMPREPLY[@]}"; do
      if [[ -d "${COMPREPLY[i]}" ]]; then
        COMPREPLY[i]="${COMPREPLY[i]}/";
      fi
    done
    if [[ "${#COMPREPLY[@]}" == 0 && -d "$cur" ]]; then
      COMPREPLY=( "$cur " )
    fi
  }

  _handle_config() {
    local config="--help"
    config+=" --version"
"####;
pub const C1: &str = r####"
    case "$prev" in
"####;
pub const C2: &str = r####"      *) _compreply compgen -W "$config" -- "$cur" ;;
    esac

    return 0
  }

  _handle_actions() {
"####;
pub const C3: &str = r####"
    case "${COMP_WORDS[1]}" in
"####;
pub const C4: &str = r####"        esac
      ;;
"####;
pub const C5: &str = r####"      *) _compreply compgen -W "--help" -- "$cur" ;;
    esac

    return 0
  }

  # begin main logic
  local topLevel="-e"
  topLevel+=" --help"
  topLevel+=" --version"
"####;
pub const C6: &str = r####"
  local cur=""; local prev=""; local prevWasEq=false; COMPREPLY=()
  local ghostty="$1"

  # script assumes default COMP_WORDBREAKS of roughly $' \t\n"\'><=;|&(:'
  # if = is missing this script will degrade to matching on keys only.
  # eg: --key=
  # this can be improved if needed see: https://github.com/ghostty-org/ghostty/discussions/2994

  if [ "$2" = "=" ]; then cur=""
  else                    cur="$2"
  fi

  if [ "$3" = "=" ]; then prev="${COMP_WORDS[COMP_CWORD-2]}"; prevWasEq=true;
  else                    prev="${COMP_WORDS[COMP_CWORD-1]}"
  fi

  # current completion is double quoted add a space so the cursor progresses
  if [[ "$2" == \"*\" ]]; then
    COMPREPLY=( "$cur " );
    return;
  fi

  case "$COMP_CWORD" in
    1)
      case "${COMP_WORDS[1]}" in
        -e | --help | --version) return 0 ;;
        --*) _handle_config ;;
        *) _compreply compgen -W "${topLevel}" -- "$cur"; _add_spaces ;;
      esac
      ;;
    *)
      case "$prev" in
        -e | --help | --version) return 0 ;;
        *)
          if [[ "=" != "${COMP_WORDS[COMP_CWORD]}" && $prevWasEq != true ]]; then
            # must be completing with a space after the key eg: '--<key> '
            # clear out prev so we don't run any of the key specific completions
            prev=""
          fi

          case "${COMP_WORDS[1]}" in
            --*) _handle_config ;;
            +*) _handle_actions ;;
          esac
          ;;
      esac
      ;;
  esac

  return 0
}

complete -o nospace -o bashdefault -F _ghostty ghostty
"####;
