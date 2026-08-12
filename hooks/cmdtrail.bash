# cmdtrail hook for bash
# Install: eval "$(cmdtrail init bash)"   # in ~/.bashrc

__cmdtrail_last_hist=-1

__cmdtrail_log() {
    local exit_code=$?
    local hist_line
    hist_line=$(HISTTIMEFORMAT= history 1)
    local hist_num=${hist_line%%[!0-9]*}
    if [[ -z "$hist_num" || "$hist_num" == "$__cmdtrail_last_hist" ]]; then
        return
    fi
    __cmdtrail_last_hist=$hist_num
    local cmd=${hist_line#*[0-9] }
    cmdtrail log "$cmd" --cwd "$PWD" --shell bash --exit-code "$exit_code" >/dev/null 2>&1
}
PROMPT_COMMAND="__cmdtrail_log${PROMPT_COMMAND:+; $PROMPT_COMMAND}"

__cmdtrail_pick() {
    local suggestion
    suggestion=$(cmdtrail suggest --cwd "$PWD" --pick 2>/dev/null)
    if [[ -n "$suggestion" ]]; then
        READLINE_LINE="$suggestion"
        READLINE_POINT=${#suggestion}
    fi
}
bind -x '"\C-g": __cmdtrail_pick'
