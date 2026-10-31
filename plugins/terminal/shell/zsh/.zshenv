# Shiny Terminal — zsh shell integration.
#
# The Terminal window starts zsh with ZDOTDIR pointing here, so this file runs
# first. It hands ZDOTDIR back (the user's .zprofile, .zshrc and .zlogin then
# load as usual), sources the user's own .zshenv and adds the hooks:
#
#   OSC 133;A        a prompt starts          (precmd)
#   OSC 133;C        the command's output     (preexec)
#   OSC 133;D;<code> the command finished     (precmd, before A)
#   OSC 7            the working directory    (precmd)
#
# The hooks are added before the user's rc files, so ours run first and still
# see the command's exit status.

unset ZDOTDIR
[[ -r "$HOME/.zshenv" ]] && source "$HOME/.zshenv"

if [[ -o interactive ]]; then
  __shiny_precmd() {
    local ec=$?
    print -rn -- $'\e]133;D;'"$ec"$'\a\e]7;file://'"${HOST}${PWD}"$'\a\e]133;A\a'
    return $ec
  }
  __shiny_preexec() { print -rn -- $'\e]133;C\a' }
  autoload -Uz add-zsh-hook
  add-zsh-hook precmd __shiny_precmd
  add-zsh-hook preexec __shiny_preexec
fi
