<#
.SYNOPSIS
Checks two runtime claims in docs/GAPS.md on Windows.

.DESCRIPTION
  - A probe in a `cdylib` loaded with LoadLibraryW can be traced ("Shared
    libraries"). A scratch DLL defines `plugonly:tick` and `shared:tick`;
    the executable that loads it defines `shared:tick` too. Each module has
    its own copy of anyprobe's provider table, so `shared` is registered
    twice in one process under one GUID. One logman session enables both
    GUIDs, and the trace must hold `plugonly` events from the DLL and
    `shared` events from both the executable and the DLL.
  - Large native strings ("Large native strings and byte slices"). A scratch
    program fires one event per size, from 1,000 to 70,000 bytes, each a
    native `&str` with its size as `id`. TraceLogging cuts a counted string
    at 65,535 bytes, and ETW drops an event over 64 KB including its headers.
    The check expects every value up to 60,000 bytes recorded whole, the
    70,000-byte event not recorded, and the program unaffected; it reports
    which sizes in between were recorded whole, cut, or not at all.

Prints ok/FAIL per check and exits non-zero if any check failed.

Run from an elevated (administrator) Windows PowerShell: logman needs it.

.EXAMPLE
powershell -ExecutionPolicy Bypass -File spike\scripts\check-gaps-windows.ps1
#>

$ErrorActionPreference = 'Stop'
Set-Location (Join-Path $PSScriptRoot '..\..')

$identity = [Security.Principal.WindowsIdentity]::GetCurrent()
$isAdmin = ([Security.Principal.WindowsPrincipal] $identity).IsInRole(
    [Security.Principal.WindowsBuiltInRole]::Administrator)
if (-not $isAdmin) {
    Write-Host 'Run from an elevated prompt: logman needs administrator rights.'
    exit 2
}

$work = Join-Path ([IO.Path]::GetTempPath()) "anyprobe-gaps-$PID"
New-Item -ItemType Directory -Force $work | Out-Null
$script:failed = $false
$procs = @()

function Expect([string] $What, [bool] $Ok) {
    if ($Ok) {
        Write-Host "  ok    $What"
    } else {
        Write-Host "  FAIL  $What"
        $script:failed = $true
    }
}

# Polls LOG until a line matches PATTERN; returns the match or $null.
function Wait-Line([string] $Log, [string] $Pattern, [int] $Seconds = 20) {
    for ($i = 0; $i -lt $Seconds * 10; $i++) {
        if (Test-Path $Log) {
            $m = Select-String -Path $Log -Pattern $Pattern
            if ($null -ne $m) { return $m }
        }
        Start-Sleep -Milliseconds 100
    }
    return $null
}

# Writes the piped text to PATH as UTF-8 without a BOM: Windows PowerShell's
# `Set-Content -Encoding utf8` adds one.
function Write-File([string] $Path) {
    [IO.File]::WriteAllText($Path, ($input | Out-String))
}

# Number of times PATTERN occurs in FILE.
function Count([string] $File, [string] $Pattern) {
    if (-not (Test-Path $File)) { return 0 }
    $n = (Select-String -Path $File -Pattern $Pattern -CaseSensitive -AllMatches |
        ForEach-Object { $_.Matches.Count } | Measure-Object -Sum).Sum
    if ($null -eq $n) { return 0 }
    return $n
}

# Starts an ETW session NAME on every GUID in GUIDS, writing to ETL.
function Start-Session([string] $Name, [string[]] $Guids, [string] $Etl) {
    $pf = Join-Path $work "$Name.providers"
    $Guids | ForEach-Object { "$_ 0xffffffffffffffff 0xff" } | Set-Content -Encoding ascii $pf
    logman create trace $Name -pf $pf -bs 1024 -nb 16 64 -o $Etl -ets | Out-Null
}

# Stops session NAME and decodes the trace it wrote to XML; returns the XML
# path, or $null when there is no trace.
function Stop-Session([string] $Name, [string] $Xml) {
    logman stop $Name -ets | Out-Null
    Start-Sleep -Seconds 1
    $etl = Get-ChildItem -Path $work -Filter "$Name*.etl" | Select-Object -First 1
    if ($null -eq $etl) { return $null }
    tracerpt $etl.FullName -o $Xml -of XML -y | Out-Null
    return $Xml
}

try {
    $anyprobe = (Join-Path $PWD 'crates\anyprobe') -replace '\\', '/'
    $crate = Join-Path $work 'dl'
    New-Item -ItemType Directory -Force "$crate\plug\src", "$crate\host\src\bin" | Out-Null
    @"
[package]
name = "plug"
version = "0.0.0"
edition = "2024"
[lib]
crate-type = ["cdylib"]
[dependencies]
anyprobe = { path = "$anyprobe", default-features = false }
"@ | Write-File "$crate\plug\Cargo.toml"
    @'
mod shared {
    anyprobe::probes! {
        provider = "shared";
        pub fn tick(from: &str, n: u64);
    }
}
mod plugonly {
    anyprobe::probes! {
        provider = "plugonly";
        pub fn tick(from: &str, n: u64);
    }
}
#[unsafe(no_mangle)]
pub extern "C" fn plug_tick(n: u64) {
    if shared::tick::enabled() {
        shared::tick::fire("plug", n);
    }
    if plugonly::tick::enabled() {
        plugonly::tick::fire("plugonly", n);
    }
}
'@ | Write-File "$crate\plug\src\lib.rs"
    @"
[package]
name = "host"
version = "0.0.0"
edition = "2024"
[dependencies]
anyprobe = { path = "$anyprobe", default-features = false }
"@ | Write-File "$crate\host\Cargo.toml"
    @'
use std::ffi::c_void;
use std::io::Write;
use std::time::Duration;
#[link(name = "kernel32")]
unsafe extern "system" {
    fn LoadLibraryW(name: *const u16) -> *mut c_void;
    fn GetProcAddress(module: *mut c_void, name: *const u8) -> *mut c_void;
}
anyprobe::probes! {
    provider = "shared";
    fn tick(from: &str, n: u64);
}
fn main() {
    let path: Vec<u16> = std::env::args().nth(1).unwrap().encode_utf16().chain([0]).collect();
    // SAFETY: a test program; the library and symbol exist.
    let f = unsafe {
        let h = LoadLibraryW(path.as_ptr());
        assert!(!h.is_null(), "LoadLibraryW failed");
        let p = GetProcAddress(h, c"plug_tick".as_ptr().cast());
        assert!(!p.is_null(), "GetProcAddress failed");
        std::mem::transmute::<*mut c_void, extern "C" fn(u64)>(p)
    };
    let guid = anyprobe::__private::etw::guid_string;
    println!(
        "loaded pid={} shared-guid={} plugonly-guid={}",
        std::process::id(),
        guid("shared"),
        guid("plugonly")
    );
    let _ = std::io::stdout().flush();
    let mut enabled = false;
    for n in 0..4000 {
        if tick::enabled() {
            tick::fire("host", n);
        }
        f(n);
        if tick::enabled() != enabled {
            enabled = !enabled;
            println!("host-enabled={enabled}");
            let _ = std::io::stdout().flush();
        }
        std::thread::sleep(Duration::from_millis(25));
    }
}
'@ | Write-File "$crate\host\src\main.rs"
    @'
use std::io::Write;
use std::time::{Duration, Instant};
anyprobe::probes! {
    provider = "bigstr";
    fn big(id: u64, s: &str);
}
const SIZES: [usize; 16] = [
    1000, 32768, 60000, 65000, 65200, 65300, 65350, 65400, 65420, 65440, 65460, 65480, 65500,
    65535, 65600, 70000,
];
fn main() {
    println!("guid={}", anyprobe::__private::etw::guid_string(big::PROVIDER));
    let _ = std::io::stdout().flush();
    let start = Instant::now();
    while !big::enabled() {
        if start.elapsed() > Duration::from_secs(30) {
            println!("never-enabled");
            return;
        }
        std::thread::sleep(Duration::from_millis(50));
    }
    // Give the session time to enable every keyword before firing.
    std::thread::sleep(Duration::from_millis(500));
    let s = "x".repeat(70000);
    for n in SIZES {
        big::fire(n as u64, &s[..n]);
    }
    println!("fired {}", SIZES.len());
    let _ = std::io::stdout().flush();
    std::thread::sleep(Duration::from_secs(1));
    println!("done");
}
'@ | Write-File "$crate\host\src\bin\big.rs"

    $env:CARGO_TARGET_DIR = Join-Path $PWD 'target\check-gaps'
    Push-Location "$crate\plug"; cargo build -q --release; $plugOk = $LASTEXITCODE -eq 0; Pop-Location
    Push-Location "$crate\host"; cargo build -q --release; $hostOk = $LASTEXITCODE -eq 0; Pop-Location
    if (-not ($plugOk -and $hostOk)) {
        Write-Host '  FAIL  build the scratch crates'
        exit 1
    }
    $dll = Join-Path $env:CARGO_TARGET_DIR 'release\plug.dll'
    $hostExe = Join-Path $env:CARGO_TARGET_DIR 'release\host.exe'
    $bigExe = Join-Path $env:CARGO_TARGET_DIR 'release\big.exe'

    Write-Host '== cdylib loaded with LoadLibraryW'
    $hlog = Join-Path $work 'host.log'
    $hproc = Start-Process -FilePath $hostExe -ArgumentList "`"$dll`"" `
        -RedirectStandardOutput $hlog -RedirectStandardError (Join-Path $work 'host.err') `
        -NoNewWindow -PassThru
    $procs += $hproc
    $m = Wait-Line $hlog 'shared-guid=(\{[0-9a-fA-F-]+\}) plugonly-guid=(\{[0-9a-fA-F-]+\})'
    Expect 'host loaded the DLL and printed both GUIDs' ($null -ne $m)
    if ($null -ne $m) {
        $shared = $m.Matches[0].Groups[1].Value
        $plugonly = $m.Matches[0].Groups[2].Value
        Write-Host "  info  shared $shared, plugonly $plugonly"
        Start-Session 'anyprobe-gaps-dl' @($shared, $plugonly) (Join-Path $work 'anyprobe-gaps-dl.etl')
        $on = Wait-Line $hlog 'host-enabled=true'
        Expect 'session start turned the host''s shared:tick on' ($null -ne $on)
        Start-Sleep -Seconds 3
        $xml = Stop-Session 'anyprobe-gaps-dl' (Join-Path $work 'dl.xml')
        $off = Wait-Line $hlog 'host-enabled=false' 5
        Expect 'session stop turned it off again' ($null -ne $off)
        Expect 'session wrote a trace' ($null -ne $xml)
        $n = Count $xml '>plugonly<'
        Expect "plugonly:tick from the DLL recorded ($n events)" ($n -gt 20)
        $n = Count $xml '>host<'
        Expect "shared:tick from the executable recorded ($n events)" ($n -gt 20)
        $n = Count $xml '>plug<'
        Expect "shared:tick from the DLL recorded ($n events)" ($n -gt 20)
    }
    Stop-Process -Id $hproc.Id -ErrorAction SilentlyContinue

    Write-Host '== large native strings'
    $blog = Join-Path $work 'big.log'
    $bproc = Start-Process -FilePath $bigExe -RedirectStandardOutput $blog `
        -RedirectStandardError (Join-Path $work 'big.err') -NoNewWindow -PassThru
    $procs += $bproc
    $m = Wait-Line $blog 'guid=(\{[0-9a-fA-F-]+\})'
    Expect 'big printed its provider GUID' ($null -ne $m)
    if ($null -ne $m) {
        $guid = $m.Matches[0].Groups[1].Value
        Start-Session 'anyprobe-gaps-big' @($guid) (Join-Path $work 'anyprobe-gaps-big.etl')
        $fired = Wait-Line $blog '^fired'
        Expect 'big fired every size' ($null -ne $fired)
        $done = Wait-Line $blog '^done' 10
        Expect 'big ran to the end after firing' ($null -ne $done)
        $xml = Stop-Session 'anyprobe-gaps-big' (Join-Path $work 'big.xml')
        Expect 'session wrote a trace' ($null -ne $xml)
        $recorded = @{}
        if ($null -ne $xml) {
            $text = [IO.File]::ReadAllText($xml)
            $re = '<Data Name="id">(\d+)</Data>\s*<Data Name="s">(x*)</Data>'
            foreach ($r in [regex]::Matches($text, $re)) {
                $recorded[[int] $r.Groups[1].Value] = $r.Groups[2].Value.Length
            }
        }
        $sizes = 1000, 32768, 60000, 65000, 65200, 65300, 65350, 65400, 65420, 65440, 65460,
            65480, 65500, 65535, 65600, 70000
        foreach ($s in $sizes) {
            if ($recorded.ContainsKey($s)) {
                $len = $recorded[$s]
                $how = if ($len -eq $s) { 'whole' } else { "cut to $len bytes" }
                Write-Host "  info  $s bytes: recorded, $how"
            } else {
                Write-Host "  info  $s bytes: not recorded"
            }
        }
        $whole = $sizes | Where-Object { $_ -le 60000 } |
            Where-Object { $recorded[$_] -eq $_ }
        Expect 'every value up to 60,000 bytes recorded whole' (@($whole).Count -eq 3)
        Expect 'the 70,000-byte event not recorded' (-not $recorded.ContainsKey(70000))
    }
    Stop-Process -Id $bproc.Id -ErrorAction SilentlyContinue

    if ($script:failed) {
        foreach ($f in 'host.log', 'host.err', 'big.log', 'big.err') {
            $p = Join-Path $work $f
            if (Test-Path $p) {
                Write-Host "  --- $f (last 20 lines)"
                Get-Content $p -Tail 20
            }
        }
    }
} finally {
    foreach ($p in $procs) { Stop-Process -Id $p.Id -ErrorAction SilentlyContinue }
    foreach ($name in 'anyprobe-gaps-dl', 'anyprobe-gaps-big') {
        try { logman stop $name -ets | Out-Null } catch { }
    }
    Remove-Item Env:CARGO_TARGET_DIR -ErrorAction SilentlyContinue
    Remove-Item -Recurse -Force $work -ErrorAction SilentlyContinue
}

if ($script:failed) {
    Write-Host 'FAIL'
    exit 1
}
Write-Host 'PASS'
exit 0
