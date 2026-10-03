<#
Starts an ETW session on the anyprobe `attr` and `attr_async` examples
(`#[probe]`) and checks what the decoded trace holds.

Usage (elevated Windows PowerShell; `logman` needs administrator rights):
  powershell -ExecutionPolicy Bypass -File spike\scripts\attach-windows-attr.ps1 [-Profiles release,release-lto] [-KeepTraces DIR]

For each profile and example it builds the example, starts it, records its
provider for three seconds with `logman`, and decodes the trace with
`tracerpt`. For `attr` it checks that:
  - native arguments and a native return value are recorded;
  - a `serde` argument is recorded as its JSON;
  - a `debug` return value is recorded as its `{:?}` output;
  - arguments collapsed into one object are recorded as that JSON object;
  - `debug(self)` on a method is recorded as the receiver's `{:?}` output.
Encoding runs only after the enabled check, so any encoded value in the trace
shows that the session turned the check on.
For `attr_async` it checks that:
  - `fetch` returns pair with their entries by invocation id, each carrying
    the value its entry's arguments give;
  - `fetch` calls overlap: some return arrives after another call's entry;
  - the cancelled `slow` fires its unwind probe, not panicking, with its
    entry's invocation id, and never its return probe;
  - `may_panic` fires its return probe for even ids and its unwind probe
    for odd ids;
  - `exported` (`symbol`) fires its probes.
A session starts after the example does, so the checks do not count events;
they check every event the session did record.
-KeepTraces copies each decoded trace (XML) into DIR.
Prints ok/FAIL per check and exits non-zero if any check failed.
#>
param(
    [string[]] $Profiles = @('release', 'release-lto'),
    [string] $KeepTraces = ''
)

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

# Builds `$Example`, records its provider while it runs and returns the decoded
# trace as text, or $null after printing why there is none. The example's output
# is left in $script:exampleLog.
function Capture([string] $Example, [string] $Profile, [string[]] $ExampleArgs) {
    cargo build -q -p anyprobe --example $Example --profile $Profile
    if ($LASTEXITCODE -ne 0) {
        Write-Host '  FAIL  build'
        $script:profileFailed = $true
        return $null
    }
    $bin = "target\$Profile\examples\$Example.exe"
    $tag = "$Example-$Profile"
    $log = Join-Path $work "$tag.log"
    $etl = Join-Path $work "$tag.etl"
    $xml = Join-Path $work "$tag.xml"
    $session = "anyprobe-$tag"
    $script:exampleLog = $log

    $proc = Start-Process -FilePath $bin -ArgumentList $ExampleArgs `
        -RedirectStandardOutput $log -NoNewWindow -PassThru
    Start-Sleep -Seconds 2
    $match = Select-String -Path $log -Pattern 'etw-guid=(\{[0-9a-fA-F-]+\})'
    Expect "$Example printed its provider GUID" ($null -ne $match)
    if ($null -eq $match) {
        Stop-Process -Id $proc.Id
        return $null
    }
    $guid = $match.Matches[0].Groups[1].Value

    logman create trace $session -p $guid 0xffffffffffffffff 0xff -o $etl -ets | Out-Null
    Start-Sleep -Seconds 3
    logman stop $session -ets | Out-Null
    Start-Sleep -Seconds 1
    Stop-Process -Id $proc.Id

    $etlFile = Get-ChildItem -Path $work -Filter "$tag*.etl" | Select-Object -First 1
    Expect 'session wrote a trace file' ($null -ne $etlFile)
    if ($null -ne $etlFile) {
        tracerpt $etlFile.FullName -o $xml -of XML -y | Out-Null
    }
    if (-not (Test-Path $xml)) { return $null }
    if ($KeepTraces -ne '') {
        New-Item -ItemType Directory -Force $KeepTraces | Out-Null
        Copy-Item $xml (Join-Path $KeepTraces "$tag.xml") -Force
    }
    $script:exampleXml = $xml
    return Get-Content $xml -Raw
}

# Every event of the decoded trace as @{ Name; Data = @{ field = value } }.
# The event name is the first `probe__kind` token in the event's XML.
function Get-Events([string] $Trace) {
    $events = @()
    foreach ($m in [regex]::Matches($Trace, '(?s)<Event\b.*?</Event>')) {
        $block = $m.Value
        $name = [regex]::Match($block, '\b(\w+?__(?:entry|return|unwind))\b')
        if (-not $name.Success) { continue }
        $data = @{}
        foreach ($d in [regex]::Matches($block, '<Data Name="([^"]+)">([^<]*)</Data>')) {
            $data[$d.Groups[1].Value] = $d.Groups[2].Value
        }
        $events += , @{ Name = $name.Groups[1].Value; Data = $data }
    }
    return , $events
}

function Show-Failure([string] $Xml) {
    $script:failed = $true
    Write-Host '  --- example output'
    if ($script:exampleLog -and (Test-Path $script:exampleLog)) { Get-Content $script:exampleLog }
    if ($Xml -and (Test-Path $Xml)) {
        Write-Host '  --- decoded trace (first 80 lines)'
        Get-Content $Xml -TotalCount 80
    }
}

# A double quote in element text may be written as itself or as &quot;.
$q = '(?:"|&quot;)'

try {
    foreach ($p in $Profiles) {
        Write-Host "== $p (anyprobe example attr)"
        $script:profileFailed = $false
        $script:exampleXml = $null
        $trace = Capture 'attr' $p @('0', '20')
        if ($null -ne $trace) {
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
        }
        if ($script:profileFailed) { Show-Failure $script:exampleXml }

        Write-Host "== $p (anyprobe example attr_async)"
        $script:profileFailed = $false
        $script:exampleXml = $null
        # 1000 iterations of 20 ms outlast the three-second session.
        $trace = Capture 'attr_async' $p @('1000', '20')
        if ($null -ne $trace) {
            $ev = Get-Events $trace
            $by = @{}
            foreach ($e in $ev) {
                if (-not $by.ContainsKey($e.Name)) { $by[$e.Name] = @() }
                $by[$e.Name] += , $e
            }
            function Of([string] $Name) { if ($by.ContainsKey($Name)) { return $by[$Name] } else { return @() } }

            $entries = @{}
            foreach ($e in Of 'fetch__entry') { $entries[$e.Data['invocation']] = $e }
            $returns = Of 'fetch__return'
            Expect 'fetch entry and return events recorded' `
                ($entries.Count -gt 0 -and $returns.Count -gt 0)

            # A return with invocation 0 started before the session did.
            $paired = 0
            $badPair = 0
            foreach ($r in $returns) {
                $inv = $r.Data['invocation']
                if ($inv -eq '0') { continue }
                if (-not $entries.ContainsKey($inv)) { $badPair++; continue }
                $en = $entries[$inv]
                $want = [uint64] $en.Data['id'] * 10 + $en.Data['path'].Length
                if ([uint64] $r.Data['ret'] -eq $want) { $paired++ } else { $badPair++ }
            }
            Expect "fetch returns pair with their entries ($paired paired, $badPair not)" `
                ($paired -ge 5 -and $badPair -eq 0)

            # Within one iteration the second call returns before the first, so
            # some entry must sit between another call's entry and its return.
            $order = @()
            foreach ($e in $ev) {
                if ($e.Name -eq 'fetch__entry') { $order += "E$($e.Data['invocation'])" }
                if ($e.Name -eq 'fetch__return') { $order += "R$($e.Data['invocation'])" }
            }
            $overlap = 0
            for ($i = 0; $i -lt $order.Count; $i++) {
                if ($order[$i] -notlike 'E*') { continue }
                $next = $order[$i + 1]
                if ($next -like 'E*') { $overlap++ }
            }
            Expect "fetch calls overlap ($overlap entries followed by another entry)" ($overlap -ge 5)

            $slowEntries = @{}
            foreach ($e in Of 'slow__entry') { $slowEntries[$e.Data['invocation']] = $e }
            $unwinds = Of 'slow__unwind'
            $badUnwind = 0
            foreach ($u in $unwinds) {
                if ($u.Data['invocation'] -ne '0' -and
                    (-not $slowEntries.ContainsKey($u.Data['invocation']) -or $u.Data['panicking'] -ne 'false')) {
                    $badUnwind++
                }
            }
            Expect "cancelled slow unwinds, not panicking, with its invocation id ($($unwinds.Count) events)" `
                ($unwinds.Count -ge 5 -and $badUnwind -eq 0)
            Expect 'cancelled slow never returns' ((Of 'slow__return').Count -eq 0)

            $evenIds = @(); $oddIds = @()
            foreach ($e in Of 'may_panic__entry') {
                if ([uint64] $e.Data['id'] % 2 -eq 0) { $evenIds += $e } else { $oddIds += $e }
            }
            Expect 'may_panic entry for even and odd ids' ($evenIds.Count -ge 5 -and $oddIds.Count -ge 5)
            $mpReturns = (Of 'may_panic__return').Count
            $mpUnwinds = (Of 'may_panic__unwind').Count
            # One return per even entry and one unwind per odd entry, give or
            # take the single call the session could have cut off at either end.
            Expect "may_panic returns for even ids ($mpReturns returns, $($evenIds.Count) even entries)" `
                ($mpReturns -ge 5 -and [Math]::Abs($mpReturns - $evenIds.Count) -le 1)
            Expect "may_panic unwinds for odd ids ($mpUnwinds unwinds, $($oddIds.Count) odd entries)" `
                ($mpUnwinds -ge 5 -and [Math]::Abs($mpUnwinds - $oddIds.Count) -le 1)

            Expect 'exported (symbol) fires entry and return' `
                ((Of 'exported__entry').Count -ge 5 -and (Of 'exported__return').Count -ge 5)
        }
        if ($script:profileFailed) { Show-Failure $script:exampleXml }
        if ($script:profileFailed) { $script:failed = $true }
    }
} finally {
    Remove-Item -Recurse -Force $work -ErrorAction SilentlyContinue
}

if ($script:failed) { Write-Host 'FAIL'; exit 1 }
Write-Host 'PASS'
