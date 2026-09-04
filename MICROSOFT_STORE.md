# Microsoft Store submission

Be Asset Optimizer is submitted as an MSIX desktop app. Microsoft Store signs the package after certification, so the Store submission does not require a CA-issued code-signing certificate.

## Partner Center identity

| Field | Value |
| --- | --- |
| Package/Identity/Name | `StudioJuh.BeAssetOptimizer` |
| Package/Identity/Publisher | `CN=32199432-A020-4365-AE9B-74806B8E24F8` |
| Package/Properties/PublisherDisplayName | `StudioJuh` |

These values are case-sensitive and must remain identical to Partner Center.

## Build the MSIX

From the repository root on Windows:

```powershell
npm run package:store
```

The package is written to:

```text
src-tauri/target/store/msix/output/
```

The script performs a release build, prepares and verifies the open-source AI components, stages the Store assets and notices, creates the MSIX with the Windows SDK `MakeAppx.exe`, then unpacks it again to verify the Partner Center identity.

The Store build bundles the Real-ESRGAN executable and models so all executable code is covered by the Store package and AI restoration works without downloading executable code after installation. BOOTH and portable builds retain the smaller on-demand component behavior.

## Version mapping

Microsoft Store package versions must use four numeric fields, the first field cannot be zero, and the fourth field is reserved for the Store. The build script maps the application version by adding one to its major version:

```text
App 0.6.1 -> MSIX 1.6.1.0
App 1.0.0 -> MSIX 2.0.0.0
```

This keeps Store versions increasing across the application's `0.x` to `1.x` transition.

## Submission declarations

- Architecture: x64
- Device family: Windows Desktop
- Minimum Windows version: Windows 10, version 2004 (`10.0.19041.0`)
- Language: Japanese (`ja-jp`)
- Restricted capability: `runFullTrust`
- Privacy policy: <https://github.com/studio-juh/be-asset-optimizer/blob/master/PRIVACY.md>
- Support: <https://github.com/studio-juh/be-asset-optimizer/issues>

`runFullTrust` is required because Be Asset Optimizer is a Tauri/Win32 desktop application that processes files selected by the user, invokes the bundled local Real-ESRGAN executable for optional AI restoration, and opens generated files in Windows Explorer. It does not request elevation, install services or drivers, modify system settings, or upload user files.

## Before submission

1. Run `npm run package:store` on a clean release checkout.
2. Test the package on a separate Windows user or virtual machine using a development certificate whose subject exactly matches the Partner Center publisher.
3. Run the Windows App Certification Kit.
4. Upload the resulting `.msix` directly in the Partner Center package section.
5. Declare and justify the `runFullTrust` restricted capability in the submission options.

Do not upload the NSIS `.exe`, WiX `.msi`, or portable ZIP to the MSIX product.
