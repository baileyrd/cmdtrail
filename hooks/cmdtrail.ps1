# cmdtrail hook for PowerShell / PSReadLine
# Install: add to your $PROFILE ->  . (cmdtrail init pwsh | Out-String)
# or simply:  cmdtrail init pwsh >> $PROFILE

if (-not (Get-Module -ListAvailable -Name PSReadLine)) {
    Write-Warning "cmdtrail: PSReadLine not found; logging/suggest key binding skipped."
    return
}

# Logging happens from `prompt`, not AddToHistoryHandler. AddToHistoryHandler
# fires while the line is being submitted, BEFORE it executes, so $? there
# can never reflect that line's real outcome (every command was silently
# logged with exit-code 0, always). `prompt` runs after the command
# finishes and before the next line is read -- the same semantic position
# as bash's PROMPT_COMMAND / zsh's precmd, where $? is correct.
#
# Wraps whatever `prompt` function already exists (oh-my-posh, posh-git,
# a custom one, ...) instead of replacing it, so nothing else in your
# profile silently stops rendering.
if (Test-Path Function:\prompt) {
    Rename-Item Function:\prompt __cmdtrail_previous_prompt -Force
}

$global:__cmdtrail_last_history_id = -1
function prompt {
    $exitCode = if ($?) { 0 } else { 1 }
    $lastCmd = Get-History -Count 1
    # Guard against logging the same history entry twice if `prompt`
    # happens to run more than once between commands.
    if ($lastCmd -and $lastCmd.Id -ne $global:__cmdtrail_last_history_id) {
        $global:__cmdtrail_last_history_id = $lastCmd.Id
        try {
            & cmdtrail log $lastCmd.CommandLine --cwd (Get-Location).Path --shell pwsh --exit-code $exitCode 2>$null | Out-Null
        } catch {}
    }
    if (Test-Path Function:\__cmdtrail_previous_prompt) {
        & $function:__cmdtrail_previous_prompt
    } else {
        "PS $($executionContext.SessionState.Path.CurrentLocation)$('>' * ($nestedPromptLevel + 1)) "
    }
}

Set-PSReadLineKeyHandler -Chord 'Ctrl+g' -ScriptBlock {
    $cwd = (Get-Location).Path
    $suggestion = & cmdtrail suggest --cwd $cwd --pick 2>$null
    if ($LASTEXITCODE -eq 0 -and $suggestion) {
        [Microsoft.PowerShell.PSConsoleReadLine]::RevertLine()
        [Microsoft.PowerShell.PSConsoleReadLine]::Insert($suggestion)
    }
}
