# Report -> resolve -> publish pipeline

```
app (user clicks "Report these devices")
  -> Cloudflare Worker  (backend/worker)          validates, rate-limits, keeps the report in KV (no GitHub credential in Cloudflare)
  -> GitHub Action      (.github/workflows/auto-fetch.yml, hourly, windows runner) pulls /pending with a shared read key, then /ack

        fetcher.exe     (app/src/bin/fetcher.rs)   hardware ID -> Microsoft Update Catalog -> cab
                                                   -> SHA-1 check, INF mentions the ID, .cat files Microsoft-signed -> inf-zip
        gh release      "drivers"                  the zip is mirrored there (the program only trusts this repository's releases)
        Publish-Catalog.ps1                        validates every entry, signs, uploads the "catalog" release
        git commit      DriverScan/driverdb.json
  -> every installed copy gets the entry at its next scan (signed catalog, SHA-256 of the zip)
```

## One-time setup
1. **Worker**: `cd backend/worker; npx wrangler kv namespace create RATE` (paste the id into `wrangler.toml`), then `npx wrangler deploy`. A workers.dev subdomain must exist for the account.
2. **Secrets**: run `backend/setup-secrets.ps1`. It creates a random read key and stores it as Worker secret `READ_KEY` and GitHub secret `REPORT_READ_KEY`, and sets the variable `REPORT_WORKER_URL`.
3. **Signing key** (to publish without a person): GitHub secret `UPDATE_PRIVATE_KEY_XML` = contents of `%USERPROFILE%\.tiers-update\private.xml`. See the risk note below.
4. `REPORT_URL` in `app/src/ui.rs` points at the Worker (`.../report`); release a program version to show the Report button.

Manual run: Actions -> "Auto-fetch drivers..." -> Run workflow -> enter one hardware ID (or leave it empty to collect stored reports now).
A reported device that the fetcher skipped (for example "no driver in the Microsoft Update Catalog") is final: the report is removed after a successful run.
## What is and is not trusted
- The Worker holds no GitHub credential. Reading or deleting stored reports needs the read key.
- A report can only choose a hardware ID (letters, digits and `_ & \ - . # { }`, max 200 chars). It never supplies a URL, a hash or a file.
- The fetcher only accepts packages that Microsoft publishes: cab SHA-1 equals Microsoft's value, the INF names the device, every `.cat` is Valid and signed by Microsoft. Otherwise the report is skipped.
- Programs still verify the zip's SHA-256 and install with `pnputil`, which Windows only accepts for signed packages.

## Risk to know
The signing key becomes a repository secret. Anyone who can push to this repository or edit the workflow could use it to sign a catalog. Keep branch protection on `main`, restrict who has write access, and prefer a repository with no outside collaborators. If you do not want the key in GitHub, drop the "Sign and publish" step and the "Commit" step: the Action then only mirrors the zips, and you sign locally with `Publish-Catalog.ps1`.
