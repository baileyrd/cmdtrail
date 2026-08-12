# cmdtrail hook for zsh
# Install: eval "$(cmdtrail init zsh)"   # in ~/.zshrc

__cmdtrail_log() {
    local exit_code=$?
    local cmd
    cmd=$(fc -ln -1)
    [[ -z "$cmd" ]] && return
    cmdtrail log "$cmd" --cwd "$PWD" --shell zsh --exit-code "$exit_code" >/dev/null 2>&1
}
autoload -Uz add-zsh-hook
add-zsh-hook precmd __cmdtrail_log

__cmdtrail_pick_widget() {
    local suggestion
    suggestion=$(cmdtrail suggest --cwd "$PWD" --pick 2>/dev/null)
    if [[ -n "$suggestion" ]]; then
        BUFFER="$suggestion"
        CURSOR=${#BUFFER}
    fi
    zle redisplay
}
zle -N __cmdtrail_pick_widget
bindkey '^G' __cmdtrail_pick_widget

# --- Ghost-text-as-you-type ---
# Shows the single best suggestion as dimmed text after the cursor (like
# zsh-autosuggestions), backed by cmdtrail's own ranking. Self-contained:
# uses zsh's own `add-zle-hook-widget` (bundled with zsh itself, no
# zsh-autosuggestions plugin dependency).
#
# Perf note: forks `cmdtrail suggest` synchronously on every edited
# keystroke (debounced against unchanged buffers / mid-line cursor). On an
# indexed SQLite history this is normally single-digit milliseconds; on a
# very large history it may be noticeable. Set CMDTRAIL_GHOST_TEXT=0
# before this hook loads to disable it and keep only the Ctrl+G picker.
if [[ "${CMDTRAIL_GHOST_TEXT:-1}" != "0" ]]; then
    autoload -Uz add-zle-hook-widget
    zmodload zsh/terminfo 2>/dev/null
    typeset -g _cmdtrail_last_buffer=""

    __cmdtrail_suggest_widget() {
        # Only suggest when the cursor is at the end of the buffer
        # (appending mid-line doesn't make sense), and only re-query when
        # the buffer actually changed (skip pure cursor movement).
        if [[ $CURSOR -ne $#BUFFER || -z "$BUFFER" ]]; then
            POSTDISPLAY=""
            _cmdtrail_last_buffer="$BUFFER"
            return
        fi
        [[ "$BUFFER" == "$_cmdtrail_last_buffer" ]] && return
        _cmdtrail_last_buffer="$BUFFER"

        local suggestion
        suggestion=$(cmdtrail suggest --cwd "$PWD" --query "$BUFFER" --limit 1 2>/dev/null)
        if [[ -n "$suggestion" && "$suggestion" == "$BUFFER"* && "$suggestion" != "$BUFFER" ]]; then
            POSTDISPLAY="${suggestion#$BUFFER}"
        else
            POSTDISPLAY=""
        fi
    }
    add-zle-hook-widget line-pre-redraw __cmdtrail_suggest_widget

    __cmdtrail_accept_suggestion() {
        if [[ -n "$POSTDISPLAY" ]]; then
            BUFFER="$BUFFER$POSTDISPLAY"
            POSTDISPLAY=""
            CURSOR=${#BUFFER}
        fi
    }

    # Right arrow: accept if a suggestion is showing and the cursor is
    # already at the end of the buffer, otherwise move the cursor as usual.
    __cmdtrail_accept_or_forward_char() {
        if [[ -n "$POSTDISPLAY" && $CURSOR -eq $#BUFFER ]]; then
            __cmdtrail_accept_suggestion
        else
            zle .forward-char
        fi
    }
    zle -N __cmdtrail_accept_or_forward_char
    bindkey "${terminfo[kcuf1]:-$'\e[C'}" __cmdtrail_accept_or_forward_char

    # End key: same acceptance behavior.
    __cmdtrail_accept_or_end_of_line() {
        if [[ -n "$POSTDISPLAY" ]]; then
            __cmdtrail_accept_suggestion
        else
            zle .end-of-line
        fi
    }
    zle -N __cmdtrail_accept_or_end_of_line
    bindkey "${terminfo[kend]:-$'\e[F'}" __cmdtrail_accept_or_end_of_line
fi
