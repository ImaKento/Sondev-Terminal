# wimux shell integration (PowerShell 7 / Windows PowerShell 5.1)
#
# NOTE: Keep this file ASCII-only. Windows PowerShell 5.1 reads BOM-less
#       files as Shift_JIS, and multibyte comments break the parser.
#
# wimux loads this automatically:  -NoExit -Command ". '<this file>'"
# Manual use ($PROFILE):  if ($env:WIMUX) { . "C:\path\to\wimux\shell-integration\wimux.ps1" }
#
# On every prompt, report to wimux via OSC sequences:
#   - current directory   OSC 9;9;"<path>"
#   - conda env name      OSC 1337;SetUserVar=WIMUX_CONDA=<base64>
#
# The existing prompt function (conda / oh-my-posh / starship ...) is wrapped, not replaced.

if ($global:__wimux_loaded) { return }
$global:__wimux_loaded = $true

# ---------------------------------------------------------------------------
# conda: if `conda` is not set up in this shell, define a lazy stub.
# The (slow) conda hook runs only on the first `conda` call, so shell startup stays fast.
# Override the install location with $env:WIMUX_CONDA_ROOT if needed.
# ---------------------------------------------------------------------------
if (-not (Get-Command conda -ErrorAction SilentlyContinue)) {
    $__wimux_conda_roots = @(
        $env:WIMUX_CONDA_ROOT,
        "$env:USERPROFILE\anaconda3",
        "$env:USERPROFILE\miniconda3",
        "$env:USERPROFILE\miniforge3",
        "$env:LOCALAPPDATA\anaconda3",
        "$env:LOCALAPPDATA\miniconda3",
        "$env:ProgramData\anaconda3",
        "$env:ProgramData\miniconda3",
        "$env:ProgramData\Anaconda3",
        "C:\anaconda3",
        "C:\miniconda3"
    ) | Where-Object { $_ -and (Test-Path (Join-Path $_ "Scripts\conda.exe")) }

    if ($__wimux_conda_roots) {
        $global:__wimux_conda_exe = Join-Path @($__wimux_conda_roots)[0] "Scripts\conda.exe"

        function global:conda {
            Remove-Item Function:\conda -ErrorAction SilentlyContinue
            $hook = (& $global:__wimux_conda_exe "shell.powershell" "hook") | Out-String
            # Import the conda module globally (we are inside a function scope here)
            $hook = $hook -replace 'Import-Module ', 'Import-Module -Global '
            Invoke-Expression $hook
            conda @args
        }
    }
    Remove-Variable __wimux_conda_roots -ErrorAction SilentlyContinue
}

# ---------------------------------------------------------------------------
# cdi: pick a folder from wimux's directory history, inside the terminal.
#   cdi            -> pick from the list (most frequent / recent first)
#   cdi <words>    -> start with <words> as the filter
# Uses fzf (on PATH or bundled with the app), otherwise a small built-in picker.
# The list is written by wimux to %APPDATA%\wimux\dirs.txt.
# ---------------------------------------------------------------------------
function global:__wimux_pick([string[]]$items, [string]$query) {
    $esc = [char]0x1b
    $max = 12
    $sel = 0
    $q = $query
    $lines = 0
    while ($true) {
        $words = @($q.ToLower().Split(' ', [StringSplitOptions]::RemoveEmptyEntries))
        $hits = @($items | Where-Object {
            $p = $_.ToLower()
            $ok = $true
            foreach ($w in $words) { if (-not $p.Contains($w)) { $ok = $false; break } }
            $ok
        } | Select-Object -First $max)
        if ($sel -ge $hits.Count) { $sel = [Math]::Max(0, $hits.Count - 1) }

        $out = ""
        if ($lines -gt 0) { $out += "$esc[$($lines)A" }
        $out += "`r$esc[J"
        $out += "$esc[36mcdi>$esc[0m $q$esc[90m  (Enter: cd / Esc: cancel)$esc[0m`n"
        if ($hits.Count -eq 0) {
            $out += "$esc[90m  (no match)$esc[0m`n"
        }
        for ($i = 0; $i -lt $hits.Count; $i++) {
            if ($i -eq $sel) {
                $out += "$esc[7m> $($hits[$i])$esc[0m`n"
            } else {
                $out += "  $($hits[$i])`n"
            }
        }
        [Console]::Write($out)
        $lines = 1 + [Math]::Max(1, $hits.Count)

        $k = [Console]::ReadKey($true)
        $ctrl = ($k.Modifiers -band [ConsoleModifiers]::Control) -ne 0
        if ($k.Key -eq 'Enter') {
            [Console]::Write("$esc[$($lines)A`r$esc[J")
            if ($hits.Count -gt 0) { return $hits[$sel] }
            return $null
        } elseif ($k.Key -eq 'Escape' -or ($ctrl -and $k.Key -eq 'C')) {
            [Console]::Write("$esc[$($lines)A`r$esc[J")
            return $null
        } elseif ($k.Key -eq 'UpArrow' -or ($ctrl -and $k.Key -eq 'P')) {
            if ($sel -gt 0) { $sel-- }
        } elseif ($k.Key -eq 'DownArrow' -or ($ctrl -and $k.Key -eq 'N') -or $k.Key -eq 'Tab') {
            if ($sel -lt $hits.Count - 1) { $sel++ }
        } elseif ($k.Key -eq 'Backspace') {
            if ($q.Length -gt 0) { $q = $q.Substring(0, $q.Length - 1); $sel = 0 }
        } elseif ($ctrl -and $k.Key -eq 'U') {
            $q = ""; $sel = 0
        } elseif (-not [char]::IsControl($k.KeyChar)) {
            $q += $k.KeyChar; $sel = 0
        }
    }
}

# fzf on PATH, otherwise the copy bundled with the app (next to sondev.exe)
function global:__wimux_fzf {
    $c = Get-Command fzf -CommandType Application -ErrorAction SilentlyContinue | Select-Object -First 1
    if ($c) { return $c.Source }
    if ($env:WIMUX_BIN_DIR) {
        $p = Join-Path $env:WIMUX_BIN_DIR "fzf.exe"
        if (Test-Path -LiteralPath $p) { return $p }
    }
    return $null
}

function global:cdi {
    $dir = if ($env:WIMUX_CONFIG_DIR) { $env:WIMUX_CONFIG_DIR } else { Join-Path $env:APPDATA "wimux" }
    $file = Join-Path $dir "dirs.txt"
    if (-not (Test-Path -LiteralPath $file)) {
        Write-Host "cdi: no history yet (move around with cd first)"
        return
    }
    $items = @(Get-Content -LiteralPath $file -Encoding UTF8 | Where-Object {
        $_ -and (Test-Path -LiteralPath $_ -PathType Container)
    })
    $query = $args -join " "

    $target = $null
    $fzf = __wimux_fzf
    if ($fzf) {
        # Run fzf through .NET Process: the list goes in and the result comes out
        # as UTF-8 bytes, so Japanese paths stay intact without touching the console
        # encoding (which breaks native commands on PowerShell 7). Start-Process is
        # not used because it refuses winget's symlinked fzf.exe. fzf draws its UI
        # on the console itself.
        try {
            $psi = New-Object System.Diagnostics.ProcessStartInfo
            $psi.FileName = $fzf
            $psi.Arguments = '--height 40% --reverse --prompt "cdi> "'
            if ($query) { $psi.Arguments += ' --query "' + ($query -replace '"', '') + '"' }
            $psi.UseShellExecute = $false
            $psi.RedirectStandardInput = $true
            $psi.RedirectStandardOutput = $true
            $p = [System.Diagnostics.Process]::Start($psi)
            $bytes = [Text.Encoding]::UTF8.GetBytes((($items -join "`n") + "`n"))
            $p.StandardInput.BaseStream.Write($bytes, 0, $bytes.Length)
            $p.StandardInput.Close()
            $reader = New-Object System.IO.StreamReader($p.StandardOutput.BaseStream, (New-Object System.Text.UTF8Encoding $false))
            $target = $reader.ReadToEnd().Trim()
            $p.WaitForExit()
        } catch {
            # fzf could not be started: fall back to the built-in picker
            $target = __wimux_pick $items $query
        }
    } else {
        $target = __wimux_pick $items $query
    }
    if ($target) { Set-Location -LiteralPath $target }
}

# ---------------------------------------------------------------------------
# claude: run Claude Code with a SessionStart hook that tells wimux the session
# id of this pane, so each tab resumes its own session next time
# (claude --resume <id> instead of claude --continue).
# ---------------------------------------------------------------------------
function global:claude {
    $exe = Get-Command claude -CommandType Application, ExternalScript -ErrorAction SilentlyContinue |
        Select-Object -First 1
    if (-not $exe) {
        Write-Host "claude: Claude Code is not installed"
        return
    }
    $settings = $env:WIMUX_CLAUDE_SETTINGS
    if ($settings -and (Test-Path -LiteralPath $settings)) {
        & $exe.Source --settings $settings @args
    } else {
        & $exe.Source @args
    }
}

# ---------------------------------------------------------------------------
# web: open a browser tab next to this terminal (same pane group).
#   web                 -> empty tab, type a URL in its address bar
#   web 3000            -> http://localhost:3000
#   web github.com      -> https://github.com
#   web rust tutorial   -> web search
# ---------------------------------------------------------------------------
function global:web {
    $req = (@("web") + $args) -join " "
    $b64 = [Convert]::ToBase64String([Text.Encoding]::UTF8.GetBytes($req))
    [Console]::Write("$([char]0x1b)]1337;SetUserVar=WIMUX_CMD=$b64$([char]0x07)")
}

# ---------------------------------------------------------------------------
# prompt
# ---------------------------------------------------------------------------
$global:__wimux_original_prompt = $function:prompt

function global:__wimux_osc([string]$body) {
    # ESC ] <body> ESC \
    return "$([char]0x1b)]$body$([char]0x1b)\"
}

# Current git branch, read from .git/HEAD (no git.exe call, so it is fast).
# Returns $null outside a repository; a short hash on a detached HEAD.
function global:__wimux_git_branch([string]$path) {
    $d = $path
    while ($d) {
        $git = Join-Path $d ".git"
        $head = $null
        if (Test-Path -LiteralPath $git -PathType Container) {
            $head = Join-Path $git "HEAD"
        } elseif (Test-Path -LiteralPath $git -PathType Leaf) {
            # worktree / submodule: "gitdir: <path>"
            $line = Get-Content -LiteralPath $git -TotalCount 1
            if ($line -match '^gitdir:\s*(.+)$') {
                $gd = $Matches[1].Trim()
                if (-not [IO.Path]::IsPathRooted($gd)) { $gd = Join-Path $d $gd }
                $head = Join-Path $gd "HEAD"
            }
        }
        if ($head) {
            if (-not (Test-Path -LiteralPath $head)) { return $null }
            $h = (Get-Content -LiteralPath $head -TotalCount 1).Trim()
            if ($h -like "ref: refs/heads/*") { return $h.Substring(16) }
            return $h.Substring(0, [Math]::Min(7, $h.Length))
        }
        $d = Split-Path -Parent $d
    }
    return $null
}

function global:prompt {
    $osc = ""
    try {
        # current directory (skip non-filesystem locations such as the registry)
        $loc = $executionContext.SessionState.Path.CurrentLocation
        if ($loc.Provider.Name -eq "FileSystem") {
            $osc += __wimux_osc "9;9;`"$($loc.ProviderPath)`""
        }

        # conda env name (empty string when no env is active)
        $conda = "$env:CONDA_DEFAULT_ENV"
        $b64 = [Convert]::ToBase64String([Text.Encoding]::UTF8.GetBytes($conda))
        $osc += __wimux_osc "1337;SetUserVar=WIMUX_CONDA=$b64"
    } catch {
        # never break the prompt because of wimux
    }

    $original = if ($global:__wimux_original_prompt) {
        & $global:__wimux_original_prompt
    } else {
        "PS $($executionContext.SessionState.Path.CurrentLocation)$('>' * ($nestedPromptLevel + 1)) "
    }

    # Show the git branch in the standard "PS path>" prompt: PS C:\repo (main)>
    # Custom prompts (oh-my-posh, starship ...) usually show it already, so they are left as is.
    # Turn off with $env:WIMUX_GIT_PROMPT = "0".
    try {
        if ($env:WIMUX_GIT_PROMPT -ne "0" -and $loc -and $loc.Provider.Name -eq "FileSystem" -and
            "$original" -match '^(.*PS .*?)(>+ ?)$') {
            $pre = $Matches[1]
            $suf = $Matches[2]
            $branch = __wimux_git_branch $loc.ProviderPath
            if ($branch) {
                $e = [char]0x1b
                $original = "$pre $($e)[35m($branch)$($e)[0m$suf"
            }
        }
    } catch {
    }

    # Put OSC at the start of the prompt string (same approach as Windows Terminal docs)
    return $osc + $original
}
