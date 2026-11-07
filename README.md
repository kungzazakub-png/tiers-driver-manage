# TierS Driver Manage

Windows driver scanner/installer (scan, review, install). Sources: Windows Update, Microsoft Update Catalog, NVIDIA (direct), and your own `driverdb.json` catalog.

- Install: download `TierS-Driver-Manage-Setup.exe` from the latest release.
- The program updates itself: it checks `latest.json` on this repository's latest release at start-up. Updates are signed (RSA/SHA-256); only the public key (`DriverScan/UpdatePublicKey.xml`) is in this repository.
- Publish a new version: `.\Publish-Update.ps1 -Version x.y.z -Notes "..."`.
