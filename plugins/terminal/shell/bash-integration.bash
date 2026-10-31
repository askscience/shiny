# Shiny Terminal — bash shell integration.
#
# Loaded with `bash --init-file <this file>` for the Terminal window's shells
# (never the AI's `terminal_exec` shell). It replaces bash's own startup
# files, so it first runs what a login shell would, then adds the hooks:
#
#   OSC 133;A        a prompt starts          (PROMPT_COMMAND)
#   OSC 133;C        the command's output     (PS0)
#   OSC 133;D;<code> the command finished     (PROMPT_COMMAND, before A)
#   OSC 7            the working directory    (PROMPT_COMMAND)
#
# The window uses them for the command marks and blocks, and for the cwd in
# the status line. Nothing is exported: a nested shell starts clean.

if [ -r /etc/profile ]; then . /etc/profile; fi
if [ -r "$HOME/.bash_profile" ]; then . "$HOME/.bash_profile"
elif [ -r "$HOME/.bash_login" ]; then . "$HOME/.bash_login"
elif [ -r "$HOME/.profile" ]; then . "$HOME/.profile"
elif [ -r "$HOME/.bashrc" ]; then . "$HOME/.bashrc"
fi

if [ -z "${__shiny_integration-}" ]; then
  __shiny_integration=1

  __shiny_precmd() {
    local ec=$?
    printf '\033]133;D;%s\007\033]7;file://%s%s\007\033]133;A\007' "$ec" "${HOSTNAME-}" "$PWD"
    return $ec
  }

  # First in PROMPT_COMMAND, so it still sees the command's exit status.
  if [[ "$(declare -p PROMPT_COMMAND 2>/dev/null)" == "declare -a"* ]]; then
    PROMPT_COMMAND=(__shiny_precmd "${PROMPT_COMMAND[@]}")
  else
    PROMPT_COMMAND="__shiny_precmd${PROMPT_COMMAND:+; $PROMPT_COMMAND}"
  fi
  PS0="${PS0-}"$'\033]133;C\007'
fi
