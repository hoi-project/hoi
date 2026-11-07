#!/usr/bin/env pwsh
# hoi-sandbox.ps1 — run the hoi sandbox from PowerShell on Windows.
#
# The sandbox itself is a bash script, so PowerShell cannot run it directly — this is why
#     ./sandbox/hoi-sandbox up demo
# fails with "not recognized as the name of a cmdlet". This wrapper forwards your arguments to the
# bash script through Git Bash or WSL. Run it from the repo root:
#
#     ./sandbox/hoi-sandbox.ps1 up demo
#     ./sandbox/hoi-sandbox.ps1 pentest demo red=1 blue=1
#
# Needs Docker Desktop plus either Git for Windows (Git Bash) or WSL. For `pentest`, WSL is
# recommended; it also needs the `claude` CLI and Node available in the shell that runs.
$ErrorActionPreference = 'Stop'
# The complete stack uses a native Node launcher; no Bash/WSL is needed for it.
if ($args.Count -eq 0 -or $args[0] -in @('dashboard', 'start', 'stop', 'status') -or ($args[0] -eq 'up' -and $args.Count -eq 1)) {
    if (-not (Get-Command node -ErrorAction SilentlyContinue)) {
        throw 'Node 22 or newer is required. Install Node, then rerun ./sandbox/hoi-sandbox.ps1 dashboard.'
    }
    & node (Join-Path $PSScriptRoot 'launch.mjs') @args
    exit $LASTEXITCODE
}
# Git Bash and WSL both want a forward-slash path; a Windows backslash path gets its
# separators swallowed (G:\...\hoi-sandbox -> G:...hoi-sandbox -> "No such file or directory").
$script = (Join-Path $PSScriptRoot 'hoi-sandbox') -replace '\\', '/'

# On Windows, C:\Windows\System32\bash.exe is the WSL launcher, not Git Bash. It cannot take a
# Windows path like G:/..., so only use `bash` directly when it is real Git Bash; otherwise fall
# through to WSL and translate the path with wslpath so it works regardless of the current dir.
$bash = Get-Command bash -ErrorAction SilentlyContinue
$isWslBash = $bash -and ((Split-Path $bash.Source -Parent) -ieq (Join-Path $env:SystemRoot 'System32'))
if (-not $bash -or $isWslBash) {
    $gitBashPaths = @((Join-Path $env:ProgramFiles 'Git\bin\bash.exe'), (Join-Path $env:LOCALAPPDATA 'Programs\Git\bin\bash.exe'))
    foreach ($gitBashPath in $gitBashPaths) {
        if (Test-Path -LiteralPath $gitBashPath) {
            $bash = Get-Command $gitBashPath
            $isWslBash = $false
            break
        }
    }
}
if ($bash -and -not $isWslBash) {
    & $bash.Source $script @args
    exit $LASTEXITCODE
}

$wsl = Get-Command wsl -ErrorAction SilentlyContinue
if ($wsl) {
    $wslScript = (& $wsl.Source wslpath -a $script).Trim()
    & $wsl.Source bash $wslScript @args
    exit $LASTEXITCODE
}

Write-Error "need bash to run the hoi sandbox on Windows. Install Git for Windows (Git Bash) or enable WSL, then re-run from the repo root, e.g.: ./sandbox/hoi-sandbox.ps1 up demo"
exit 1
