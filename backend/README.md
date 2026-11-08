# Report -> resolve -> publish pipeline

```
app (user clicks "Report these devices")
  -> Cloudflare Worker  (backend/worker)          validates, rate-limits, files a GitHub issue (token can only write issues)
  -> GitHub Action      (.github/workflows/auto-fetch.yml, windows runner)
        fetcher.exe     (app/src/bin/fetcher.rs)   hardware ID -> Microsoft Update Catalog -> cab
                                                   -> SHA-1 check, INF mentions the ID, .cat files Microsoft-signed -> inf-zip
        gh release      "drivers"                  the zip is mirrored there (the program only trusts this repository's releases)
        Publish-Catalog.ps1                        validates every entry, signs, uploads the "catalog" release
        git commit      DriverScan/driverdb.json
  -> every installed copy gets the entry at its next scan (signed catalog, SHA-256 of the zip)
```

## One-time setup
1. **Worker**: `cd backend/worker; npx wrangler deploy`. Then `npx wrangler secret put GH_TOKEN` with a fine-grained token for THIS repository only, permission "Issues: read and write". Optional: `npx wrangler kv namespace create RATE` and paste the id into `wrangler.toml` (rate limit + 7-day de-duplication).
2. **Repository variable** `REPORT_BOT_LOGIN` = the GitHub login that owns the token (only issues filed by that account are trusted).
3. **Repository secret** `UPDATE_PRIVATE_KEY_XML` = the contents of `%USERPROFILE%\.tiers-update\private.xml` (the catalog signing key). See the risk note below.
4. Put the Worker address in `REPORT_URL` (`app/src/ui.rs`, e.g. `https://tiers-hwid-report.<you>.workers.dev/report`) and release a new program version. While it is empty the Report button is hidden.

Manual run: Actions -> "Auto-fetch drivers..." -> Run workflow -> enter one hardware ID.

## What is and is not trusted
- A report can only choose a hardware ID (letters, digits and `_ & \ - . # { }`, max 200 chars). It never supplies a URL, a hash or a file.
- The fetcher only accepts packages that Microsoft publishes: cab SHA-1 equals Microsoft's value, the INF names the device, every `.cat` is Valid and signed by Microsoft. Otherwise the report is skipped.
- Programs still verify the zip's SHA-256 and install with `pnputil`, which Windows only accepts for signed packages.

## Risk to know
The signing key becomes a repository secret. Anyone who can push to this repository or edit the workflow could use it to sign a catalog. Keep branch protection on `main`, restrict who has write access, and prefer a repository with no outside collaborators. If you do not want the key in GitHub, drop the "Sign and publish" step and the "Commit" step: the Action then only mirrors the zips, and you sign locally with `Publish-Catalog.ps1`.
