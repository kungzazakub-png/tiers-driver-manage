# One-time: creates the update signing key pair.
#   private key -> %USERPROFILE%\.tiers-update\private.xml   (keep secret, back it up; whoever has it can publish updates)
#   public key  -> DriverScan\UpdatePublicKey.xml            (ships inside the program; it only verifies)
$dir = Join-Path $env:USERPROFILE '.tiers-update'
$priv = Join-Path $dir 'private.xml'
if (Test-Path $priv) { throw "A private key already exists at $priv. Delete it only if you accept that installed copies stop trusting new updates." }
New-Item -ItemType Directory -Force $dir | Out-Null
$rsa = New-Object Security.Cryptography.RSACryptoServiceProvider(3072)
[IO.File]::WriteAllText($priv, $rsa.ToXmlString($true))
& icacls.exe $dir /inheritance:r /grant:r "$($env:USERNAME):(OI)(CI)F" | Out-Null
[IO.File]::WriteAllText((Join-Path $PSScriptRoot 'DriverScan\UpdatePublicKey.xml'), $rsa.ToXmlString($false))
"Created. Private key: $priv"
