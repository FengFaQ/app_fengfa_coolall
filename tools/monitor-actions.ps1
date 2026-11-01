[CmdletBinding()]
param(
    [string]$Repository = 'FengFaQ/app_fengfa_coolall',
    [ValidateRange(60, 86400)]
    [int]$IntervalSeconds = 300,
    [switch]$Once
)

$ErrorActionPreference = 'Stop'
$projectRoot = Split-Path -Parent $PSScriptRoot
$logDirectory = Join-Path $projectRoot '_run'
$logPath = Join-Path $logDirectory 'actions-monitor.log'
$statePath = Join-Path $logDirectory 'actions-monitor-state.json'

# gh is authenticated separately; this script never reads or writes credentials.
$userPath = [Environment]::GetEnvironmentVariable('Path', 'User')
$machinePath = [Environment]::GetEnvironmentVariable('Path', 'Machine')
$env:Path = "$userPath;$machinePath;$env:Path"
$gh = Get-Command gh -ErrorAction SilentlyContinue
if (-not $gh) {
    $ghLink = Join-Path $env:LOCALAPPDATA 'Microsoft\WinGet\Links\gh.exe'
    if (Test-Path -LiteralPath $ghLink) { $gh = Get-Item -LiteralPath $ghLink }
}
if (-not $gh) { throw 'GitHub CLI (gh) not found; authenticate it separately before monitoring.' }
$env:GH_PAGER = 'cat'
New-Item -ItemType Directory -Force -Path $logDirectory | Out-Null

function Write-MonitorLog([string]$Message) {
    $line = '{0} {1}' -f (Get-Date -Format 'yyyy-MM-ddTHH:mm:ssK'), $Message
    Add-Content -LiteralPath $logPath -Value $line -Encoding utf8
}

function Get-MonitorState {
    if (-not (Test-Path -LiteralPath $statePath)) { return @{} }
    try {
        $state = @{}
        $saved = Get-Content -LiteralPath $statePath -Raw | ConvertFrom-Json
        foreach ($property in $saved.PSObject.Properties) {
            $state[$property.Name] = $property.Value
        }
        return $state
    } catch {
        Write-MonitorLog "ERROR state read: $($_.Exception.Message)"
        return @{}
    }
}

function Save-MonitorState($State) {
    $State | ConvertTo-Json -Depth 4 | Set-Content -LiteralPath $statePath -Encoding utf8
}

function Write-NewArtifacts([string]$RunId, [System.Collections.Hashtable]$State) {
    $key = "artifacts:$RunId"
    if ($State.ContainsKey($key)) { return }
    $response = & $gh.Source api "repos/$Repository/actions/runs/$RunId/artifacts" | ConvertFrom-Json
    $artifacts = @($response.artifacts | ForEach-Object {
        '{0}:{1}bytes:expired={2}' -f $_.name, $_.size_in_bytes, $_.expired
    })
    # Do not permanently cache an empty list: artifacts can appear shortly after completion.
    if ($artifacts.Count -gt 0) {
        $State[$key] = $artifacts
        Write-MonitorLog "ARTIFACT run=$RunId $($artifacts -join ';')"
    }
}

function Invoke-MonitorScan {
    param([System.Collections.Hashtable]$State)
    $response = & $gh.Source api "repos/$Repository/actions/runs?branch=main&per_page=10" | ConvertFrom-Json
    $latest = @($response.workflow_runs | Select-Object -First 1)
    foreach ($requiredId in @(41264317282, 41263517368)) {
        if ($latest.id -notcontains $requiredId) {
            try {
                $requiredRun = & $gh.Source api "repos/$Repository/actions/runs/$requiredId" | ConvertFrom-Json
                $latest += $requiredRun
            } catch {
                Write-MonitorLog "ERROR run=$requiredId $($_.Exception.Message)"
            }
        }
    }
    foreach ($run in $latest) {
        $key = [string]$run.id
        $value = 'run={0}|sha={1}|status={2}|conclusion={3}|url={4}' -f $run.id, $run.head_sha, $run.status, $run.conclusion, $run.html_url
        if (-not $State.ContainsKey($key) -or $State[$key] -ne $value) {
            Write-MonitorLog "RUN $value"
            $State[$key] = $value
            Save-MonitorState $State
        }
        if ($run.status -eq 'completed' -and $run.conclusion -eq 'success') {
            try {
                Write-NewArtifacts -RunId $key -State $State
                Save-MonitorState $State
            } catch {
                Write-MonitorLog "ERROR artifacts run=$key $($_.Exception.Message)"
            }
        }
    }
    Save-MonitorState $State
}

$state = Get-MonitorState
Write-MonitorLog "HEARTBEAT monitor started pid=$PID interval=${IntervalSeconds}s"
while ($true) {
    try {
        Invoke-MonitorScan -State $state
    } catch {
        Write-MonitorLog "ERROR scan: $($_.Exception.Message)"
    }
    if ($Once) { break }
    Start-Sleep -Seconds $IntervalSeconds
}
