<#
.SYNOPSIS
Attaches an ETW session to the running spike and checks what it records.

.DESCRIPTION
For each profile it builds the spike, starts it, reads the provider GUID the
spike prints, runs a logman session against that GUID for three seconds, stops
it, decodes the trace with tracerpt, and checks that:
  - the provider saw the session start (entry-enabled=true) and stop;
  - the trace holds work__entry events for every label, about equally often;
  - the trace holds work__return events.
Prints ok/FAIL per check and exits non-zero if any check failed.

Run from an elevated (administrator) prompt: logman needs it.

.PARAMETER Profiles
Cargo profiles to test. Defaults to release and release-lto.

Set ATTACH_CRATE=anyprobe to check the anyprobe crate's `work` example, which
has the spike's provider, probes and command line, instead of the spike.

.EXAMPLE
pwsh spike\scripts\attach-windows.ps1
#>
param([string[]] $Profiles = @('release', 'release-lto'))

$ErrorActionPreference = 'Stop'
Set-Location (Join-Path $PSScriptRoot '..\..')

$identity = [Security.Principal.WindowsIdentity]::GetCurrent()
$isAdmin = ([Security.Principal.WindowsPrincipal] $identity).IsInRole(
    [Security.Principal.WindowsBuiltInRole]::Administrator)
if (-not $isAdmin) {
    Write-Host 'Run from an elevated prompt: logman needs administrator rights.'
    exit 2
}

if ($env:ATTACH_CRATE -eq 'anyprobe') {
    $packageArgs = @('-p', 'anyprobe', '--example', 'work')
    $binRel = 'examples\work.exe'
    $checked = 'anyprobe example work'
} else {
    $packageArgs = @('-p', 'anyprobe-spike')
    $binRel = 'anyprobe-spike.exe'
    $checked = 'anyprobe-spike'
}

$work = Join-Path ([IO.Path]::GetTempPath()) "anyprobe-spike-$PID"
New-Item -ItemType Directory -Force $work | Out-Null
$script:failed = $false

function Expect([string] $What, [bool] $Ok) {
    if ($Ok) {
        Write-Host "  ok    $What"
    } else {
        Write-Host "  FAIL  $What"
        $script:profileFailed = $true
    }
}

try {
    foreach ($p in $Profiles) {
        Write-Host "== $p ($checked)"
        $script:profileFailed = $false
        cargo build -q @packageArgs --profile $p
        if ($LASTEXITCODE -ne 0) {
            Write-Host '  FAIL  build'
            $script:failed = $true
            continue
        }
        $bin = "target\$p\$binRel"
        $log = Join-Path $work "$p.log"
        $etl = Join-Path $work "$p.etl"
        $xml = Join-Path $work "$p.xml"
        $session = "anyprobe-spike-$p"

        $proc = Start-Process -FilePath $bin -ArgumentList '0', '20' `
            -RedirectStandardOutput $log -NoNewWindow -PassThru
        Start-Sleep -Seconds 2
        $match = Select-String -Path $log -Pattern 'etw-guid=(\{[0-9a-fA-F-]+\})'
        Expect 'probe binary printed its provider GUID' ($null -ne $match)
        if ($null -eq $match) {
            Stop-Process -Id $proc.Id
            Get-Content $log
            $script:failed = $true
            continue
        }
        $guid = $match.Matches[0].Groups[1].Value

        logman create trace $session -p $guid 0xffffffffffffffff 0xff -o $etl -ets | Out-Null
        Start-Sleep -Seconds 3
        logman stop $session -ets | Out-Null
        Start-Sleep -Seconds 1
        Stop-Process -Id $proc.Id

        $etlFile = Get-ChildItem -Path $work -Filter "$p*.etl" | Select-Object -First 1
        Expect 'session wrote a trace file' ($null -ne $etlFile)
        if ($null -ne $etlFile) {
            tracerpt $etlFile.FullName -o $xml -of XML -y | Out-Null
        }

        Expect 'session start turned the probe on in the process' `
            (Select-String -Path $log -Pattern 'entry-enabled=true' -Quiet)
        Expect 'session stop turned it off again' `
            (Select-String -Path $log -Pattern 'entry-enabled=false' -Quiet)

        $counts = @()
        foreach ($label in 'first-site', 'second-site', 'u32', 'u64') {
            $n = 0
            if (Test-Path $xml) {
                $n = (Select-String -Path $xml -Pattern ">$label<" -CaseSensitive -AllMatches |
                    ForEach-Object { $_.Matches.Count } | Measure-Object -Sum).Sum
            }
            Expect "recorded label '$label' ($n events)" ($n -gt 0)
            $counts += $n
        }
        $spread = ($counts | Measure-Object -Maximum).Maximum - ($counts | Measure-Object -Minimum).Minimum
        Expect 'every label recorded equally often (within 1)' ($spread -le 1)
        Expect 'recorded work__return events' `
            ((Test-Path $xml) -and (Select-String -Path $xml -Pattern 'work__return' -Quiet))

        if ($script:profileFailed) {
            $script:failed = $true
            Write-Host '  --- spike output'
            Get-Content $log
            if (Test-Path $xml) {
                Write-Host '  --- decoded trace (first 80 lines)'
                Get-Content $xml -TotalCount 80
            }
        }
    }
} finally {
    Remove-Item -Recurse -Force $work -ErrorAction SilentlyContinue
}

if ($script:failed) {
    Write-Host 'FAIL'
    exit 1
}
Write-Host 'PASS'
exit 0
