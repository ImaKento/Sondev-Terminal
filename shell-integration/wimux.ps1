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
        $global:__wimux_conda_root = @($__wimux_conda_roots)[0]

        function global:conda {
            $root = $global:__wimux_conda_root
            $exe = Join-Path $root "Scripts\conda.exe"
            $py = Join-Path $root "python.exe"
            $hook = (& $exe "shell.powershell" "hook" 2>$null) | Out-String
            $viaPython = $false
            if (-not $hook.Trim() -and (Test-Path -LiteralPath $py)) {
                # conda.exe (the launcher) may fail on some setups
                # ("invalid choice: ''"). Run conda through python instead.
                $hook = (& $py -m conda "shell.powershell" "hook" 2>$null) | Out-String
                $viaPython = [bool]$hook.Trim()
            }
            if (-not $hook.Trim()) {
                Write-Host "conda: could not initialize conda ($root)" -ForegroundColor Red
                return
            }
            # Import the conda module globally (we are inside a function scope here).
            # It defines the alias `conda` -> Invoke-Conda, which takes precedence over this stub.
            $hook = $hook -replace 'Import-Module ', 'Import-Module -Global '
            # The hook ends with "conda activate base" (auto_activate_base). Run it only
            # after __wimux_conda_fix, otherwise it fails on PowerShell 7 ("invalid choice: ''").
            $lines = $hook -split "`r?`n"
            $deferred = @($lines | Where-Object { $_ -match '^\s*conda\s+activate\b' })
            $hook = ($lines | Where-Object { $_ -notmatch '^\s*conda\s+activate\b' }) -join "`n"
            Invoke-Expression $hook
            __wimux_conda_fix -Force:$viaPython
            # Skip the base activation when the user is activating / deactivating anyway
            if ($args.Count -eq 0 -or ($args[0] -ne 'activate' -and $args[0] -ne 'deactivate')) {
                foreach ($line in $deferred) { Invoke-Expression $line }
            }
            Remove-Item Function:\conda -ErrorAction SilentlyContinue
            conda @args
        }
    }
    Remove-Variable __wimux_conda_roots -ErrorAction SilentlyContinue
}

# PowerShell 7: conda's module runs "& $Env:CONDA_EXE $Env:_CE_M $Env:_CE_CONDA <args>"
# with _CE_M / _CE_CONDA empty, and newer PowerShell passes them to conda.exe as ""
# ("invalid choice: ''", then "Invoke-Expression: ... empty string").
# Route conda through "python -m conda" so no empty argument is passed.
# Also applies when conda was set up by the user's profile (conda init).
function global:__wimux_conda_via_python {
    # activation resets these, so set them on every call
    $Env:CONDA_EXE = $global:__wimux_conda_py
    $Env:_CE_M = "-m"
    $Env:_CE_CONDA = "conda"
    Invoke-Conda @args
}

function global:__wimux_conda_fix([switch]$Force) {
    if (-not $Force -and $PSVersionTable.PSVersion.Major -lt 7) { return }
    if (-not (Get-Command Invoke-Conda -ErrorAction SilentlyContinue)) { return }
    $py = $Env:CONDA_PYTHON_EXE
    if ((-not $py -or -not (Test-Path -LiteralPath $py)) -and $Env:CONDA_EXE) {
        # <root>\Scripts\conda.exe -> <root>\python.exe
        $py = Join-Path (Split-Path -Parent (Split-Path -Parent $Env:CONDA_EXE)) "python.exe"
    }
    if (-not $py -or -not (Test-Path -LiteralPath $py)) { return }
    $global:__wimux_conda_py = $py
    Set-Alias -Name conda -Value __wimux_conda_via_python -Scope Global -Force
}

# conda set up by the user's profile: fix it now
__wimux_conda_fix

# ---------------------------------------------------------------------------
# cdi: pick a folder from wimux's directory history, inside the terminal.
#   cdi            -> pick from the list (most frequent / recent first)
#   cdi <words>    -> start with <words> as the filter
# Uses fzf (on PATH or bundled with the app), otherwise a small built-in picker.
# The list is written by wimux to %APPDATA%\wimux\dirs.txt.
# ---------------------------------------------------------------------------
function global:__wimux_pick([string[]]$items, [string]$query, [string]$name = "cdi", [string]$action = "cd") {
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
        $out += "$esc[36m$name>$esc[0m $q$esc[90m  (Enter: $action / Esc: cancel)$esc[0m`n"
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

# Pick one of $items with fzf (or the built-in picker). Returns $null when cancelled.
function global:__wimux_select([string[]]$items, [string]$query, [string]$name, [string]$action) {
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
            $psi.Arguments = '--height 40% --reverse --prompt "' + $name + '> "'
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
            $target = __wimux_pick $items $query $name $action
        }
    } else {
        $target = __wimux_pick $items $query $name $action
    }
    if ($target) { return $target }
    return $null
}

function global:cdi {
    $dir = if ($env:WIMUX_CONFIG_DIR) { $env:WIMUX_CONFIG_DIR } else { Join-Path $env:APPDATA "SondevTerminal" }
    $file = Join-Path $dir "dirs.txt"
    if (-not (Test-Path -LiteralPath $file)) {
        Write-Host "cdi: no history yet (move around with cd first)"
        return
    }
    $items = @(Get-Content -LiteralPath $file -Encoding UTF8 | Where-Object {
        $_ -and (Test-Path -LiteralPath $_ -PathType Container)
    })
    $query = $args -join " "

    $target = __wimux_select $items $query "cdi" "cd"
    if ($target) { Set-Location -LiteralPath $target }
}

# ---------------------------------------------------------------------------
# clist: pick a conda environment and activate it (instead of `conda info -e`
# followed by `conda activate <name>`).
#   clist          -> pick from the list (the active one is marked with a trailing *)
#   clist <words>  -> start with <words> as the filter
# The list is read from the conda folders directly (fast; conda is not started):
# <root>\envs\* and ~\.conda\environments.txt.
# ---------------------------------------------------------------------------
function global:__wimux_conda_envs {
    $roots = @()
    if ($global:__wimux_conda_root) { $roots += $global:__wimux_conda_root }
    # CONDA_EXE is <root>\Scripts\conda.exe, or <root>\python.exe after __wimux_conda_fix
    $exes = @($env:CONDA_EXE) + @(Get-Command conda.exe, conda.bat -CommandType Application -ErrorAction SilentlyContinue |
        ForEach-Object { $_.Source })
    foreach ($e in $exes) {
        if (-not $e) { continue }
        $d = Split-Path -Parent $e
        if ((Split-Path -Leaf $e) -ne "python.exe") { $d = Split-Path -Parent $d }
        $roots += $d
    }
    if ($env:WIMUX_CONDA_ROOT) { $roots += $env:WIMUX_CONDA_ROOT }
    $roots = @($roots | Where-Object { $_ -and (Test-Path -LiteralPath (Join-Path $_ "conda-meta")) } |
        ForEach-Object { (Resolve-Path -LiteralPath $_).ProviderPath.TrimEnd('\') } | Select-Object -Unique)

    $paths = New-Object System.Collections.Generic.List[string]
    foreach ($r in $roots) {
        $paths.Add($r)
        $envs = Join-Path $r "envs"
        if (Test-Path -LiteralPath $envs) {
            Get-ChildItem -LiteralPath $envs -Directory -ErrorAction SilentlyContinue |
                Where-Object { Test-Path -LiteralPath (Join-Path $_.FullName "conda-meta") } |
                ForEach-Object { $paths.Add($_.FullName) }
        }
    }
    $txt = Join-Path $HOME ".conda\environments.txt"
    if (Test-Path -LiteralPath $txt) {
        Get-Content -LiteralPath $txt -Encoding UTF8 -ErrorAction SilentlyContinue |
            Where-Object { $_ -and (Test-Path -LiteralPath (Join-Path $_ "conda-meta")) } |
            ForEach-Object { $paths.Add($_.TrimEnd('\')) }
    }

    # name: "base" for a root, the folder name for <root>\envs\<name>, else the full path
    $seen = @{}
    $out = @()
    foreach ($p in $paths) {
        $key = $p.ToLower()
        if ($seen.ContainsKey($key)) { continue }
        $seen[$key] = $true
        $parent = Split-Path -Parent $p
        $name = $p
        if ($roots -contains $p) {
            $name = if ($roots[0] -eq $p) { "base" } else { $p }
        } elseif ((Split-Path -Leaf $parent) -eq "envs" -and ($roots -contains (Split-Path -Parent $parent))) {
            $name = Split-Path -Leaf $p
        }
        $out += [pscustomobject]@{ Name = $name; Path = $p }
    }
    return $out
}

function global:clist {
    $envs = @(__wimux_conda_envs)
    if ($envs.Count -eq 0) {
        Write-Host "clist: no conda environments found (set `$env:WIMUX_CONDA_ROOT to the conda folder)"
        return
    }
    $current = "$env:CONDA_PREFIX".TrimEnd('\').ToLower()
    $items = @($envs | ForEach-Object {
        if ($_.Path.ToLower() -eq $current) { "$($_.Name) *" } else { $_.Name }
    })
    $picked = __wimux_select $items ($args -join " ") "clist" "activate"
    if (-not $picked) { return }
    conda activate ($picked -replace '\s*\*$', '')
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
    __wimux_agent_started "claude"
    $extra = @()
    # Sondev's browser (MCP "sondev-browser"), when "AI browser control" is on.
    # Not for subcommands (claude mcp ..., claude config ...).
    # --mcp-config takes several files, so it must be followed by another option (--settings).
    $mcp = $env:WIMUX_CLAUDE_MCP
    $settings = $env:WIMUX_CLAUDE_SETTINGS
    $has_settings = $settings -and (Test-Path -LiteralPath $settings)
    $sub = @("mcp", "config", "update", "doctor", "install", "setup-token", "plugin", "migrate-installer")
    $is_sub = $args.Count -gt 0 -and ($sub -contains "$($args[0])")
    if ($mcp -and $has_settings -and -not $is_sub -and (Test-Path -LiteralPath $mcp)) {
        $extra += @("--mcp-config", $mcp)
    }
    if ($has_settings) {
        $extra += @("--settings", $settings)
    }
    & $exe.Source @extra @args
}

# ---------------------------------------------------------------------------
# Other AI agents (codex, gemini, aider ...): tell wimux which agent runs in
# this pane, so the sidebar shows its state (working / waiting) and wimux
# notifies when it finishes. Commands the user already defined as a function
# or alias are left alone.
# Codex: also hand it wimux's notify program (unless config.toml sets notify),
# so the "turn finished" notification is exact.
# ---------------------------------------------------------------------------
function global:__wimux_agent_started([string]$name) {
    $b64 = [Convert]::ToBase64String([Text.Encoding]::UTF8.GetBytes($name))
    [Console]::Write("$([char]0x1b)]1337;SetUserVar=WIMUX_AGENT=$b64$([char]0x07)")
}

function global:__wimux_run_agent {
    $name = $args[0]
    $rest = @($args | Select-Object -Skip 1)
    $exe = Get-Command $name -CommandType Application, ExternalScript -ErrorAction SilentlyContinue |
        Select-Object -First 1
    if (-not $exe) {
        Write-Host "${name}: command not found"
        return
    }
    $extra = @()
    if ($name -eq "codex" -and $env:WIMUX_EXE -and -not $env:WIMUX_EXE.Contains("'")) {
        $home_dir = if ($env:CODEX_HOME) { $env:CODEX_HOME } else { Join-Path $HOME ".codex" }
        $cfg = Join-Path $home_dir "config.toml"
        $has_notify = (Test-Path -LiteralPath $cfg) -and
            (Select-String -LiteralPath $cfg -Pattern '^\s*notify\s*=' -Quiet)
        if (-not $has_notify) {
            $exe_path = $env:WIMUX_EXE.Replace('\', '/')
            $extra = @("-c", "notify=['$exe_path','--codex-notify']")
        }
    }
    __wimux_agent_started $name
    & $exe.Source @extra @rest
}

foreach ($__wimux_name in @("codex", "gemini", "aider", "opencode", "copilot", "cursor-agent",
                            "qwen", "amp", "crush", "goose", "kiro-cli")) {
    if (-not (Get-Command $__wimux_name -CommandType Function, Alias -ErrorAction SilentlyContinue)) {
        $__wimux_body = [scriptblock]::Create("__wimux_run_agent '$__wimux_name' @args")
        Set-Item -Path "Function:global:$__wimux_name" -Value $__wimux_body
    }
}
Remove-Variable __wimux_name, __wimux_body -ErrorAction SilentlyContinue

# sondev notify [--title <title>] <message>: show a notification from this pane
# (e.g. at the end of a long build:  cargo build; sondev notify "build done")
if ($env:WIMUX_EXE) {
    function global:sondev { & $env:WIMUX_EXE @args }
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
