# 0009: Licensed under Apache-2.0 only

- Status: accepted
- Date: 2026-10-31

## Context

Irsty started under `MIT OR Apache-2.0`, the Rust ecosystem's convention. The dual licence exists mainly so that libraries can be used by GPLv2-only projects (which can't take Apache-2.0 code) while still offering Apache-2.0's patent terms to everyone else.

Irsty is an application. Nobody links a GPLv2 program against an IRC client, and scripts in Phase 4 run inside Irsty rather than linking to it, so the MIT option buys little. What does matter for an application that expects outside contributions is a patent grant and clear terms for contributions, which MIT lacks.

## Decision

License Irsty under the Apache License, Version 2.0, only. Contributions are accepted under the same licence (Apache-2.0 section 5). There is one licence file, `LICENSE`.

The change was made while the project had a single copyright holder, so no other contributor's consent was needed.

## Consequences

- Downstream users get an explicit patent licence, with patent retaliation.
- Irsty can't be combined into GPLv2-only works. GPL-3.0 projects, distributions and forks are unaffected.
- Dependency licences are unaffected; `deny.toml` still governs which licences our dependencies may use.
