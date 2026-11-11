<#
.SYNOPSIS
    Authenticode-sign is-alive.exe with your own certificate, for internal
    deployment, or make a self-signed test certificate to try it out.

.DESCRIPTION
    Signing makes Windows name a publisher instead of "Unknown publisher",
    but only on machines that trust the certificate's root:

    - A company signing with its internal CA (AD CS) or a self-signed
      certificate pushes that trust with Group Policy or Intune: the root
      into Trusted Root Certification Authorities and the signing
      certificate into Trusted Publishers. Then it is the company's own
      build, which the license (internal use) allows.
    - It does nothing for people downloading the public release: that
      needs a publicly trusted certificate, signed in CI. SmartScreen
      treats a self-signed signature like no signature at all.

    See docs/SIGNING.md. Signs in place with a SHA-256 digest and a
    timestamp, so the signature stays valid after the certificate expires.
    Windows only (Windows PowerShell 5.1 or PowerShell 7).

.EXAMPLE
    # Test on a VM, as administrator: make a test certificate, trust it on
    # this machine, sign.
    .\sign-windows.ps1 -Path .\is-alive.exe -CreateTestCert -TrustOnThisMachine

.EXAMPLE
    # A code-signing certificate already in your store (or LocalMachine's).
    .\sign-windows.ps1 -Path .\is-alive.exe -Thumbprint 0123456789ABCDEF0123456789ABCDEF01234567

.EXAMPLE
    # A .pfx file; the password is prompted for.
    .\sign-windows.ps1 -Path .\is-alive.exe -PfxPath .\codesign.pfx

.EXAMPLE
    # Undo the test: remove the test certificate from every store it went into.
    .\sign-windows.ps1 -RemoveTestCert
#>
[CmdletBinding(DefaultParameterSetName = 'Store')]
param(
    # The file to sign. Default: is-alive.exe here, else target\release\is-alive.exe.
    [string]$Path,

    # Certificate in CurrentUser\My or LocalMachine\My, by thumbprint...
    [Parameter(ParameterSetName = 'Store')]
    [string]$Thumbprint,
    # ...or by subject (substring match, e.g. "Contoso"). Newest one wins.
    [Parameter(ParameterSetName = 'Store')]
    [Parameter(ParameterSetName = 'Test')]
    [string]$Subject,

    # A .pfx (PKCS #12) file holding the certificate and its private key.
    [Parameter(ParameterSetName = 'Pfx', Mandatory)]
    [string]$PfxPath,
    [Parameter(ParameterSetName = 'Pfx')]
    [securestring]$PfxPassword,

    # Make a self-signed code-signing certificate (CurrentUser\My), export
    # its public part to -CerPath, and sign with it. For testing only.
    [Parameter(ParameterSetName = 'Test', Mandatory)]
    [switch]$CreateTestCert,
    [Parameter(ParameterSetName = 'Test')]
    [string]$CerPath = 'is-alive-test-signing.cer',
    # Also trust it on this machine (LocalMachine Root + TrustedPublisher).
    # Needs an elevated prompt.
    [Parameter(ParameterSetName = 'Test')]
    [switch]$TrustOnThisMachine,

    # Remove the test certificate (by -Subject) from My, Root and
    # TrustedPublisher, then stop. Needs an elevated prompt for LocalMachine.
    [Parameter(ParameterSetName = 'Remove', Mandatory)]
    [switch]$RemoveTestCert,
    [Parameter(ParameterSetName = 'Remove')]
    [string]$TestSubject = 'CN=is-alive test signing (self-signed)',

    # Timestamp server; http:// only (Set-AuthenticodeSignature can't use
    # https). -NoTimestamp signs without one (offline VM); such a signature
    # turns invalid when the certificate expires.
    [string]$TimestampServer = 'http://timestamp.digicert.com',
    [switch]$NoTimestamp
)

$ErrorActionPreference = 'Stop'
$DefaultTestSubject = 'CN=is-alive test signing (self-signed)'

function Test-Admin {
    $id = [Security.Principal.WindowsIdentity]::GetCurrent()
    ([Security.Principal.WindowsPrincipal]$id).IsInRole([Security.Principal.WindowsBuiltInRole]::Administrator)
}

if ($RemoveTestCert) {
    $stores = 'Cert:\CurrentUser\My', 'Cert:\CurrentUser\Root', 'Cert:\CurrentUser\TrustedPublisher',
              'Cert:\LocalMachine\My', 'Cert:\LocalMachine\Root', 'Cert:\LocalMachine\TrustedPublisher'
    $found = $false
    foreach ($store in $stores) {
        foreach ($c in Get-ChildItem $store | Where-Object Subject -EQ $TestSubject) {
            $found = $true
            try {
                Remove-Item -LiteralPath $c.PSPath
                "Removed $($c.Thumbprint) from $store"
            } catch {
                Write-Warning "Could not remove $($c.Thumbprint) from $store (run elevated?): $($_.Exception.Message)"
            }
        }
    }
    if (-not $found) { "No certificate with subject '$TestSubject' found." }
    return
}

if (-not $Path) {
    $Path = @('is-alive.exe', 'target\release\is-alive.exe') | Where-Object { Test-Path -LiteralPath $_ } | Select-Object -First 1
    if (-not $Path) { throw 'No is-alive.exe here or in target\release; pass -Path.' }
}
$Path = (Resolve-Path -LiteralPath $Path).ProviderPath

# The certificate to sign with.
if ($CreateTestCert) {
    if ($TrustOnThisMachine -and -not (Test-Admin)) {
        throw '-TrustOnThisMachine writes to LocalMachine stores: run PowerShell as administrator.'
    }
    if (-not $Subject) { $Subject = $DefaultTestSubject }
    if (-not $Subject.StartsWith('CN=')) { $Subject = "CN=$Subject" }
    $cert = New-SelfSignedCertificate -Type CodeSigningCert -Subject $Subject `
        -CertStoreLocation Cert:\CurrentUser\My -KeyAlgorithm RSA -KeyLength 3072 `
        -HashAlgorithm SHA256 -NotAfter (Get-Date).AddYears(2) `
        -FriendlyName 'is-alive test signing'
    $CerPath = [IO.Path]::GetFullPath((Join-Path (Get-Location).ProviderPath $CerPath))
    Export-Certificate -Cert $cert -FilePath $CerPath -Type CERT | Out-Null
    "Created $($cert.Subject) ($($cert.Thumbprint)) in CurrentUser\My"
    "Public certificate (no private key): $CerPath"
    if ($TrustOnThisMachine) {
        # Root: the chain is trusted. TrustedPublisher: Windows treats files
        # it signs as from a known publisher (no prompt where policy asks).
        Import-Certificate -FilePath $CerPath -CertStoreLocation Cert:\LocalMachine\Root | Out-Null
        Import-Certificate -FilePath $CerPath -CertStoreLocation Cert:\LocalMachine\TrustedPublisher | Out-Null
        'Trusted on this machine: LocalMachine\Root and LocalMachine\TrustedPublisher'
    }
} elseif ($PfxPath) {
    if (-not $PfxPassword) { $PfxPassword = Read-Host -AsSecureString "Password for $PfxPath" }
    $cert = New-Object System.Security.Cryptography.X509Certificates.X509Certificate2(
        (Resolve-Path -LiteralPath $PfxPath).ProviderPath, $PfxPassword)
    if (-not $cert.HasPrivateKey) { throw "$PfxPath holds no private key." }
} else {
    $all = @(Get-ChildItem Cert:\CurrentUser\My, Cert:\LocalMachine\My -CodeSigningCert |
        Where-Object { $_.HasPrivateKey -and $_.NotAfter -gt (Get-Date) })
    $cert = $all | Where-Object {
        ($Thumbprint -and $_.Thumbprint -eq ($Thumbprint -replace '\s', '').ToUpper()) -or
        ($Subject -and $_.Subject -like "*$Subject*")
    } | Sort-Object NotAfter -Descending | Select-Object -First 1
    if (-not $cert) {
        $list = ($all | ForEach-Object { "  $($_.Thumbprint)  $($_.Subject)  (until $($_.NotAfter.ToString('yyyy-MM-dd')))" }) -join "`n"
        if (-not $list) { $list = '  (none)' }
        throw "Pick a certificate with -Thumbprint or -Subject, or use -PfxPath / -CreateTestCert. Code-signing certificates with a private key:`n$list"
    }
}

# Sign. -HashAlgorithm is the file digest.
$params = @{
    FilePath      = $Path
    Certificate   = $cert
    HashAlgorithm = 'SHA256'
    IncludeChain  = 'NotRoot'
}
if (-not $NoTimestamp) { $params.TimestampServer = $TimestampServer }
$null = Set-AuthenticodeSignature @params

$sig = Get-AuthenticodeSignature -LiteralPath $Path
"Signed:    $Path"
"Signer:    $($sig.SignerCertificate.Subject)"
"Timestamp: $(if ($sig.TimeStamperCertificate) { $sig.TimeStamperCertificate.Subject } else { 'none' })"
"Status:    $($sig.Status)"
if (-not $NoTimestamp -and -not $sig.TimeStamperCertificate) {
    Write-Warning "No timestamp from $TimestampServer (offline?): the signature ends when the certificate does."
}
switch ($sig.Status) {
    'Valid' { 'This machine trusts the signature: Windows will show the publisher name.' }
    { $_ -in 'UnknownError', 'NotTrusted' } {
        Write-Warning ("Signed, but this machine does not trust the certificate's root ($($sig.StatusMessage)). " +
            'Import the root into Trusted Root Certification Authorities (and the certificate into ' +
            'Trusted Publishers) on the machines that should trust it; see docs/SIGNING.md.')
    }
    default { throw "Signing failed: $($sig.Status) $($sig.StatusMessage)" }
}
