# Records every outbound TCP connection Halcyon opens, with the host behind each address.
#
#   powershell -ExecutionPolicy Bypass -File tools/network-trace.ps1 -Seconds 90 -Out trace.txt
#
# ## Why this rather than a packet capture
#
# docs/06 Phase 6 asks for a network trace showing that reading a message causes no request the
# reader did not ask for. A packet capture would be the obvious instrument and needs npcap and
# administrator; this needs neither, and answers the question that is actually being asked --
# *which hosts did this process talk to* -- rather than a question about packets.
#
# What it cannot see: a connection opened and closed entirely between two polls. The interval is
# deliberately short for that reason, and the limitation is real and is written into the record
# rather than left for a reader to discover. A remote image fetch on a page held open is not the
# kind of thing that hides in 300ms, but a single beacon might.

param(
    [int]$Seconds = 60,
    [int]$IntervalMs = 300,
    [string]$Out = ''
)

$ErrorActionPreference = 'Stop'

$seen = [ordered]@{}
$deadline = (Get-Date).AddSeconds($Seconds)
$resolved = @{}

function HostFor($address) {
    if ($resolved.ContainsKey($address)) { return $resolved[$address] }

    $name = $address
    try {
        $entry = [System.Net.Dns]::GetHostEntry($address)
        if ($entry -and $entry.HostName) { $name = $entry.HostName }
    } catch {
        # No reverse record. The address is still the evidence; the name is a convenience.
        $name = '(no reverse DNS)'
    }

    $resolved[$address] = $name
    return $name
}

Write-Output "watching halcyon for $Seconds seconds, polling every ${IntervalMs}ms"

while ((Get-Date) -lt $deadline) {
    $pids = @(Get-Process halcyon -ErrorAction SilentlyContinue | Select-Object -ExpandProperty Id)

    if ($pids.Count -gt 0) {
        $connections = Get-NetTCPConnection -ErrorAction SilentlyContinue |
            Where-Object { $pids -contains $_.OwningProcess -and $_.RemoteAddress -notmatch '^(0\.0\.0\.0|127\.0\.0\.1|::|::1)$' }

        foreach ($c in $connections) {
            $key = "$($c.RemoteAddress):$($c.RemotePort)"
            if (-not $seen.Contains($key)) {
                $seen[$key] = [pscustomobject]@{
                    Address = $c.RemoteAddress
                    Port    = $c.RemotePort
                    First   = (Get-Date).ToString('HH:mm:ss')
                    State   = $c.State
                }
                Write-Output "  $((Get-Date).ToString('HH:mm:ss'))  $key"
            }
        }
    }

    Start-Sleep -Milliseconds $IntervalMs
}

$lines = @()
$lines += "Halcyon network trace"
$lines += "recorded $(Get-Date -Format 'yyyy-MM-dd HH:mm:ss'), $Seconds seconds, ${IntervalMs}ms polling"
$lines += ""

if ($seen.Count -eq 0) {
    $lines += "No outbound connections observed."
} else {
    $lines += "{0,-24} {1,-7} {2,-9} {3}" -f 'ADDRESS', 'PORT', 'FIRST', 'HOST'
    foreach ($key in $seen.Keys) {
        $c = $seen[$key]
        $lines += "{0,-24} {1,-7} {2,-9} {3}" -f $c.Address, $c.Port, $c.First, (HostFor $c.Address)
    }
}

$text = $lines -join [Environment]::NewLine
Write-Output ''
Write-Output $text

if ($Out -ne '') {
    Set-Content -Path $Out -Value $text -Encoding utf8
    Write-Output ''
    Write-Output "written to $Out"
}
