# License

CacheWitness is licensed under the [Apache License 2.0](https://github.com/AsyncNomad/CacheWitness/blob/main/LICENSE). This covers the runner crates (`cawit`, `cawit-verify`, `cawit-resp`), the Node hook (`cawit-hook`), the documentation, and the lab scripts.

**Exceptions.** The lab patches that modify other projects are licensed under those projects' terms, because they contain excerpts of their code and modify it:

| File | Target | License |
|---|---|---|
| `lab/twenty/refill-delay.patch` | Twenty | GNU AGPL v3.0 |
| `lab/directus/cawit.patch` | Directus | Monospace Sustainable Core License 1.0 |

They are not part of any published package. Details: [`THIRD_PARTY_NOTICES.md`](https://github.com/AsyncNomad/CacheWitness/blob/main/THIRD_PARTY_NOTICES.md).

**Trademarks.** Twenty is a trademark of Twenty.com, PBC. Directus is a trademark of Monospace Inc. They are named only to identify the software tested; CacheWitness is not affiliated with or endorsed by them.

**Disclaimer.** The software is provided "as is", without warranty of any kind. The sync hook and scenario runner are for test environments only.
