# 许可证

CacheWitness 基于 [Apache License 2.0](https://github.com/AsyncNomad/CacheWitness/blob/main/LICENSE) 发布，涵盖执行器 crate（`cawit`、`cawit-verify`、`cawit-resp`）、Node 钩子（`cawit-hook`）、文档和实验脚本。

**例外。** 修改其他项目的实验补丁包含这些项目的代码片段并对其进行修改，因此遵循这些项目的许可证：

| 文件 | 目标 | 许可证 |
|---|---|---|
| `lab/twenty/refill-delay.patch` | Twenty | GNU AGPL v3.0 |
| `lab/directus/cawit.patch` | Directus | Monospace Sustainable Core License 1.0 |

它们不属于任何已发布的软件包。详情见 [`THIRD_PARTY_NOTICES.md`](https://github.com/AsyncNomad/CacheWitness/blob/main/THIRD_PARTY_NOTICES.md)。

**商标。** Twenty 是 Twenty.com, PBC 的商标。Directus 是 Monospace Inc. 的商标。这些名称仅用于指明被测试的软件；CacheWitness 与它们没有关联，也未获其认可。

**免责声明。** 本软件按"原样"提供，不附带任何形式的担保。同步钩子和场景执行器仅用于测试环境。
