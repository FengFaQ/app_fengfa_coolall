$ErrorActionPreference = 'Stop'

function Send-Cdp {
    param(
        [System.Net.WebSockets.ClientWebSocket]$Ws,
        [int]$Id,
        [string]$Method,
        [hashtable]$Params = @{}
    )
    $msg = @{ id = $Id; method = $Method; params = $Params } | ConvertTo-Json -Compress -Depth 10
    $bytes = [Text.Encoding]::UTF8.GetBytes($msg)
    $seg = [ArraySegment[byte]]::new($bytes)
    $Ws.SendAsync($seg, [System.Net.WebSockets.WebSocketMessageType]::Text, $true, [Threading.CancellationToken]::None).Wait()

    $buf = New-Object byte[] 2097152
    for ($i = 0; $i -lt 200; $i++) {
        $sb = [Text.StringBuilder]::new()
        do {
            $r = $Ws.ReceiveAsync([ArraySegment[byte]]::new($buf), [Threading.CancellationToken]::None).Result
            $null = $sb.Append([Text.Encoding]::UTF8.GetString($buf, 0, $r.Count))
        } while (-not $r.EndOfMessage)

        $obj = $sb.ToString() | ConvertFrom-Json
        if ($obj.id -eq $Id) { return $obj }
    }
    return $null
}

$targets = Invoke-RestMethod 'http://127.0.0.1:9222/json' -TimeoutSec 10
$page = $targets | Where-Object { $_.type -eq 'page' } | Select-Object -First 1
if (-not $page) { throw 'no page target' }
Write-Host "Target URL: $($page.url)"

$ws = [System.Net.WebSockets.ClientWebSocket]::new()
$ws.ConnectAsync([Uri]$page.webSocketDebuggerUrl, [Threading.CancellationToken]::None).Wait()
Write-Host "WebSocket connected"

try {
    $null = Send-Cdp -Ws $ws -Id 1 -Method 'Network.enable'
    Write-Host "Network.enable OK"

    $res = Send-Cdp -Ws $ws -Id 2 -Method 'Network.getAllCookies'
    $cookies = $res.result.cookies
    Write-Host "Total cookies: $($cookies.Count)"

    $coolapk = @($cookies | Where-Object { $_.domain -like '*coolapk*' })
    Write-Host ""
    Write-Host "=== coolapk cookies: $($coolapk.Count) ==="
    foreach ($c in $coolapk) {
        $v = $c.value
        if ($v.Length -gt 50) { $v = $v.Substring(0, 50) + '...' }
        Write-Host ("  {0,-18} = {1}" -f $c.name, $v)
    }

    $out = Join-Path $env:TEMP 'coolapk-cookies.json'
    if ($coolapk.Count -gt 0) {
        $coolapk | ConvertTo-Json -Depth 5 | Set-Content $out -Encoding UTF8
        Write-Host "Saved to $out"
    } else {
        Write-Host "NO coolapk cookies found"
    }

    $expr = 'document.title + " | htmlLen=" + document.documentElement.outerHTML.length + " | ck=" + document.cookie.substring(0,150)'
    $res2 = Send-Cdp -Ws $ws -Id 3 -Method 'Runtime.evaluate' -Params @{ expression = $expr; returnByValue = $true }
    Write-Host ""
    Write-Host "=== page state ==="
    Write-Host ("  " + $res2.result.result.value)
}
finally {
    try { $ws.CloseAsync([System.Net.WebSockets.WebSocketCloseStatus]::NormalClosure, 'done', [Threading.CancellationToken]::None).Wait() } catch {}
    $ws.Dispose()
}
