# #380 D1-A IPC v3 mixed-version contract

**Disposition: `BLOCKED — OWNER DESIGN ESCALATION` for mixed-version rollout.**

This note records the behavior of the D1-A protocol changes and the current
installer. It does not authorize IPC v4, installer changes, Schema 38, or D1-B/C.
The source and state-machine fixes in PR #386 remain a Draft for review; this
contract is not evidence that #380 is fully fixed.

## Protocol behavior proved by source

The current protocol is still IPC v3 on `\\.\pipe\ZenCanvas.GlobalIndex.v3`.
`IndexServiceRequest` carries a protocol version, request ID, command, and
optional source snapshot. It carries no client build ID or required capability
set. The new client requires a first `Sources` Admission event before it applies
any operation event. A successful index response also requires SourceProvider
and Checkpoint, plus VolumeEntriesStale for initial indexing and Rebuild.

At `origin/master` (`3275ac7c72df8ae4139a30f16c7f649971324e97`), the v3 Service
claims its busy slot, takes the operation lock, validates the source, then calls
the provider without sending `Sources` Admission. The production provider's
first event for a supported source is SourceProvider. The old event shape for
`EntryStale` has only `entry_id`; D1-A adds `volume_id` while retaining protocol
version 3. The old desktop's event handler treats `Sources` as a no-op.

### New desktop with old v3 Service

For a supported source, the new client rejects the old Service's first
SourceProvider event as “missing Admission” before invoking the stateful Sink.
It does not Direct-fallback. D1-A classifies this exact first-event failure as
`ServiceAdmissionMissing`; the Coordinator preserves the durable source status
and completion timestamps. The old Service receives no successful event
application from the desktop and its PipeSink writes only by sending events to
that desktop. This prevents the old peer from publishing index data through
this rejected stream.

Other post-dispatch disconnects and malformed streams remain
`ServiceOutcomeUnknown`; they are not converted to the safe missing-admission
case and may record source ERROR. A busy response before Admission remains a
typed ServiceBusy rejection and does not fall back.

### Old desktop with new Service

This pairing is **not supported or safely rejected by IPC v3**. The new Service
and old Service share the same `protocol_version = 3`, and the new Service has
no field by which it can identify an old desktop. The old client ignores the
new Admission event. Its old Coordinator persists INDEXING/SYNCING before
calling the Service, and its old error decoder maps unknown error codes such as
`index_service_busy` to Provider error. A busy rejection can therefore leave
the old client recording ERROR. The old source code appears able to deserialize
the additive `volume_id` field under Serde's default unknown-field behavior,
but no old-binary/new-Service execution test was run; parsing compatibility
would not fix the state semantics in any case.

The new Service currently validates that the named-pipe client executable path
equals its own executable path. This rejects clients launched from a different
path; it does not attest the binary generation when both processes resolve to
the same installed path. IPC v3 itself cannot reject an old client by version
or capability.

## Installer transition evidence

The current NSIS install lifecycle establishes a useful stop-the-world order:

1. `ZCPrepareInstallLifecycle` stops and waits for the exact-owned Service.
2. It closes the desktop process before canonical file mutation.
3. After generated files are installed, `ZCEnsurePostInstallServiceFinal`
   converges the exact-owned Service to stable RUNNING or creates it from the
   current main executable.

If cancellation/failure occurs before file mutation, reversible recovery
restores the captured Service state. Once file mutation starts, the installer
does not restore old binaries: partial-failure recovery leaves a captured
repair Service stopped, or compensates a newly created Service. A coherent
post-generated integration failure may restore the captured Service state.
The Service ImagePath points to the installed main executable, so a successful
upgrade normally starts the new generation from the same image as the new
desktop.

This is source evidence for the installer path, not a proven mixed-version
guarantee. The installer does not add a protocol capability gate or a verified
application launch barrier spanning file replacement through Service restart.
Real SCM restart/rollback behavior, an old binary launched from the canonical
path, and an application launch during the stopped-Service interval have not
been exercised in this PR.

## Required operating contract and escalation

Until Owner approves a versioned compatibility boundary, only a homogeneous
desktop/Service generation may be treated as supported. Installation must stop
the exact-owned Service and all desktop processes before replacing the shared
executable; success starts the Service from the installed generation. A
pre-mutation rollback restores the old pair. A post-mutation failure must keep
the Service stopped and the product unavailable until a coherent reinstall or
repair completes. No mixed-version pair may perform indexing or Direct
fallback during the transition.

Current v3 code cannot enforce every rule above: it can fail closed for a new
client talking to an old Service and preserve health for that exact
missing-Admission case, but a new Service cannot distinguish or reject an old
client. Owner must choose the next boundary before rollout:

- approve a versioned capability/handshake and Service-side rejection policy,
  with a tested installer gate and rollback path; or
- approve a stricter stop-the-world installation/startup gate that makes mixed
  clients impossible and is tested across cancellation, service restart, and
  post-mutation failure.

The minimum protocol proposal should carry an explicit client generation and
required event-contract capability, reject unsupported clients before the
Service claims the operation, and define how an old client handles rejection.
Whether that is a new IPC version or an explicitly versioned extension is an
Owner decision. D1-A does not choose or implement it. Until the decision and
runtime/installer evidence exist, mixed-version rollout remains blocked.
