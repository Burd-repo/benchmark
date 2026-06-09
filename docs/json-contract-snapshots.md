# JSON Contract Snapshots

The fast contract suite versions sanitized JSON snapshots for:

- provider details;
- raw diagnostic data;
- provider registration payload.

Snapshots live under `crates/burd-bench/testdata/contracts/`. They are generated
from deterministic internal fixtures and compared during `cargo test`.

Before comparison, the test replaces:

- secret fields with `<redacted>`;
- filesystem paths with `<path>`;
- timestamps with `<timestamp>`;
- public keys, signatures, and report hashes with `<cryptographic-value>`.

This keeps the public JSON shape reviewable without storing machine-specific
paths, real agent state, or reusable cryptographic material.

When an intentional contract change is made, update and review the snapshots:

```powershell
.\scripts\update-contract-snapshots.ps1
git diff -- crates/burd-bench/testdata/contracts
```

Do not approve snapshot updates mechanically. Removed fields, renamed fields,
type changes, and newly exposed values require explicit review.
