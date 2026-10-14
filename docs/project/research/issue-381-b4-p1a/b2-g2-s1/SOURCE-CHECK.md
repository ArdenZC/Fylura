# G2-S1 source crosscheck record

## Microsoft PowerShell reference check

The official Microsoft Learn PowerShell reference sources were read from Microsoft's public documentation repository `MicrosoftDocs/windows-powershell-docs`, default branch `main` at commit `c3934e67de374f52fa6759f916c112c30a1240c7` (tree `04c12b11080864758fb41d6a802146e248cbcfb5`, commit date 2026-09-23). This was a read-only GitHub API fetch; no PowerShell or storage command was run.

| Learn reference | Repository source at checked commit | Source facts checked | Static-model consequence |
| --- | --- | --- | --- |
| [New-VHD](https://learn.microsoft.com/en-us/powershell/module/hyper-v/new-vhd) | `docset/winserver2025-ps/Hyper-V/New-VHD.md`, blob `cec55341c64b00776165db8b0b5df400d21aef50` | The fixed-without-source parameter set has `-Path`, `-SizeBytes`, and `-Fixed`; the extension selects VHD versus VHDX. The separate source-disk parameter sets demonstrate why source import must stay prohibited. | Model accepts only exact new `.vhdx` path, `fixed` allocation, explicit cap, and no source import. |
| [Mount-VHD](https://learn.microsoft.com/en-us/powershell/module/hyper-v/mount-vhd) | `docset/winserver2025-ps/Hyper-V/Mount-VHD.md`, blob `5b658ab19b734a1c0e24d61f134ede0a7892f53c` | `-NoDriveLetter` is available. `-PassThru` returns an object representing the mounted VHD. | Model requires a returned VHD object identity, no drive letter, and one new disk bound to that object. |
| [Initialize-Disk](https://learn.microsoft.com/en-us/powershell/module/storage/initialize-disk) | `docset/winserver2025-ps/Storage/Initialize-Disk.md`, blob `02668e386dd16e7ffb0d91ce994301aa0f9762d9` | An `-InputObject` CIM-instance parameter set exists; number, friendly-name, path, and unique-ID parameter sets also exist. The examples show why choosing a convenient number is not identity proof. | Model targets only the exact validated returned-disk identity; number/name/default selectors are rejected. A real adapter would still need to prove the CIM object's identity before any separately authorized execution. |
| [Format-Volume](https://learn.microsoft.com/en-us/powershell/module/storage/format-volume) | `docset/winserver2025-ps/Storage/Format-Volume.md`, blob `f901378c9669265bb47c3597e2a02e8c6810fff0` | A `-Partition` CIM-instance parameter set exists, alongside broader volume selectors and `-Force`. | Model binds formatting to the exact returned partition identity and NTFS; no volume selector or force behavior is emitted. |

These are command-surface facts, not proof that the current hosted Windows runner has Hyper-V/storage providers, permissions, or safe cleanup semantics. The checked reference snapshot is pinned above so a future owner can compare any later documentation drift.

## Apple `hdiutil` reference crosscheck

The Owner contract comment [#414 comment 6169345320](https://github.com/ArdenZC/Zen-Canvas/issues/414#issuecomment-6169345320) cites [the `hdiutil` manual](https://manp.gs/mac/1/hdiutil). For this static source check, the command text was cross-read from the read-only public GitHub mirror [`keith/xcode-man-pages`, `docs/hdiutil.1.html`](https://github.com/keith/xcode-man-pages/blob/46d6bd5d4488143b1928411eb2bbaa262e712e0a/docs/hdiutil.1.html), commit `46d6bd5d4488143b1928411eb2bbaa262e712e0a`, tree `b3c5704c306ff3ab2c85af09cef31522d4b516ce`, dated 2026-09-28; the mirror labels the snapshot “Xcode 27.2 beta 2”. This is a third-party mirror of a command manual, **not a canonical Apple documentation endpoint**.

The mirrored blob is `af19d1aacce60c4784f99d814c344ffa7c3988a9`. It states that `-plist` provides plist result output intended for other programs; `create -type UDIF` creates UDRW; SPARSE/SPARSEBUNDLE expand as written; `-fs` supports APFS; `-mountpoint` can specify a mount path; and detach accepts a device name and has a separate `-force` option. The manual also says `create` fails for an existing image unless overwrite is explicitly requested, and its current snapshot marks several `hdiutil` verbs deprecated in favor of `diskutil image` (with ASIF available only through that newer interface). The model uses none of the deprecated/newer ASIF path, requires absent target and structured result, and never emits force/global-disk intents.

Before any separately authorized G2-S2 proposal, Owner review must compare this mirror with the current [Owner-cited `hdiutil` manual](https://manp.gs/mac/1/hdiutil) and verify the exact structured attach output, APFS creation and detach contract on the selected macOS version. The mirrored text is source crosscheck for the stated model assumptions, not certification of macOS 26 or any runner behavior. No macOS command was executed.

## Crosscheck field semantics

The fixtures set `inventoryEvidence.crosscheck: "PASS"` only to exercise the positive branch of a synthetic model. It is not a cryptographic source receipt, a runtime fetch, a native preflight, or evidence that every source page was validated. The test separately proves `FAIL` and `MISSING` crosschecks suppress all intents. The mirror-versus-canonical Apple source and target-OS gap remains explicit above for independent review.
