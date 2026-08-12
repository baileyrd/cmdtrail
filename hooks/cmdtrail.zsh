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
