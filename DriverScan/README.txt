TierS Driver Manage - scan and install drivers with one click (Windows 10/11 x64)

INSTALL
1. Run TierS-Driver-Manage-Setup.exe and follow the wizard (needs administrator rights and an internet connection).
2. Start "TierS Driver Manage" from the Start menu or the desktop shortcut.

USE
1. Click "Scan drivers". The program reads this PC's hardware and looks for newer drivers in:
   - Windows Update
   - Microsoft Update Catalog (searched by hardware ID, WHQL-signed)
   - NVIDIA (the manufacturer's own service, for GeForce cards)
   - the program's signed online catalog and your own driverdb.json
2. Review the list. Chipset/system drivers are not ticked by default. Devices with no driver and no match are listed as "No driver found automatically".
3. Click "Install". Progress is shown per driver. When it finishes you can restart the computer from the same screen.

SAFETY
- Only https downloads from an allowed host list that is built into the program (a catalog can never widen it).
- Every file is checked: SHA-256 (or SHA-1 for Microsoft Catalog), and installers must carry a valid digital signature from the expected publisher.
- A system restore point is created before installing (Windows allows one every 24 hours).
- The program updates itself from a signed manifest; updates never contain executables and are rolled back on any error.

YOUR OWN CATALOG
- Add drivers to driverdb.json with New-CatalogEntry.ps1 (downloads the file, checks hash and signature, writes the entry).
- Use -ExpectedSha512 when the vendor prints a SHA-512 on its download page.

FILES
- Log: C:\ProgramData\TierS Driver Manage\DriverScan.log
- Shortcuts, uninstall entry and the Setup wizard are created by the installer.
