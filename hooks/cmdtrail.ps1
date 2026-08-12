# cmdtrail hook for PowerShell / PSReadLine
# Install: add to your $PROFILE ->  . (cmdtrail init pwsh | Out-String)
# or simply:  cmdtrail init pwsh >> $PROFILE

if (-not (Get-Module -ListAvailable -Name PSReadLine)) {
    Write-Warning "cmdtrail: PSReadLine not found; logging/suggest key binding skipped."
    return
}

Set-PSReadLineOption -AddToHistoryHandler {
    param([string]$line)
    try {
        $exit = if ($?) { 0 } else { 1 }
        & cmdtrail log $line --cwd (Get-Location).Path --shell pwsh --exit-code $exit 2>$null | Out-Null
    } catch {}
    return $true
}

Set-PSReadLineKeyHandler -Chord 'Ctrl+g' -ScriptBlock {
    $cwd = (Get-Location).Path
    $suggestion = & cmdtrail suggest --cwd $cwd --pick 2>$null
    if ($LASTEXITCODE -eq 0 -and $suggestion) {
        [Microsoft.PowerShell.PSConsoleReadLine]::RevertLine()
        [Microsoft.PowerShell.PSConsoleReadLine]::Insert($suggestion)
    }
}
