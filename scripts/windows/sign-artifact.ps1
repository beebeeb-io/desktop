<#
.SYNOPSIS
  Tauri bundle.windows.signCommand wrapper - Azure Artifact Signing (public trust).

.DESCRIPTION
  Invoked by the Tauri bundler once per file that needs signing (the app .exe,
  sidecars, the NSIS installer and its uninstaller, the MSI). Publisher is the
  validated organization behind the certificate profile: CN=Initlabs B.V.

  Authentication is ambient Azure auth (Azure CLI login) - the Microsoft
  Artifact Signing dlib acquires the token itself via DefaultAzureCredential.
  In CI, `azure/login` (OIDC federated credential) provides that login. There
  are no signing credentials in this repo, in env vars, or in CI secrets.

  Identity validation id: 3177670b-59dd-4d4b-acc5-8f10e8916fec (decision 1499).
  Service coordinates are non-secret defaults, overridable by env:

    AZURE_SIGNING_ENDPOINT  (default https://weu.codesigning.azure.net/)
    AZURE_SIGNING_ACCOUNT   (default beebeebsigning)
    AZURE_SIGNING_PROFILE   (default beebeeb-publictrust)

  Behavior on failure (task 1617: "missing credentials leaves this blocked, not
  unsigned-green"): with BEEBEEB_REQUIRE_SIGNING=1 (set only by release CI) any
  problem is a hard failure; otherwise this script warns and leaves the file
  unsigned so local Windows bundling keeps working. Publication is separately
  blocked on signature presence by scripts/windows/verify-authenticode.ps1.

.NOTES
  Timestamping is mandatory: Artifact Signing certificates live 3 days; the
  RFC3161 timestamp keeps signatures valid after cert rotation.
#>
[CmdletBinding()]
param(
  [Parameter(Mandatory = $true, Position = 0)]
  [string]$FilePath,
  [string]$Description = 'Beebeeb'
)

Set-StrictMode -Version Latest
$ErrorActionPreference = 'Stop'

$Endpoint = if ($env:AZURE_SIGNING_ENDPOINT) { $env:AZURE_SIGNING_ENDPOINT } else { 'https://weu.codesigning.azure.net/' }
$Account  = if ($env:AZURE_SIGNING_ACCOUNT)  { $env:AZURE_SIGNING_ACCOUNT }  else { 'beebeebsigning' }
$Profile  = if ($env:AZURE_SIGNING_PROFILE)  { $env:AZURE_SIGNING_PROFILE }  else { 'beebeeb-publictrust' }
$Required = $env:BEEBEEB_REQUIRE_SIGNING -eq '1'
$CacheDir = Join-Path $env:LOCALAPPDATA 'beebeeb-signing'

function Stop-Signing([string]$Message) {
  if ($Required) {
    Write-Host "::error::$Message"
    exit 1
  }
  Write-Warning "$Message - leaving the file UNSIGNED (local build; BEEBEEB_REQUIRE_SIGNING=1 makes this a hard failure)."
  exit 0
}

function Find-SignTool {
  if ($env:TAURI_WINDOWS_SIGNTOOL_PATH) { return $env:TAURI_WINDOWS_SIGNTOOL_PATH }
  $cmd = Get-Command signtool.exe -ErrorAction SilentlyContinue
  if ($cmd) { return $cmd.Source }
  # Newest x64 signtool from any installed Windows SDK. The Artifact Signing
  # dlib needs >= 10.0.2261.755; version sort picks the newest anyway.
  $candidates = @()
  foreach ($root in @('C:\Program Files (x86)\Windows Kits\10\bin', 'C:\Program Files\Windows Kits\10\bin')) {
    if (Test-Path $root) {
      $candidates += Get-ChildItem -Path (Join-Path $root '*\x64\signtool.exe') -ErrorAction SilentlyContinue
      $candidates += Get-ChildItem -Path (Join-Path $root 'x64\signtool.exe') -ErrorAction SilentlyContinue
    }
  }
  $versioned = @($candidates | Where-Object { $_.Directory.Parent.Name -match '^\d' })
  $best = $versioned | Sort-Object { [version]($_.Directory.Parent.Name -replace '[^\d\.]','') } -Descending | Select-Object -First 1
  if ($best) { return $best.FullName }
  $flat = @($candidates | Select-Object -First 1)
  if ($flat.Count -gt 0 -and $flat[0]) { return $flat[0].FullName }
  return $null
}

function Install-ArtifactSigningDlib {
  # Returns the x64 dlib path, downloading the Microsoft package on first use.
  $dll = Get-ChildItem -Path $CacheDir -Recurse -Filter '*Dlib.dll' -ErrorAction SilentlyContinue |
    Where-Object { $_.FullName -match '\\x64\\' } | Select-Object -First 1
  if ($dll) { return $dll.FullName }

  New-Item -ItemType Directory -Force -Path $CacheDir | Out-Null
  $zip = Join-Path $CacheDir 'microsoft.artifactsigning.client.zip'
  $dst = Join-Path $CacheDir 'dlib'
  Write-Host "Downloading Microsoft.ArtifactSigning.Client (dlib)..."
  try {
    # PowerShell 5.1 defaults can exclude TLS 1.2 on older images.
    [Net.ServicePointManager]::SecurityProtocol = [Net.ServicePointManager]::SecurityProtocol -bor [Net.SecurityProtocolType]::Tls12
    # Progress rendering on PS 5.1 makes multi-MB downloads pathologically
    # slow (the first CI release run hung here for ~50 minutes before the
    # cancel); suppress the progress bar and bound the download.
    $ProgressPreference = 'SilentlyContinue'
    # Version pinned deliberately (supply-chain hardening, PR #90 review):
    # 1.0.128 verified to carry bin/x64/Azure.CodeSigning.Dlib.dll. Bump
    # explicitly after checking the new package.
    Invoke-WebRequest -TimeoutSec 300 -Uri 'https://www.nuget.org/api/v2/package/Microsoft.ArtifactSigning.Client/1.0.128' -OutFile $zip -UseBasicParsing
    Expand-Archive -Path $zip -DestinationPath $dst -Force
  } catch {
    Stop-Signing "Could not download/extract the Artifact Signing dlib: $($_.Exception.Message)"
  }
  # The nupkg may carry the binaries directly or wrap them in an inner zip.
  Get-ChildItem -Path $dst -Recurse -Filter '*.zip' -ErrorAction SilentlyContinue | ForEach-Object {
    Expand-Archive -Path $_.FullName -DestinationPath (Join-Path $_.Directory.FullName $_.BaseName) -Force
  }
  $dll = Get-ChildItem -Path $dst -Recurse -Filter '*Dlib.dll' -ErrorAction SilentlyContinue |
    Where-Object { $_.FullName -match '\\x64\\' } | Select-Object -First 1
  if (-not $dll) {
    $dll = Get-ChildItem -Path $dst -Recurse -Filter '*Dlib.dll' -ErrorAction SilentlyContinue | Select-Object -First 1
  }
  if (-not $dll) { Stop-Signing "Artifact Signing dlib not found after extraction (looked for *Dlib.dll under $dst)" }
  return $dll.FullName
}

if (-not (Test-Path -LiteralPath $FilePath)) {
  Stop-Signing "sign-artifact: file not found: $FilePath"
}

function Ensure-AzFreshLogin {
  # azure/login's OIDC client assertion is valid for ~5 minutes, but the build
  # signs ~19 minutes after the login step (release compile time). By sign
  # time the az CLI cache holds an EXPIRED assertion and the dlib's
  # AzureCliCredential fails with AADSTS700024 (measured in release run 5).
  # Re-login az right here with a FRESH GitHub OIDC token, fetched using this
  # job's own id-token permission. No long-lived secret involved.
  $reqToken = $env:ACTIONS_ID_TOKEN_REQUEST_TOKEN
  $reqUrl = $env:ACTIONS_ID_TOKEN_REQUEST_URL
  $clientId = $env:AZURE_CLIENT_ID
  $tenantId = $env:AZURE_TENANT_ID
  if (-not $reqToken -or -not $reqUrl -or -not $clientId -or -not $tenantId) {
    Stop-Signing "sign-artifact: OIDC re-login prerequisites missing (ACTIONS_ID_TOKEN_REQUEST_* / AZURE_CLIENT_ID / AZURE_TENANT_ID) - is the build job's id-token permission set and azure/login configured?"
  }
  Write-Host "sign-artifact: refreshing az login with a fresh GitHub OIDC token..."
  [Net.ServicePointManager]::SecurityProtocol = [Net.ServicePointManager]::SecurityProtocol -bor [Net.SecurityProtocolType]::Tls12
  $resp = Invoke-RestMethod -Method Get `
    -Headers @{ Authorization = "Bearer $reqToken" } `
    -Uri ($reqUrl + $(if ($reqUrl.Contains('?')) { '&' } else { '?' }) + 'audience=api://AzureADTokenExchange') `
    -TimeoutSec 60
  if (-not $resp.value) { Stop-Signing "sign-artifact: GitHub OIDC token fetch returned no token" }
  & az login --service-principal -u $clientId -t $tenantId --federated-token $resp.value --output none
  if ($LASTEXITCODE -ne 0) {
    Stop-Signing "sign-artifact: az login with the fresh federated token failed (exit $LASTEXITCODE)"
  }
}

Ensure-AzFreshLogin

$signtool = Find-SignTool
if (-not $signtool) { Stop-Signing "sign-artifact: signtool.exe not found (install the Windows SDK)" }

$dlib = Install-ArtifactSigningDlib

$metadata = Join-Path $CacheDir 'metadata.json'
@{
  Endpoint               = $Endpoint
  CodeSigningAccountName = $Account
  CertificateProfileName = $Profile
  # The dlib runs DefaultAzureCredential internally; without exclusions it
  # probes Managed Identity (IMDS is absent on GitHub runners — packets are
  # silently dropped, not refused), SharedTokenCache and Visual Studio user
  # stores; the first signed release runs hung 33-45 minutes inside the first
  # signtool call with zero sign requests reaching the service (diagnosed
  # from the verbose bundler log: dlib init at 22:11:10Z, job cancelled at
  # 22:44Z with signtool still alive). The documented contract is an array of
  # PascalCase credential type names (learn.microsoft.com/azure/artifact-
  # signing/how-to-signing-integrations); application-time matching is
  # case-sensitive (Azure/trusted-signing-action#70), so spell exactly.
  # Keep Environment + WorkloadIdentity (azure/login's OIDC env) and
  # AzureCliCredential (logged in) as the credential chain.
  ExcludeCredentials = @(
    'ManagedIdentityCredential',
    'SharedTokenCacheCredential',
    'VisualStudioCredential',
    'VisualStudioCodeCredential',
    'AzureDeveloperCliCredential',
    'AzurePowerShellCredential',
    'InteractiveBrowserCredential'
  )
} | ConvertTo-Json | Set-Content -LiteralPath $metadata -Encoding ASCII

Write-Host "sign-artifact: signing '$FilePath' via $Account/$Profile ($Endpoint)"
& $signtool sign /v /fd SHA256 /d $Description `
    /tr http://timestamp.acs.microsoft.com /td SHA256 `
    /dlib $dlib /dmdf $metadata `
    $FilePath
if ($LASTEXITCODE -ne 0) {
  Stop-Signing "sign-artifact: signtool failed with exit code $LASTEXITCODE for '$FilePath' (check the dlib output above; the az login was refreshed at sign time)"
}

# Self-check: never trust the tool's exit code alone (evidence rule).
$sig = Get-AuthenticodeSignature -LiteralPath $FilePath
if ($sig.Status -ne 'Valid') {
  Stop-Signing "sign-artifact: post-sign check failed for '$FilePath': Status=$($sig.Status) ($($sig.StatusMessage))"
}
if ($sig.SignerCertificate.Subject -notmatch 'CN=Initlabs B\.V\.') {
  Stop-Signing "sign-artifact: post-sign check failed for '$FilePath': unexpected signer '$($sig.SignerCertificate.Subject)'"
}
if (-not $sig.TimeStamperCertificate) {
  Stop-Signing "sign-artifact: post-sign check failed for '$FilePath': no RFC3161 timestamp (signature would expire with the 3-day cert)"
}
Write-Host "sign-artifact: OK '$FilePath' - Valid, signer $($sig.SignerCertificate.Subject), timestamped by $($sig.TimeStamperCertificate.Subject)"
exit 0
