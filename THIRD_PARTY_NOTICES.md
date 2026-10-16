# Third-party notices

CacheWitness is licensed under the Apache License 2.0 ([LICENSE](LICENSE)), **except for the files listed below**, which modify other projects and are licensed under those projects' terms.

## Files under other licenses

| File | Modifies | License | Copyright |
|---|---|---|---|
| [`lab/twenty/refill-delay.patch`](lab/twenty/refill-delay.patch) | [Twenty](https://github.com/twentyhq/twenty) at commit `c3ba1401f45dbc35bf5ed3d48c4ddc2e58f2bb15` | GNU Affero General Public License v3.0 ([LICENSES/AGPL-3.0.txt](LICENSES/AGPL-3.0.txt)) | Twenty.com, PBC |
| [`lab/directus/cawit.patch`](lab/directus/cawit.patch) | [Directus](https://github.com/directus/directus) v12.5.0, commit `4f77933bf9551cac6b289576078783e189b8f4e3` | Monospace Sustainable Core License 1.0 ([LICENSES/MSCL-1.0.txt](LICENSES/MSCL-1.0.txt)) | Monospace Inc. |

Each patch is a test-only modification used to reproduce cache races in a local, network-isolated lab. It contains short excerpts of the target's source code (diff context and the lines it changes) and the lines CacheWitness adds; the whole patch, including the added lines, is offered under the target's license, as stated in its header. The target source itself is not included in this repository: `lab/<target>/setup.sh` fetches the pinned upstream commit and applies the patch.

- The Twenty file the patch modifies is not marked `@license Enterprise`; the Twenty Application Exception does not apply to modifications of Twenty, so plain AGPLv3 applies. The lab runs the modified server only locally and does not offer it to other users over a network.
- The Directus patch is used for non-commercial research and testing, a Permitted Purpose under MSCL-1.0, and does not touch license key functionality.

The published packages (`cawit`, `cawit-verify`, `cawit-resp` on crates.io and `cawit-hook` on npm) contain none of these files.

When regenerating a patch, use `lab/make-patch.sh`, which keeps the license header.

## Dependencies

The Rust crates depend on third-party crates (for example `tokio`, `tokio-postgres`, `serde`, `clap`, `anyhow`) under their own licenses, mostly MIT or Apache-2.0; `cargo` fetches them, and they are not vendored here. The Node hook has no dependencies. The lab's test harness (`lab/truth/`) installs `pg` and `redis` from npm (MIT) at setup time.

## Trademarks

Twenty is a trademark of Twenty.com, PBC. Directus is a trademark of Monospace Inc. Other names are trademarks of their owners. They are used only to identify the software tested. CacheWitness is not affiliated with or endorsed by any of them.
