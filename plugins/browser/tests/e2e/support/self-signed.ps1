param(
    [int] $LifetimeSeconds = 3600,
    [string] $FindThumbprint
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
try {
    $request = [System.Security.Cryptography.X509Certificates.CertificateRequest]::new(
        'CN=localhost',
        $rsa,
        [System.Security.Cryptography.HashAlgorithmName]::SHA256,
        [System.Security.Cryptography.RSASignaturePadding]::Pkcs1
    )
    $san = [System.Security.Cryptography.X509Certificates.SubjectAlternativeNameBuilder]::new()
    $san.AddDnsName('localhost')
    $san.AddIpAddress([ipaddress]::Loopback)
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
    $cert = $request.CreateSelfSigned($now.AddMinutes(-1), $now.AddSeconds($LifetimeSeconds))
    try {
        @{
            cert = $cert.ExportCertificatePem()
            key = $rsa.ExportPkcs8PrivateKeyPem()
        } | ConvertTo-Json -Compress
    } finally {
        $cert.Dispose()
    }
} finally {
    $rsa.Dispose()
}
