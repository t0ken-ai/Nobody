# Exercise the actual NSIS payload in a disposable hosted runner: clean install,
# update-mode replacement, binary identity, then uninstall. Never run this test
# against a developer's installed Nobody or its HKCU uninstall registration.
$ErrorActionPreference = 'Stop'
if ($env:GITHUB_ACTIONS -ne 'true' -or $env:RUNNER_OS -ne 'Windows' -or !$env:RUNNER_TEMP) {
    throw 'This installer integration check is restricted to disposable Windows CI runners.'
}
$config = Get-Content 'src-tauri/tauri.conf.json' -Raw | ConvertFrom-Json
$installer = (Resolve-Path "artifacts/releases/Nobody-$($config.version)-Windows-x64-setup.exe").Path
$expected = (Get-FileHash 'src-tauri/target/release/nobody.exe' -Algorithm SHA256).Hash
$fixture = Join-Path $env:RUNNER_TEMP ("Nobody installer " + [guid]::NewGuid().ToString('N'))
$destination = Join-Path $fixture 'installed app'
$sentinel = Join-Path $fixture 'unrelated-data.txt'
New-Item -ItemType Directory -Path $fixture | Out-Null
Set-Content $sentinel 'preserve unrelated data'

# /D must be the final NSIS argument, unquoted even when the path has spaces.
# No /R is used: validation must not auto-launch the app or announce a LAN peer.
function Invoke-Installer([string]$arguments) {
    $process = Start-Process -FilePath $installer -ArgumentList $arguments -PassThru
    if (!$process.WaitForExit(180000)) { $process.Kill(); throw 'Installer timed out.' }
    if ($process.ExitCode -ne 0) { throw "Installer failed: $($process.ExitCode)" }
    if ((Get-FileHash (Join-Path $destination 'nobody.exe') -Algorithm SHA256).Hash -ne $expected) { throw 'Installed binary does not match the signed release build.' }
    if ((Get-Content $sentinel -Raw).Trim() -ne 'preserve unrelated data') { throw 'Unrelated data changed.' }
}
try {
    Invoke-Installer "/S /D=$destination"
    # Force an observable replacement; a successful no-op cannot satisfy the
    # update check. The test marker is never executed and stays in RUNNER_TEMP.
    [IO.File]::WriteAllText((Join-Path $destination 'nobody.exe'), 'old disposable binary')
    Invoke-Installer "/S /UPDATE /D=$destination"
    Write-Host 'PASS: NSIS clean install and update-mode replacement preserve unrelated data.'
} finally {
    $uninstaller = Join-Path $destination 'uninstall.exe'
    if (Test-Path $uninstaller) {
        $process = Start-Process -FilePath $uninstaller -ArgumentList '/S' -PassThru
        if (!$process.WaitForExit(60000)) { $process.Kill(); throw 'Uninstaller timed out.' }
    }
    # Leave cleanup to the ephemeral runner; an NSIS uninstaller may spawn a
    # child and finish after its launcher. Never delete outside this test tree.
}
