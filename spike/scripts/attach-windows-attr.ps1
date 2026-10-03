<#
Starts an ETW session on the anyprobe `attr` example (`#[probe]`) and checks
what the decoded trace holds for each encoding.

Usage (elevated Windows PowerShell; `logman` needs administrator rights):
  powershell -ExecutionPolicy Bypass -File spike\scripts\attach-windows-attr.ps1 [-Profiles release,release-lto]

For each profile it builds the example, starts it, records the provider `attr`
for three seconds with `logman`, decodes the trace with `tracerpt`, and checks
that:
  - native arguments and a native return value are recorded;
  - a `serde` argument is recorded as its JSON;
  - a `debug` return value is recorded as its `{:?}` output;
  - arguments collapsed into one object are recorded as that JSON object;
  - `debug(self)` on a method is recorded as the receiver's `{:?}` output.
Encoding runs only after the enabled check, so any encoded value in the trace
shows that the session turned the check on.
Prints ok/FAIL per check and exits non-zero if any check failed.
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

$work = Join-Path ([IO.Path]::GetTempPath()) "anyprobe-attr-$PID"
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

# A double quote in element text may be written as itself or as &quot;.
$q = '(?:"|&quot;)'

try {
    foreach ($p in $Profiles) {
        Write-Host "== $p (anyprobe example attr)"
        $script:profileFailed = $false
        cargo build -q -p anyprobe --example attr --profile $p
        if ($LASTEXITCODE -ne 0) {
            Write-Host '  FAIL  build'
            $script:failed = $true
            continue
        }
        $bin = "target\$p\examples\attr.exe"
        $log = Join-Path $work "$p.log"
        $etl = Join-Path $work "$p.etl"
        $xml = Join-Path $work "$p.xml"
        $session = "anyprobe-attr-$p"

        $proc = Start-Process -FilePath $bin -ArgumentList '0', '20' `
            -RedirectStandardOutput $log -NoNewWindow -PassThru
        Start-Sleep -Seconds 2
        $match = Select-String -Path $log -Pattern 'etw-guid=(\{[0-9a-fA-F-]+\})'
        Expect 'example printed its provider GUID' ($null -ne $match)
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
        $trace = ''
        if (Test-Path $xml) { $trace = Get-Content $xml -Raw }

        Expect 'native arguments: id and path' ($trace -match '>/index<')
        Expect 'native return value' ($trace -match 'Name="ret"[^>]*>[0-9]*6<')
        Expect 'serde argument as JSON' `
            ($trace -match ">\{${q}table${q}:${q}rows${q},${q}limit${q}:5\}<")
        Expect 'debug return value, Ok' ($trace -match '>Ok\(5\)<')
        Expect 'debug return value, Err' ($trace -match ">Err\(${q}odd [0-9]+${q}\)<")
        Expect 'collapsed arguments as one JSON object' `
            ($trace -match ">\{${q}id${q}:[0-9]+,${q}a${q}:${q}x${q},${q}b${q}:${q}y${q},${q}c${q}:${q}z${q},${q}tags${q}:")
        Expect 'debug(self) and a native argument' `
            ($trace -match '>Counter \{ n: [0-9]+ \}<')

        if ($script:profileFailed) {
            $script:failed = $true
            Write-Host '  --- example output'
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

if ($script:failed) { Write-Host 'FAIL'; exit 1 }
Write-Host 'PASS'
