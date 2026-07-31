# Start meerdere instanties naast elkaar op deze PC, elk met een eigen datamap en poort.
# Voor snel handmatig testen zonder tweede machine.
#
#   .\scripts\run-peers.ps1              # 2 instanties
#   .\scripts\run-peers.ps1 -Count 3     # 3 instanties, volledige mesh
#   .\scripts\run-peers.ps1 -Stop        # alles afsluiten
#   .\scripts\run-peers.ps1 -Logs        # per instantie een venster met de meterregels
#
# De datamappen komen in .\.localpeers\ en worden bij elke start opnieuw opgebouwd,
# zodat je nooit met een halve staat van een vorige run zit.

param(
    [int]$Count = 2,
    [switch]$Stop,
    [switch]$Release,
    [switch]$Logs
)

$ErrorActionPreference = "Stop"
$root = Split-Path -Parent $PSScriptRoot
$peerRoot = Join-Path $root ".localpeers"
$basePort = 41650

if ($Stop) {
    $procs = Get-Process fitcom -ErrorAction SilentlyContinue
    if ($null -eq $procs) { Write-Host "Er draait niets."; exit 0 }
    $procs | Stop-Process -Force
    Write-Host "$($procs.Count) instantie(s) gestopt."
    exit 0
}

# Eén venster per instantie met alleen de meterregels van deler en kijker. Los aan te
# roepen terwijl er al instanties draaien; dit start of stopt niets.
function Start-LogVensters {
    $dirs = Get-ChildItem $peerRoot -Directory -ErrorAction SilentlyContinue
    if (-not $dirs) { Write-Host "Geen .localpeers-mappen; start eerst instanties."; return }
    foreach ($d in $dirs) {
        $logDir = Join-Path $d.FullName "logs"
        $cmd = "`$f = Get-ChildItem '$logDir\*.log' | Sort-Object LastWriteTime | Select-Object -Last 1;" +
               "Write-Host '$($d.Name)' -ForegroundColor Cyan;" +
               "Get-Content `$f.FullName -Wait -Tail 0 | Select-String 'deler|kijker'"
        Start-Process powershell -ArgumentList "-NoExit", "-Command", $cmd | Out-Null
    }
    Write-Host "$($dirs.Count) logvenster(s) geopend."
}

if ($Logs -and -not $Stop) {
    # Instanties schrijven hun eerste regels pas na een paar honderd milliseconden.
    if (-not (Test-Path $peerRoot)) { throw "Nog niets gestart: draai eerst zonder -Logs." }
    Start-LogVensters
    exit 0
}

if ($Count -lt 2) { throw "Met minder dan 2 instanties valt er niets te testen." }

# Bestaande instanties eerst weg: anders klapt het binden van de poort.
Get-Process fitcom -ErrorAction SilentlyContinue | Stop-Process -Force
Start-Sleep -Milliseconds 500

$profile = if ($Release) { "release" } else { "debug" }
$buildArgs = @("build", "-p", "fitcom")
if ($Release) { $buildArgs += "--release" }

Write-Host "Bouwen ($profile)..."
& cargo @buildArgs
if ($LASTEXITCODE -ne 0) { throw "build mislukt" }

$exe = Join-Path $root "target\$profile\fitcom.exe"
if (-not (Test-Path $exe)) { throw "exe niet gevonden op $exe" }

if (Test-Path $peerRoot) { Remove-Item -Recurse -Force $peerRoot }
New-Item -ItemType Directory -Force $peerRoot | Out-Null

for ($i = 0; $i -lt $Count; $i++) {
    $name = "peer$($i + 1)"
    $dir = Join-Path $peerRoot $name
    New-Item -ItemType Directory -Force $dir | Out-Null

    # Elke instantie krijgt alle anderen als peer: volledige mesh, geen host.
    $lines = @(
        "display_name = `"$name`""
        "control_port = $($basePort + $i * 2)"
        "media_port   = $($basePort + $i * 2 + 1)"
    )
    for ($j = 0; $j -lt $Count; $j++) {
        if ($j -eq $i) { continue }
        $lines += ""
        $lines += "[[peers]]"
        $lines += "address      = `"127.0.0.1`""
        $lines += "label        = `"peer$($j + 1)`""
        $lines += "control_port = $($basePort + $j * 2)"
    }
    $lines -join "`n" | Out-File -FilePath (Join-Path $dir "config.toml") -Encoding utf8

    Start-Process -FilePath $exe -ArgumentList @("--data-dir", $dir) | Out-Null
    Write-Host "$name gestart op poort $($basePort + $i * 2)  ($dir)"
}

Write-Host ""
Write-Host "Meters: .\scripts\run-peers.ps1 -Logs   (venster per instantie)"
Write-Host "Ruw:    Get-Content $peerRoot\peer1\logs\*.log -Wait"
Write-Host "Stop:   .\scripts\run-peers.ps1 -Stop"
