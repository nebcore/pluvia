# Arma el paquete para repartir: compila en release y genera
# "Pluvia app/Pluvia-v<versión>.zip" con esta estructura:
#
#   Pluvia/
#     Pluvia.exe
#     LEEME.txt
#     assets/   (los sonidos de $bundledSounds, tomados de assets/ del proyecto)
#
# La versión sale de Cargo.toml (ver CHANGELOG.md). Uso, desde la raíz del proyecto:
#   powershell -ExecutionPolicy Bypass -File packaging\package.ps1

$ErrorActionPreference = 'Stop'

# Sonidos que vienen con el paquete (desde assets/ del proyecto). Es una lista fija a
# propósito: assets/ también tiene lo que se agrega para probar, y eso no se reparte.
$bundledSounds = @('lluvia_ruido.mp3')
$root = Split-Path $PSScriptRoot -Parent
Set-Location $root

$version = (Select-String -Path Cargo.toml -Pattern '^version\s*=\s*"(.+)"' | Select-Object -First 1).Matches[0].Groups[1].Value
Write-Host "Empaquetando Pluvia v$version"

cargo build --release
if ($LASTEXITCODE -ne 0) { throw "Falló la compilación" }

# Se arma en una carpeta temporal para no dejar restos si algo falla.
$staging = Join-Path ([IO.Path]::GetTempPath()) "pluvia-package-$version"
if (Test-Path $staging) { Remove-Item $staging -Recurse -Force }
$app = Join-Path $staging 'Pluvia'
New-Item -ItemType Directory -Force (Join-Path $app 'assets') | Out-Null

Copy-Item 'target\release\pluvia.exe' (Join-Path $app 'Pluvia.exe')
Copy-Item 'packaging\LEEME.txt' $app
foreach ($sound in $bundledSounds) {
    $source = Join-Path 'assets' $sound
    if (-not (Test-Path $source)) { throw "Falta el sonido incluido: $source" }
    Copy-Item $source (Join-Path $app 'assets')
}

$outDir = Join-Path $root 'Pluvia app'
New-Item -ItemType Directory -Force $outDir | Out-Null
$zip = Join-Path $outDir "Pluvia-v$version.zip"
if (Test-Path $zip) { Remove-Item $zip -Force }
# Entrada por entrada en vez de ZipFile.CreateFromDirectory: en Windows PowerShell ese
# guarda las rutas con "\", que otros descompresores no entienden como carpetas.
Add-Type -AssemblyName System.IO.Compression, System.IO.Compression.FileSystem
$archive = [IO.Compression.ZipFile]::Open($zip, 'Create')
try {
    Get-ChildItem $staging -Recurse -File | ForEach-Object {
        $entry = $_.FullName.Substring($staging.Length).TrimStart('\').Replace('\', '/')
        [IO.Compression.ZipFileExtensions]::CreateEntryFromFile($archive, $_.FullName, $entry, 'Optimal') | Out-Null
    }
} finally {
    $archive.Dispose()
}
Remove-Item $staging -Recurse -Force

$sizeMb = (Get-Item $zip).Length / 1MB
Write-Host ("Listo: {0} ({1:N1} MB)" -f $zip, $sizeMb)
