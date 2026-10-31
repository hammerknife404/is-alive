<#
.SYNOPSIS
    Save a Meraki organization's MX config as one JSON file for is-alive's
    Config Export.

.DESCRIPTION
    Read-only: GET calls to the Dashboard API, nothing else. The API key comes
    from MERAKI_DASHBOARD_API_KEY or a hidden prompt; it is never written.

    The file holds what the API returns, pre-shared keys and VPN secrets
    included, so keep it as private as the config itself. is-alive strips
    those when it converts it, and the converted JSON is what you share.

    Saves: the organization, its networks and devices, uplink status and
    third-party VPN peers; per appliance network: VLANs (or the single LAN),
    ports, L3 / inbound / L7 firewall rules, port forwarding, 1:1 and 1:many
    NAT, static routes, site-to-site VPN and appliance SSIDs; per MX: uplink
    settings. Needs PowerShell 5.1 or later and a key with read access.

.EXAMPLE
    .\meraki-export.ps1 -Org "Harbor Dental" -Out harbor.meraki.json

.EXAMPLE
    .\meraki-export.ps1 -Org "Harbor Dental" -Network "Main Office"
#>
param(
    # Organization name (or id). Leave out to list the ones the key can see.
    [string]$Org,
    # Only networks whose name contains this.
    [string]$Network,
    [string]$Out = "meraki-export.json"
)

$ErrorActionPreference = "Stop"
[Net.ServicePointManager]::SecurityProtocol = [Net.SecurityProtocolType]::Tls12

$key = $env:MERAKI_DASHBOARD_API_KEY
if (-not $key) {
    $secure = Read-Host "Meraki API key" -AsSecureString
    $bstr = [Runtime.InteropServices.Marshal]::SecureStringToBSTR($secure)
    try { $key = [Runtime.InteropServices.Marshal]::PtrToStringBSTR($bstr) }
    finally { [Runtime.InteropServices.Marshal]::ZeroFreeBSTR($bstr) }
}
$base = "https://api.meraki.com/api/v1"
$headers = @{ Authorization = "Bearer $key"; Accept = "application/json" }

# One GET. Rate limits (429) and brief outages are retried; a call the
# network doesn't support (400/404, e.g. VLANs off or no MX) gives $null.
function Get-Meraki([string]$Path) {
    for ($try = 1; $try -le 6; $try++) {
        try {
            $r = Invoke-RestMethod -Uri "$base$Path" -Headers $headers -Method Get -TimeoutSec 60
            # Keep a one-item list a list.
            if ($r -is [array]) { return , $r }
            return $r
        } catch {
            $code = 0
            if ($_.Exception.Response) { $code = [int]$_.Exception.Response.StatusCode }
            if ($code -eq 400 -or $code -eq 404) { return $null }
            if ($code -eq 401 -or $code -eq 403) { throw "Meraki refused the key for $Path ($code)" }
            if ($try -eq 6) { throw }
            Start-Sleep -Seconds ([math]::Pow(2, $try - 1))
        }
    }
}

$orgs = Get-Meraki "/organizations"
if (-not $Org) {
    Write-Host "Organizations this key can read:"
    $orgs | ForEach-Object { Write-Host ("  {0}  {1}" -f $_.id, $_.name) }
    Write-Host "Run again with -Org <name or id>."
    return
}
$o = $orgs | Where-Object { $_.id -eq $Org -or $_.name -eq $Org } | Select-Object -First 1
if (-not $o) { throw "No organization named or numbered '$Org' for this key" }
$oid = $o.id
Write-Host "Organization: $($o.name)"

$devices = Get-Meraki "/organizations/$oid/devices?perPage=1000"
$networks = Get-Meraki "/organizations/$oid/networks?perPage=1000"
if ($Network) { $networks = @($networks | Where-Object { $_.name -like "*$Network*" }) }

$calls = [ordered]@{
    vlans                 = "appliance/vlans"
    singleLan             = "appliance/singleLan"
    ports                 = "appliance/ports"
    l3FirewallRules       = "appliance/firewall/l3FirewallRules"
    inboundFirewallRules  = "appliance/firewall/inboundFirewallRules"
    l7FirewallRules       = "appliance/firewall/l7FirewallRules"
    portForwardingRules   = "appliance/firewall/portForwardingRules"
    oneToOneNatRules      = "appliance/firewall/oneToOneNatRules"
    oneToManyNatRules     = "appliance/firewall/oneToManyNatRules"
    staticRoutes          = "appliance/staticRoutes"
    siteToSiteVpn         = "appliance/vpn/siteToSiteVpn"
    ssids                 = "appliance/ssids"
}

$nets = foreach ($n in $networks) {
    $entry = [ordered]@{
        id = $n.id; name = $n.name; productTypes = $n.productTypes
        timeZone = $n.timeZone; tags = $n.tags
    }
    if ($n.productTypes -contains "appliance") {
        Write-Host "  $($n.name)"
        $appliance = [ordered]@{}
        foreach ($k in $calls.Keys) {
            $r = Get-Meraki "/networks/$($n.id)/$($calls[$k])"
            if ($null -ne $r) { $appliance[$k] = $r }
        }
        $entry.appliance = $appliance
        $mx = @($devices | Where-Object { $_.networkId -eq $n.id -and $_.productType -eq "appliance" })
        $entry.uplinkSettings = @(foreach ($d in $mx) {
            $u = Get-Meraki "/devices/$($d.serial)/appliance/uplinks/settings"
            if ($u) { [ordered]@{ serial = $d.serial; interfaces = $u.interfaces } }
        })
    }
    $entry
}

$export = [ordered]@{
    source             = "meraki-export.ps1"
    exported           = (Get-Date).ToString("o")
    organization       = $o
    devices            = @($devices)
    networks           = @($nets)
    uplinkStatuses     = Get-Meraki "/organizations/$oid/appliance/uplink/statuses"
    thirdPartyVPNPeers = Get-Meraki "/organizations/$oid/appliance/vpn/thirdPartyVPNPeers"
}
$export | ConvertTo-Json -Depth 30 | Set-Content -Path $Out -Encoding UTF8
Write-Host "Saved $Out. Open it in is-alive's Config Export; share the converted JSON, not this file."
