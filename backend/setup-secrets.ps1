# One-time: creates a random read key and stores it as the Worker secret READ_KEY and the GitHub secret REPORT_READ_KEY,
# and sets the repository variable REPORT_WORKER_URL. The key is never printed or saved to a file.
#   .\backend\setup-secrets.ps1
$ErrorActionPreference = 'Stop'
$repo = 'kungzazakub-png/tiers-driver-manage'
$url = 'https://tiers-hwid-report.tiers-driver-manage.workers.dev'
$env:Path += ';C:\Program Files\GitHub CLI'
$bytes = New-Object byte[] 32
[Security.Cryptography.RandomNumberGenerator]::Create().GetBytes($bytes)
$key = [BitConverter]::ToString($bytes).Replace('-', '').ToLower()
Push-Location (Join-Path $PSScriptRoot 'worker')
try { $key | npx --yes wrangler@4 secret put READ_KEY } finally { Pop-Location }
$key | gh secret set REPORT_READ_KEY --repo $repo
gh variable set REPORT_WORKER_URL --body $url --repo $repo
'Worker status codes (expect 401, 401, 200):'
foreach ($h in @($null, 'Bearer 0000', "Bearer $key")) {
    $args2 = @('-s', '-o', 'NUL', '-w', '%{http_code}'); if ($h) { $args2 += @('-H', "Authorization: $h") }
    curl.exe @args2 "$url/pending"; ''
}
'Done. The hourly Action will now collect reports.'