<#
Captures the output shown in docs/usage/windows.md and the attached-cost
numbers in docs/PERFORMANCE.md: the `demo` example under `logman` and `wpr`
(the provider GUID, the decoded events, the profile `cargo anyprobe wprp`
writes) and the `overhead` example with no tracer and with an ETW session
recording every event. Prints everything; checks nothing.

Usage (elevated Windows PowerShell):
  powershell -ExecutionPolicy Bypass -File spike\scripts\capture-docs-windows.ps1 [-Calls 100000] [-BufferKB 1024] [-Buffers 64]

-Calls is the call count for the traced `overhead` runs. Every call fires an
entry and a return probe in each of two functions and the example runs two
passes, so the default writes 800000 events to a temporary .etl file. The
untraced run uses 1000000 calls, as the macOS capture does.
-BufferKB and -Buffers set the buffer size (`logman -bs`) and the minimum buffer
count (`logman -nb`) of the `logman` overhead session. The defaults match the
buffers the profile `cargo anyprobe wprp` writes, so the two sessions are
comparable. After each traced overhead run the script prints the events the
trace holds and the events it lost, from `tracerpt -summary`, next to the
expected count.
#>
param(
    [int] $Calls = 100000,
    [int] $BufferKB = 1024,
    [int] $Buffers = 64
)

$ErrorActionPreference = 'Stop'
Set-Location (Join-Path $PSScriptRoot '..\..')

$identity = [Security.Principal.WindowsIdentity]::GetCurrent()
$isAdmin = ([Security.Principal.WindowsPrincipal] $identity).IsInRole(
    [Security.Principal.WindowsBuiltInRole]::Administrator)
if (-not $isAdmin) {
    Write-Host 'Run from an elevated prompt: wpr and logman need administrator rights.'
    exit 2
}

cargo build -q --release -p anyprobe --example demo --example overhead
if ($LASTEXITCODE -ne 0) { exit 2 }
cargo build -q --release -p cargo-anyprobe
if ($LASTEXITCODE -ne 0) { exit 2 }
$demo = 'target\release\examples\demo.exe'
$over = 'target\release\examples\overhead.exe'
$cli = 'target\release\cargo-anyprobe.exe'

$out = Join-Path ([IO.Path]::GetTempPath()) "anyprobe-docs-$PID"
New-Item -ItemType Directory -Force $out | Out-Null

function Section([string] $Title) { Write-Host "=== $Title" }

# Prints the events in a trace and the events the session lost, next to the
# count the overhead example fires: 2 functions x 2 probes x 2 passes x calls.
function Count-Events([string] $Etl) {
    $summary = Join-Path $out 'summary.txt'
    $dump = Join-Path $out 'dump.csv'
    tracerpt $Etl -summary $summary -o $dump -of CSV -y | Out-Null
    Get-Content $summary | Select-String -Pattern 'Buffers\s+(Lost|Processed)|Events\s+(Lost|Processed)'
    Write-Host ('expected events: {0}' -f (8 * $Calls))
    Remove-Item -Force $summary, $dump -ErrorAction SilentlyContinue
}

Section 'OS, CPU, power plan'
(Get-CimInstance Win32_OperatingSystem | Select-Object Caption, Version, OSArchitecture | Format-List | Out-String).Trim()
(Get-CimInstance Win32_Processor).Name
powercfg /getactivescheme
rustc -V

Section 'demo started'
$log = Join-Path $out 'demo.log'
$proc = Start-Process -FilePath $demo -RedirectStandardOutput $log -NoNewWindow -PassThru
Start-Sleep -Seconds 2
Get-Content $log -TotalCount 4
$guid = (Select-String -Path $log -Pattern 'etw-guid=(\{[0-9a-fA-F-]+\})').Matches[0].Groups[1].Value

Section 'cargo anyprobe list'
& $cli list $demo

Section 'cargo anyprobe wprp'
$wprp = Join-Path $out 'demo.wprp'
& $cli wprp $demo | Set-Content -Encoding utf8 $wprp
Get-Content $wprp

Section 'logman, 3 s'
logman query providers $guid 2>&1 | Out-String
$etl = Join-Path $out 'demo-logman.etl'
logman create trace anyprobe-docs -p $guid 0xffffffffffffffff 0xff -o $etl -ets
Start-Sleep -Seconds 3
logman stop anyprobe-docs -ets
Get-ChildItem $out -Filter 'demo-logman*.etl' | Format-Table Name, Length | Out-String
$etlFile = Get-ChildItem $out -Filter 'demo-logman*.etl' | Select-Object -First 1
$xml = Join-Path $out 'demo-logman.xml'
tracerpt $etlFile.FullName -o $xml -of XML -y | Out-Null
Section 'decoded events (first 3 of each probe, as XML)'
$text = Get-Content $xml -Raw
foreach ($name in 'tick', 'checkout__entry', 'checkout__return') {
    Write-Host "--- $name"
    [regex]::Matches($text, "<Event [^>]*>(?:(?!</Event>).)*?$name(?:(?!</Event>).)*?</Event>",
        'Singleline') | Select-Object -First 3 | ForEach-Object { $_.Value }
}

Section 'wpr with the generated profile, 3 s'
$wprEtl = Join-Path $out 'demo-wpr.etl'
wpr -start $wprp -filemode
Start-Sleep -Seconds 3
wpr -stop $wprEtl
$wprXml = Join-Path $out 'demo-wpr.xml'
tracerpt $wprEtl -o $wprXml -of XML -y | Out-Null
$wprText = Get-Content $wprXml -Raw
foreach ($name in 'tick', 'checkout__entry', 'checkout__return') {
    "{0}: {1} events" -f $name, ([regex]::Matches($wprText, "Name=`"$name`"|>$name<")).Count
}

Stop-Process -Id $proc.Id
Section "demo's own output"
Get-Content $log -TotalCount 8

Section 'overhead, no tracer'
& $over 1000000

Section "overhead, wpr recording every event, $Calls calls"
$overWprp = Join-Path $out 'overhead.wprp'
& $cli wprp $over | Set-Content -Encoding utf8 $overWprp
$overEtl = Join-Path $out 'overhead.etl'
wpr -start $overWprp -filemode
& $over $Calls
wpr -stop $overEtl
Get-ChildItem $overEtl | Format-Table Name, Length | Out-String
Count-Events $overEtl

Section "overhead, logman recording every event, $Calls calls, -bs $BufferKB -nb $Buffers $Buffers"
# The provider registers on the first probe check, so the GUID comes from the
# profile; logman can enable it before the program starts.
$overGuid = '{' + (Select-String -Path $overWprp -Pattern 'Provider overhead: ([0-9a-fA-F-]{36})').Matches[0].Groups[1].Value + '}'
$overEtl2 = Join-Path $out 'overhead-logman.etl'
logman create trace anyprobe-overhead -p $overGuid 0xffffffffffffffff 0xff -o $overEtl2 -bs $BufferKB -nb $Buffers $Buffers -ets
& $over $Calls
logman query anyprobe-overhead -ets | Select-String -Pattern 'Buffer|Events Lost'
logman stop anyprobe-overhead -ets
$overFile = Get-ChildItem $out -Filter 'overhead-logman*.etl' | Select-Object -First 1
if ($null -eq $overFile) {
    Write-Host 'no trace file: the logman session did not start'
} else {
    $overFile | Format-Table Name, Length | Out-String
    Count-Events $overFile.FullName
}

Remove-Item -Recurse -Force $out
