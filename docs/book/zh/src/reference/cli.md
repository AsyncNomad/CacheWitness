# 命令行

```text
cawit verify <SCENARIO> [OPTIONS]
```

运行一个场景并判定每一轮。

| 选项 | 含义 |
|---|---|
| `<SCENARIO>` | 场景 YAML 文件路径 |
| `--runs <N>` | 轮数；覆盖场景中的 `runs` |
| `--out <PATH>` | 以 JSON 写出完整报告（[报告格式](report-format.md#json)） |
| `--junit <PATH>` | 写出 JUnit XML 报告，每一轮一个测试用例（[报告格式](report-format.md#junit)） |
| `--show-steps` | 以 YAML 打印场景将要执行的步骤（模式会被展开），然后不连接任何服务直接以 `0` 退出 |
| `-h`、`--help` | 帮助 |

`cawit --version` 打印版本号。

## 输出

每一轮的结果和每个判定，然后是汇总：

```text
run   1  PASS
          ok   not_stale (app_equals_db): served=…-1-b (cache miss) db=…-1-b
          ok   refilled (app_cached_equals_db): served=…-1-b (cache hit) db=…-1-b
run   2  NOT_REACHED
          sync point `refill:after-read` not reached within 10000ms

directus-stale-set: 2 runs  pass 1  fail 0  not_reached 1  error 0
  not_stale            passed 1/1
  refilled             passed 1/1
```

按判定的合计只统计完成了判定的轮次（`PASS` 或 `FAIL`）。

## 退出码

| 退出码 | 含义 |
|---|---|
| `0` | 所有轮次通过 |
| `1` | 至少一轮失败 |
| `2` | 没有失败，但有轮次被丢弃（`NOT_REACHED`、`ERROR`）或没有任何轮次；使场景无法运行的错误（文件无效、无法连接 PostgreSQL 或 Redis）也会打印 `cawit: <错误>` 并返回 `2` |
