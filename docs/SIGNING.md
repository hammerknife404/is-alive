# Code signing (Windows)

`is-alive.exe` is not code-signed yet. This page explains what Windows shows
because of that, what each signing option costs and fixes, how to switch on
signing in CI, and how a company can sign its own internal copy.

## What Windows shows, and why

On first run after a download, Windows 11 can show up to three things:

| Prompt | Cause | What signing changes |
|---|---|---|
| **Open File – Security Warning**: *"The publisher could not be verified"*, Publisher: **Unknown Publisher** | The file came from the internet (it carries the *Mark of the Web*) and has no Authenticode signature that this machine trusts. | A trusted signature shows the publisher's name instead. |
| **Windows protected your PC** (SmartScreen), *More info → Run anyway* | Microsoft's cloud has not seen enough clean downloads of this file or its publisher. | Signed files build reputation **per publisher**, so later versions inherit it. Unsigned files start from zero every release. A new signer still sees the prompt until reputation builds (weeks, hundreds of installs), but with a named publisher. |
| **Smart App Control** blocks the app (no *Run anyway*) | Smart App Control is on (some fresh Windows 11 installs) and the file is unsigned with no reputation. | A publicly trusted signature lets it run. |

The version info in the .exe (*CompanyName hammerknife404*, from `build.rs`)
does not count: only a signature from a certificate that chains to a root the
machine trusts does. Nothing in the code can remove the prompts without one.

Extended Validation (EV) certificates **no longer** get instant SmartScreen
reputation; Microsoft says paying for EV only for SmartScreen "is no longer
justified". An OV certificate or Azure Artifact Signing does the same job.

## Options and costs (late 2026)

| Option | Cost | Fits is-alive? |
|---|---|---|
| **Azure Artifact Signing** (formerly Trusted Signing), Microsoft-run, publicly trusted | Basic **$9.99/month** (5,000 signatures), Premium $99.99/month; $0.005 per extra signature. Billing starts when the account is created. | **Yes, wired in below.** Public Trust needs identity validation: organizations in the US, Canada, EU, UK, Australia, New Zealand, Japan, South Korea, Singapore, Switzerland, Norway or Israel; **individual developers only in the US or Canada**. Validation takes 1 to 20 business days. |
| **OV certificate from a public CA** (Certum, SSL.com, Sectigo, DigiCert...) | Roughly $65 to $300 a year for the certificate, plus the CA's cloud-signing fee (for example SSL.com eSigner from about $180 a year). | Works from anywhere, but needs that CA's cloud-signing tool in CI (not wired in; it would replace the Azure step). Since June 2023 the private key must live on a hardware token or cloud HSM, so a plain `.pfx` in a GitHub secret is no longer obtainable from a public CA. Since March 2026 certificates last at most 460 days. |
| **EV certificate** | $250 to $600+ a year | Same as OV for SmartScreen now; not worth the premium here. |
| **SignPath Foundation** (free for open source) | Free | **No.** It requires an OSI-approved license without commercial dual-licensing; PolyForm Internal Use is not OSI-approved. |
| **Microsoft Store** (MSIX) | Store account | No SmartScreen prompt at all (Microsoft re-signs), but means packaging and Store review; not planned. |
| **Your own CA or a self-signed certificate** | Free | Only on machines that trust it, so **only for internal deployment** (see below). Microsoft treats a self-signed signature like no signature for SmartScreen. It never fixes the public download. |

## Switching on Azure Artifact Signing in CI

The `windows` job in `.github/workflows/build.yml` signs `is-alive.exe` after
`cargo build` and before the upload, when the repository variable `SIGNING` is
`azure`. Until then it logs *"Code signing is not configured"* and builds
exactly as before. Pull request builds are never signed. A signed build must
pass **Signature valid** (`Get-AuthenticodeSignature` is `Valid` and
timestamped). The release job hashes the uploaded artifact, so
`is-alive.exe.sha256` covers the signed file.

One-time setup (Azure portal, about an hour plus the validation wait):

1. **Subscription.** In an Azure subscription, open *Subscriptions → your
   subscription → Resource providers*, and register **Microsoft.CodeSigning**.
2. **Account.** Create an **Artifact Signing account** (search "Artifact
   Signing Accounts" → Create), Basic tier, in a supported region. Note the
   region's endpoint, for example:

   | Region | Endpoint |
   |---|---|
   | West Europe | `https://weu.codesigning.azure.net/` |
   | North Europe | `https://neu.codesigning.azure.net/` |
   | East US | `https://eus.codesigning.azure.net/` |
   | West US 2 | `https://wus2.codesigning.azure.net/` |

   (Full list in Microsoft's [quickstart](https://learn.microsoft.com/azure/artifact-signing/quickstart).)
3. **Identity validation.** On the account, *Access control (IAM)*: give
   yourself **Artifact Signing Identity Verifier**. Then *Identity
   validations → New identity → Public*, as Organization or Individual, and
   follow the email and ID-check steps. The name you validate is what Windows
   shows as the publisher.
4. **Certificate profile.** *Certificate profiles → Create → Public Trust*,
   pick the completed validation, and name it (for example `isalive`).
5. **Identity for CI.** In Microsoft Entra ID, *App registrations → New
   registration* (for example `is-alive-github-signing`); under *Certificates
   & secrets*, create a client secret (note its expiry, at most 24 months).
   Give the app **Artifact Signing Certificate Profile Signer** on the
   certificate profile (or the account), in *Access control (IAM)* or:

   ```sh
   az role assignment create --assignee <app's client id> \
     --role "Artifact Signing Certificate Profile Signer" \
     --scope "/subscriptions/<sub>/resourceGroups/<rg>/providers/Microsoft.CodeSigning/codeSigningAccounts/<account>/certificateProfiles/<profile>"
   ```
6. **GitHub.** *Settings → Secrets and variables → Actions*, or with `gh`:

   ```sh
   gh secret set AZURE_TENANT_ID       # Directory (tenant) ID
   gh secret set AZURE_CLIENT_ID       # Application (client) ID
   gh secret set AZURE_CLIENT_SECRET   # the secret's Value
   gh variable set AZURE_SIGNING_ENDPOINT --body https://weu.codesigning.azure.net/
   gh variable set AZURE_SIGNING_ACCOUNT  --body <account name>
   gh variable set AZURE_SIGNING_PROFILE  --body <certificate profile name>
   gh variable set SIGNING --body azure   # last: this switches it on
   ```

   With `SIGNING=azure` and anything missing, the build fails and names it.
7. **Check.** Run the workflow (*Actions → build → Run workflow*). The
   **Signature valid** step prints the signer and timestamp. In the
   downloaded artifact, *Properties → Digital Signatures* shows the same.

To switch it off, delete the `SIGNING` variable. The Basic plan's 5,000
signatures a month cover a signature per push many times over. Signing uses
SHA-256 and Microsoft's RFC 3161 timestamp server, which keeps signatures
valid after the short-lived (a few days) Artifact Signing certificates
expire.

The step uses a client secret because it is the simplest setup. A federated
credential (OpenID Connect, no stored secret) also works with the same
action; it needs `id-token: write` on the windows job and an `azure/login`
step, so it isn't wired in.

## Internal deployment: signing your own copy

The license allows a company to use and change is-alive for its internal
operations. A company that deploys it to its own machines can sign it with
its own certificate and push trust in that certificate to those machines.
Then Windows names the company as the publisher there. This does nothing on
machines outside the company.

1. Get a code-signing certificate from the company CA (AD CS *Code Signing*
   template) into the signer's certificate store, or as a `.pfx`.
2. Sign (Windows PowerShell 5.1 or PowerShell 7):

   ```powershell
   .\scripts\sign-windows.ps1 -Path .\is-alive.exe -Subject "Contoso"            # from the store
   .\scripts\sign-windows.ps1 -Path .\is-alive.exe -PfxPath .\codesign.pfx       # from a .pfx
   ```

   It signs with SHA-256 and a DigiCert timestamp (`-TimestampServer` to
   change it, `-NoTimestamp` when offline), then prints the signer and
   whether this machine trusts it.
3. Push trust to the managed machines:
   - **Group Policy:** *Computer Configuration → Policies → Windows Settings →
     Security Settings → Public Key Policies*: the root certificate under
     **Trusted Root Certification Authorities** (already there for an AD CS
     root), the signing certificate under **Trusted Publishers**.
   - **Intune:** a *Trusted certificate* profile for the root, and a custom
     OMA-URI profile for Trusted Publishers
     (`./Device/Vendor/MSFT/RootCATrustedCertificates/TrustedPublisher/<thumbprint>/EncodedCertificate`,
     the `.cer` as Base64).

SmartScreen still sees a file it has no reputation for. Copies served from an
intranet location aren't checked by SmartScreen, and IT can submit the file
at the [Microsoft Security Intelligence portal](https://www.microsoft.com/wdsi/filesubmission).

## Trying it in a Windows 11 VM

This shows the difference a trusted signature makes, without buying anything.
Run PowerShell **as administrator** in the VM, in the folder holding
`is-alive.exe` and `sign-windows.ps1`:

```powershell
Set-ExecutionPolicy -Scope Process Bypass
.\sign-windows.ps1 -Path .\is-alive.exe -CreateTestCert -TrustOnThisMachine
# Mark it as downloaded, so Windows prompts as it would for a real download
# (a file copied in from a shared folder doesn't carry the mark):
Set-Content .\is-alive.exe -Stream Zone.Identifier -Value "[ZoneTransfer]`r`nZoneId=3"
```

This creates a certificate named *is-alive test signing (self-signed)*,
exports its public part to `is-alive-test-signing.cer`, adds it to this
machine's **Trusted Root Certification Authorities** and **Trusted
Publishers**, and signs the file. Now:

- *Properties → Digital Signatures* lists the signer, and *Details* says
  *"This digital signature is OK"*.
- Double-clicking, the **Open File – Security Warning** box (when Windows
  shows it) says **Publisher: is-alive test signing (self-signed)** instead
  of *Unknown Publisher*.
- SmartScreen may still show *Windows protected your PC*: it doesn't know
  this certificate, and it treats a self-signed one like none. Only a
  publicly trusted certificate plus reputation clears that.

On another VM that hasn't imported the `.cer`, the same file still shows
*Unknown Publisher*. To import it there by hand: `certlm.msc` → *Trusted Root
Certification Authorities → Certificates* → *All Tasks → Import*, then the
same under *Trusted Publishers*. Clean up with:

```powershell
.\sign-windows.ps1 -RemoveTestCert
```

## Checking a signed download

```powershell
Get-AuthenticodeSignature .\is-alive.exe | Format-List Status, SignerCertificate, TimeStamperCertificate
```

`Status` should be `Valid`. The `.sha256` files on the release page are
computed from the signed file.
