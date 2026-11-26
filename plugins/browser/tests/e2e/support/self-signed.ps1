param(
    [int] $LifetimeSeconds = 3600,
    [string] $FindThumbprint,
    [switch] $CaSigned,
    [switch] $MismatchedHost,
    [switch] $Expired
)

$ErrorActionPreference = 'Stop'

if ($FindThumbprint) {
    # Read the two trust stores without provisioning a test root.
    $found = Get-ChildItem -Path 'Cert:\CurrentUser\Root', 'Cert:\LocalMachine\Root' |
        Where-Object Thumbprint -eq $FindThumbprint |
        Select-Object -First 1
    if ($null -eq $found) {
        'absent'
    } else {
        'present'
    }
    return
}

$rsa = [System.Security.Cryptography.RSA]::Create(2048)
$caRsa = $null
$caCert = $null
try {
    $subject = if ($MismatchedHost) { 'CN=other.local' } elseif ($CaSigned) { 'CN=pve-fixture.local' } else { 'CN=localhost' }
    $request = [System.Security.Cryptography.X509Certificates.CertificateRequest]::new(
        $subject,
        $rsa,
        [System.Security.Cryptography.HashAlgorithmName]::SHA256,
        [System.Security.Cryptography.RSASignaturePadding]::Pkcs1
    )
    $san = [System.Security.Cryptography.X509Certificates.SubjectAlternativeNameBuilder]::new()
    if ($MismatchedHost) {
        $san.AddDnsName('other.local')
    } else {
        $san.AddDnsName('localhost')
        $san.AddIpAddress([ipaddress]::Loopback)
    }
    $request.CertificateExtensions.Add($san.Build())
    $usage = [System.Security.Cryptography.OidCollection]::new()
    [void] $usage.Add([System.Security.Cryptography.Oid]::new('1.3.6.1.5.5.7.3.1'))
    $request.CertificateExtensions.Add(
        [System.Security.Cryptography.X509Certificates.X509EnhancedKeyUsageExtension]::new($usage, $false)
    )
    $request.CertificateExtensions.Add(
        [System.Security.Cryptography.X509Certificates.X509KeyUsageExtension]::new(
            [System.Security.Cryptography.X509Certificates.X509KeyUsageFlags]::DigitalSignature,
            $true
        )
    )

    $now = [DateTimeOffset]::UtcNow
    $notBefore = if ($Expired) { $now.AddHours(-2) } else { $now.AddMinutes(-1) }
    $notAfter = if ($Expired) { $now.AddHours(-1) } else { $now.AddSeconds($LifetimeSeconds) }

    if ($CaSigned) {
        $caRsa = [System.Security.Cryptography.RSA]::Create(2048)
        $caRequest = [System.Security.Cryptography.X509Certificates.CertificateRequest]::new(
            'CN=Mannie Fixture Local CA, O=Mannie Tests',
            $caRsa,
            [System.Security.Cryptography.HashAlgorithmName]::SHA256,
            [System.Security.Cryptography.RSASignaturePadding]::Pkcs1
        )
        $caRequest.CertificateExtensions.Add(
            [System.Security.Cryptography.X509Certificates.X509BasicConstraintsExtension]::new($true, $false, 0, $true)
        )
        $caRequest.CertificateExtensions.Add(
            [System.Security.Cryptography.X509Certificates.X509KeyUsageExtension]::new(
                [System.Security.Cryptography.X509Certificates.X509KeyUsageFlags]::KeyCertSign -bor
                [System.Security.Cryptography.X509Certificates.X509KeyUsageFlags]::CrlSign,
                $true
            )
        )
        $caCert = $caRequest.CreateSelfSigned($now.AddDays(-1), $now.AddDays(1))
        $serial = [byte[]]::new(16)
        [System.Security.Cryptography.RandomNumberGenerator]::Fill($serial)
        $serial[0] = $serial[0] -band 0x7f
        $cert = $request.Create($caCert, $notBefore, $notAfter, $serial)
    } else {
        $cert = $request.CreateSelfSigned($notBefore, $notAfter)
    }

    try {
        @{
            cert = $cert.ExportCertificatePem()
            key = $rsa.ExportPkcs8PrivateKeyPem()
        } | ConvertTo-Json -Compress
    } finally {
        $cert.Dispose()
    }
} finally {
    if ($null -ne $caCert) { $caCert.Dispose() }
    if ($null -ne $caRsa) { $caRsa.Dispose() }
    $rsa.Dispose()
}
