# SLOPP installer for Windows.
#
#   irm https://slopp.gs0.me/install.ps1 | iex
#
# It installs SLOPP from the latest release with its setup, checked against
# the release's SHA-256 digest and run silently, then has sloppctl install
# the servers it drives that are missing, the same way the panel's Install
# button does: Apache, PHP, MariaDB and Mailpit with winget (MariaDB's
# installer asks for permission). Running it again updates SLOPP; the setup
# closes a running panel, and the servers keep running. Options need the
# script as a script block:
#
#   & ([scriptblock]::Create((irm https://slopp.gs0.me/install.ps1))) -NoDeps
#
#   -NoDeps           install SLOPP only, leave the servers alone
#   -Yes              don't ask before installing or uninstalling
#   -Version X.Y.Z    install a specific release instead of the latest
#   -AllUsers         install for everyone (asks for admin rights)
#   -Force            reinstall the version that is already installed
#   -Uninstall        stop the servers and remove SLOPP; the stack (sites,
#                     databases) and the settings stay
#
# Also honoured: SLOPP_VERSION as the same as -Version. Works in Windows
# PowerShell 5.1 and PowerShell 7.

param(
    [switch]$NoDeps,
    [switch]$Yes,
    [string]$Version = $env:SLOPP_VERSION,
    [switch]$AllUsers,
    [switch]$Force,
    [switch]$Uninstall
)

# Everything runs inside this function, so `irm | iex` leaves no settings
# behind in the caller's session, and errors end it without closing the
# window as `exit` would.
function Invoke-SloppInstaller {
    param($NoDeps, $Yes, $Version, $AllUsers, $Force, $Uninstall)

    Set-StrictMode -Version 2.0
    $ErrorActionPreference = 'Stop'
    # Windows PowerShell's progress bar slows downloads to a crawl.
    $ProgressPreference = 'SilentlyContinue'
    # Windows PowerShell 5.1 doesn't offer TLS 1.2 by itself; GitHub needs it.
    [Net.ServicePointManager]::SecurityProtocol =
        [Net.ServicePointManager]::SecurityProtocol -bor [Net.SecurityProtocolType]::Tls12

    $repo = 'gabors0/slopp'
    $api = "https://api.github.com/repos/$repo/releases"
    # The setup's AppId (slopp.iss) and the uninstall key Inno Setup gives it.
    $appId = '{21EC18CB-2262-45D5-848B-E315EB0C19B2}'
    $uninstallKeys = @(
        "HKCU:\Software\Microsoft\Windows\CurrentVersion\Uninstall\${appId}_is1",
        "HKLM:\Software\Microsoft\Windows\CurrentVersion\Uninstall\${appId}_is1",
        "HKLM:\Software\WOW6432Node\Microsoft\Windows\CurrentVersion\Uninstall\${appId}_is1"
    )

    function Say($text) { Write-Host "==> $text" -ForegroundColor White }
    function Note($text) { Write-Host $text -ForegroundColor DarkGray }
    function Warn($text) { Write-Host "warning: $text" -ForegroundColor Yellow }

    function Confirm-Step($question) {
        if ($Yes) { return $true }
        try {
            $reply = Read-Host "$question [Y/n]"
        } catch {
            # No one to ask (a non-interactive session): go ahead.
            return $true
        }
        return -not ($reply -match '^\s*[Nn]')
    }

    # The installed SLOPP: its folder, version, uninstaller and whether it's
    # for everyone; $null when there is none.
    function Get-Installed {
        foreach ($key in $uninstallKeys) {
            $entry = Get-ItemProperty -Path $key -ErrorAction SilentlyContinue
            if ($entry) {
                return [pscustomobject]@{
                    Dir = $entry.InstallLocation.TrimEnd('\')
                    Version = $entry.DisplayVersion
                    Uninstaller = $entry.UninstallString.Trim('"')
                    AllUsers = $key.StartsWith('HKLM:')
                }
            }
        }
        return $null
    }

    function Test-SloppRunning {
        return [bool](Get-Process -Name slopp -ErrorAction SilentlyContinue)
    }

    # sloppctl's lines for a command, or $null when it doesn't run.
    function Invoke-Ctl($ctl, [string[]]$arguments) {
        if (-not (Test-Path -LiteralPath $ctl)) { return $null }
        # Windows PowerShell turns a redirected stderr line into an error,
        # which 'Stop' would make fatal.
        $ErrorActionPreference = 'Continue'
        try {
            $out = & $ctl @arguments 2>$null
        } catch {
            return $null
        }
        if ($LASTEXITCODE -ne 0) { return $null }
        return @($out)
    }

    # A line of `sloppctl paths`: "Stack root", "Settings".
    function Get-CtlPath($ctl, $label) {
        $lines = Invoke-Ctl $ctl @('paths')
        if (-not $lines) { return $null }
        foreach ($line in $lines) {
            if ($line -match "^$label\s+(.+)$") { return $Matches[1] }
        }
        return $null
    }

    # The servers sloppctl doesn't find, as @('Apache', 'MariaDB'); $null
    # when sloppctl doesn't run.
    function Get-MissingServers($ctl) {
        $lines = Invoke-Ctl $ctl @('paths')
        if ($null -eq $lines) { return $null }
        $names = @{ 'httpd' = 'Apache'; 'PHP' = 'PHP'; 'mariadbd' = 'MariaDB'; 'mailpit' = 'Mailpit' }
        $missing = @()
        foreach ($line in $lines) {
            if ($line -match '^(\S+)\s+not found$' -and $names.ContainsKey($Matches[1])) {
                $missing += $names[$Matches[1]]
            }
        }
        return , $missing
    }

    function Join-Names([string[]]$names) {
        if ($names.Count -le 1) { return $names -join '' }
        return ($names[0..($names.Count - 2)] -join ', ') + ' and ' + $names[-1]
    }

    # sloppctl installs what's missing with winget, as the panel's Install
    # button does, so the package ids and the advice live in one place.
    function Install-Servers($ctl) {
        if ($NoDeps) { return }
        $missing = Get-MissingServers $ctl
        if ($null -eq $missing) {
            Warn "sloppctl doesn't run here, so the servers weren't checked"
            return
        }
        if ($missing.Count -eq 0) {
            Note 'the servers are already installed'
            return
        }
        $list = Join-Names $missing
        if (-not (Confirm-Step "Install $($list)?")) {
            Note "skipping the servers; SLOPP's Install button gets them later"
            return
        }
        Say "Installing $list"
        & $ctl install-servers
        if ($LASTEXITCODE -ne 0) {
            Warn "not everything was installed (see above); SLOPP's Install button can try again"
        }
    }

    function Install-Slopp {
        Say 'Finding SLOPP'
        $headers = @{ 'User-Agent' = 'slopp-installer' }
        try {
            if ($Version) {
                $release = Invoke-RestMethod -UseBasicParsing -Headers $headers "$api/tags/v$Version"
            } else {
                $release = Invoke-RestMethod -UseBasicParsing -Headers $headers "$api/latest"
            }
        } catch {
            if ($Version) { throw "release v$Version not found ($($_.Exception.Message))" }
            throw "could not reach GitHub ($($_.Exception.Message))"
        }
        $tag = $release.tag_name
        $ver = $tag.TrimStart('v')
        Note "release $tag"

        $installed = Get-Installed
        $wasRunning = Test-SloppRunning
        if ($installed -and $installed.Version -eq $ver -and -not $Force) {
            Note "SLOPP $ver is already installed in $($installed.Dir) (-Force reinstalls it)"
        } else {
            $asset = $release.assets | Where-Object { $_.name -like '*-windows-x86_64-setup.exe' } |
                Select-Object -First 1
            if (-not $asset) { throw "release $tag has no setup" }
            $digest = $null
            if ($asset.PSObject.Properties['digest'] -and $asset.digest -match '^sha256:([0-9a-fA-F]{64})$') {
                $digest = $Matches[1]
            }
            if (-not $digest) { throw "release $tag has no checksum for its setup" }

            Say "Downloading SLOPP $ver"
            $tmp = Join-Path ([IO.Path]::GetTempPath()) ("slopp-" + [guid]::NewGuid())
            New-Item -ItemType Directory -Path $tmp | Out-Null
            try {
                $setup = Join-Path $tmp $asset.name
                Invoke-WebRequest -UseBasicParsing -Headers $headers -Uri $asset.browser_download_url -OutFile $setup
                $actual = (Get-FileHash -Algorithm SHA256 -Path $setup).Hash
                if ($actual -ne $digest) {
                    throw "the download doesn't match the release's checksum (expected $digest, got $($actual.ToLower()))"
                }

                # An existing install keeps its scope; upgrading one for
                # everyone needs admin rights, which the setup asks for.
                $forAll = $AllUsers -or ($installed -and $installed.AllUsers)
                $setupArgs = @('/VERYSILENT', '/SUPPRESSMSGBOXES', '/NORESTART', '/SP-')
                if ($forAll) { $setupArgs += '/ALLUSERS' } else { $setupArgs += '/CURRENTUSER' }
                if ($wasRunning) { Note 'the setup closes the running SLOPP; the servers keep running' }
                Say "Installing SLOPP $ver"
                $process = Start-Process -FilePath $setup -ArgumentList $setupArgs -Wait -PassThru
                if ($process.ExitCode -ne 0) {
                    throw "the setup failed (exit code $($process.ExitCode))"
                }
            } finally {
                Remove-Item -Recurse -Force -Path $tmp -ErrorAction SilentlyContinue
            }
            $installed = Get-Installed
            if (-not $installed) { throw "the setup finished, but SLOPP isn't registered as installed" }
            Say "Installed SLOPP $ver in $($installed.Dir)"
        }

        $ctl = Join-Path $installed.Dir 'sloppctl.exe'
        Install-Servers $ctl

        Note "open SLOPP from the Start menu, or run '$ctl status' in a terminal"
        if (-not (Test-SloppRunning) -and (Confirm-Step 'Open SLOPP now?')) {
            Start-Process -FilePath (Join-Path $installed.Dir 'slopp.exe')
        }
    }

    function Uninstall-Slopp {
        $installed = Get-Installed
        if (-not $installed) { throw "SLOPP isn't installed" }
        if (-not (Confirm-Step "Stop the servers and remove SLOPP from $($installed.Dir)?")) { return }

        # The uninstaller stops the servers too, but quietly; this shows it
        # and says where the stack stays.
        $ctl = Join-Path $installed.Dir 'sloppctl.exe'
        $root = Get-CtlPath $ctl 'Stack root'
        $settings = Get-CtlPath $ctl 'Settings'
        if (Test-Path -LiteralPath $ctl) {
            Say 'Stopping the servers'
            & $ctl stop
            if ($LASTEXITCODE -ne 0) { Warn "not every server stopped; see '$ctl status'" }
        }
        # The uninstaller can't remove a running panel's program.
        if (Test-SloppRunning) {
            Note 'closing the running SLOPP'
            Get-Process -Name slopp -ErrorAction SilentlyContinue | Stop-Process -Force
            Start-Sleep -Milliseconds 500
        }

        Say 'Removing SLOPP'
        $uninstallArgs = @('/VERYSILENT', '/SUPPRESSMSGBOXES', '/NORESTART')
        Start-Process -FilePath $installed.Uninstaller -ArgumentList $uninstallArgs -Wait
        # The uninstaller hands over to a copy of itself and returns at once:
        # wait for the copy to unregister SLOPP.
        $deadline = (Get-Date).AddMinutes(2)
        while ((Get-Installed) -and (Get-Date) -lt $deadline) { Start-Sleep -Milliseconds 500 }
        if (Get-Installed) { throw "SLOPP is still installed; remove it in Settings, Apps" }
        if (Test-Path -LiteralPath (Join-Path $installed.Dir 'slopp.exe')) {
            Warn "the uninstaller left files in $($installed.Dir); delete the folder"
        }
        Say 'Removed SLOPP'
        $kept = @()
        if ($root) { $kept += "the stack in $root" }
        if ($settings) { $kept += "the settings in $settings" }
        if ($kept.Count -gt 0) { Note "kept $($kept -join ' and '); delete them to remove everything" }
    }

    if ([Environment]::OSVersion.Version.Major -lt 10) { throw 'SLOPP needs Windows 10 or newer' }
    if (-not [Environment]::Is64BitOperatingSystem) { throw 'SLOPP needs 64-bit Windows' }
    $Version = "$Version".TrimStart('v')

    if ($Uninstall) { Uninstall-Slopp } else { Install-Slopp }
    Say 'Done'
}

try {
    Invoke-SloppInstaller -NoDeps $NoDeps -Yes $Yes -Version $Version -AllUsers $AllUsers `
        -Force $Force -Uninstall $Uninstall
} catch {
    Write-Host "error: $($_.Exception.Message)" -ForegroundColor Red
}
