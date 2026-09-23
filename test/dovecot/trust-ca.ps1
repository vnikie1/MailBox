<#
Trusts the test Dovecot's CA for the current Windows user, and removes every other
"Halcyon Test CA" that user trusts — the ones it replaces.

    powershell -NoProfile -ExecutionPolicy Bypass -File test\dovecot\trust-ca.ps1 -Path <ca.crt>

Windows asks for confirmation for each certificate added to or removed from a user's root
store. That prompt is the point: of a whole renewal, this is the one step that should need a
person. README.md, "The certificate", has the rest.
#>
param(
    [Parameter(Mandatory)] [string] $Path
)

$ErrorActionPreference = 'Stop'

$file = (Resolve-Path $Path).ProviderPath
$new = New-Object System.Security.Cryptography.X509Certificates.X509Certificate2 -ArgumentList $file

# A guard against the wrong file: a root store is not the place to find out.
if ($new.Subject -notlike 'CN=Halcyon Test CA*') {
    throw "$Path is '$($new.Subject)', not a Halcyon Test CA. Nothing was changed."
}
if (-not ($new.Extensions | Where-Object { $_.Oid.Value -eq '2.5.29.30' })) {
    throw "$Path has no name constraints, so it could vouch for any site. certs.sh adds them; nothing was changed."
}

$store = New-Object System.Security.Cryptography.X509Certificates.X509Store -ArgumentList 'Root', 'CurrentUser'
$store.Open('ReadWrite')
try {
    if ($store.Certificates.Find('FindByThumbprint', $new.Thumbprint, $false).Count -eq 0) {
        $store.Add($new)
    }

    # Only after the new one is in, so a "No" to the first prompt leaves the rig as it was.
    $old = @($store.Certificates | Where-Object {
            $_.Subject -like 'CN=Halcyon Test CA*' -and $_.Thumbprint -ne $new.Thumbprint
        })
    foreach ($cert in $old) {
        $store.Remove($cert)
    }
} finally {
    $store.Close()
}

Get-ChildItem Cert:\CurrentUser\Root |
    Where-Object { $_.Subject -like 'CN=Halcyon Test CA*' } |
    ForEach-Object { "trusted: $($_.Subject), until $($_.NotAfter.ToString('yyyy-MM-dd')), $($_.Thumbprint)" }
