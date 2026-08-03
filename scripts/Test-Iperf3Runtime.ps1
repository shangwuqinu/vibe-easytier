[CmdletBinding()]
param(
    [Parameter(Mandatory)]
    [string]$RuntimeDirectory
)

Set-StrictMode -Version Latest
$ErrorActionPreference = 'Stop'
$iperf3Path = Join-Path ([System.IO.Path]::GetFullPath($RuntimeDirectory)) 'iperf3.exe'
if (-not (Test-Path -LiteralPath $iperf3Path -PathType Leaf)) {
    throw "Staged iperf3 executable was not found: $iperf3Path"
}

$listener = [System.Net.Sockets.TcpListener]::new([System.Net.IPAddress]::Loopback, 0)
$listener.Start()
$port = ([System.Net.IPEndPoint]$listener.LocalEndpoint).Port
$listener.Stop()

$server = $null
try {
    $server = Start-Process -FilePath $iperf3Path `
        -ArgumentList @('--server', '--bind', '127.0.0.1', '--port', [string]$port, '--server-max-duration', '20') `
        -WindowStyle Hidden -PassThru
    Start-Sleep -Milliseconds 400
    if ($server.HasExited) {
        throw "The staged iperf3 server exited before the loopback test. Exit code: $($server.ExitCode)"
    }

    foreach ($direction in @('upload', 'download')) {
        $arguments = @('--client', '127.0.0.1', '--bind', '127.0.0.1', '--port', [string]$port, '--time', '1', '--omit', '0', '--interval', '0', '--json', '--version4')
        if ($direction -eq 'download') {
            $arguments += '--reverse'
        }
        $output = (& $iperf3Path @arguments 2>&1) -join [Environment]::NewLine
        if ($LASTEXITCODE -ne 0) {
            throw "iperf3 $direction loopback test failed with exit code $LASTEXITCODE."
        }
        $result = $output | ConvertFrom-Json
        if ($result.PSObject.Properties.Name -contains 'error' -and $null -ne $result.error) {
            throw "iperf3 $direction loopback test returned an error."
        }
        if ([double]$result.end.sum_received.bits_per_second -le 0 -or [int64]$result.end.sum_received.bytes -le 0) {
            throw "iperf3 $direction loopback test did not transfer data."
        }
    }
}
finally {
    if ($null -ne $server -and -not $server.HasExited) {
        Stop-Process -Id $server.Id -Force -ErrorAction SilentlyContinue
        $server.WaitForExit()
    }
}

Write-Output "iperf3 loopback upload and download tests passed on TCP $port."
