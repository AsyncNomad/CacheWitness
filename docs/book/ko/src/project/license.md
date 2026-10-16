# 라이선스

CacheWitness는 [Apache License 2.0](https://github.com/AsyncNomad/CacheWitness/blob/main/LICENSE)을 따릅니다. 실행기 crate(`cawit`, `cawit-verify`, `cawit-resp`), Node 훅(`cawit-hook`), 문서, 실험 스크립트가 여기에 해당합니다.

**예외.** 다른 프로젝트를 수정하는 실험 패치는 그 프로젝트의 코드 일부를 담고 그것을 수정하므로, 그 프로젝트의 라이선스를 따릅니다.

| 파일 | 대상 | 라이선스 |
|---|---|---|
| `lab/twenty/refill-delay.patch` | Twenty | GNU AGPL v3.0 |
| `lab/directus/cawit.patch` | Directus | Monospace Sustainable Core License 1.0 |

이 파일들은 어떤 게시 패키지에도 들어가지 않습니다. 자세한 내용: [`THIRD_PARTY_NOTICES.md`](https://github.com/AsyncNomad/CacheWitness/blob/main/THIRD_PARTY_NOTICES.md).

**상표.** Twenty는 Twenty.com, PBC의 상표이고, Directus는 Monospace Inc.의 상표입니다. 시험한 소프트웨어를 가리키기 위해서만 이름을 쓰며, CacheWitness는 이들과 제휴하거나 보증받지 않았습니다.

**면책.** 본 소프트웨어는 어떠한 보증도 없이 "있는 그대로" 제공됩니다. 동기화 훅과 시나리오 실행기는 테스트 환경 전용입니다.
