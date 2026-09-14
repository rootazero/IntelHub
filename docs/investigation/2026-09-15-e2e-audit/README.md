# 2026-09-15 E2E Investigation Audit

> **Status:** 🚧 In progress · **Target:** VM 415 (IntelHub-test, 10.10.10.45) · **Date:** 2026-09-15

## 背景

通过真实 e2e 测试发现 IntelHub 的缺陷与不足，产出 findings 文档 + 修复 plans。
仅在测试 VM 415 上做，不动生产 VM 410。

## 方法论

三条调查线并发，每条独立工作后汇总到 `FINDINGS.md`：

| 线 | 负责面 | 方法 | 主结果文件 |
|---|---|---|---|
| **A** | MCP/REST API 合同不一致 | 拉 41 tool schema → 各跑 happy path → 对比 console `.ts` 期望 | `thread-A-contract.md` |
| **B** | PG/Neo4j/Redis 跨库一致性 | 抽样 claim/document/investigation 三方对照 diff | `thread-B-consistency.md` |
| **C** | Console UI 渲染失败 | headless chromium + pageerror 监听，覆盖各页 + 主题 + 深链接 | `thread-C-ui.md` |

## 调查深度

合同 + 对抗 + 一致性（3 维度并发）

## 跳过清单

- ❌ 所有 sp2a/b/3/4/5/6/7/8/9/10 验收脚本已覆盖的基线检查项
- ❌ Climate series 路径（刚 sp8 验过）
- ✅ 全部测：ACLED/reliefweb shelved 的 graceful-degradation 路径
- ✅ 全部测：financials_fetch
- ✅ 全部测：admin tool (run_component_action) 的 X-Admin-Token DENY 路径

## 已知环境约束

- 410 和 415 的 `secrets.env` 全部 15 个数据源 key 都是 EMPTY（shelved-by-design，**不是配置缺失**）
- `hub.env` 415 比 410 多 `HUB_MCP_ALLOWED_HOSTS`（新 config）
- `compose.env` 415 比 410 多 14 个 version pin + HUGINN 配置
- 这是 by-design，不能简单复制 410 的 env 到 415 来"启用"数据源

## 目录结构

```
docs/investigation/2026-09-15-e2e-audit/
├── README.md            # 本文件
├── FINDINGS.md          # 主报告：bug 列表 + 严重度
├── thread-A-contract.md # 线 A 详细日志
├── thread-B-consistency.md # 线 B diff 结果
├── thread-C-ui.md       # 线 C pageerror 时间线
└── evidence/            # 截图、原始 JSON、db dump
    ├── thread-A/
    ├── thread-B/
    └── thread-C/

docs/superpowers/plans/  # 每个 🔴/🟠 bug 一份 fix plan
├── 2026-09-15-<bug>-fix.md
└── ...
```

## Timeline

| 阶段 | 状态 | 完成时间 |
|---|---|---|
| 0. 准备（worktree + README） | ✅ | 2026-09-15 20:08 |
| 1. 三线并发（A/B/C） | 🚧 进行中 | — |
| 2. 汇总 FINDINGS.md | ⏳ 待 | — |
| 3. 写 fix plans | ⏳ 待 | — |
| 4. 收尾 commit | ⏳ 待 | — |

## 索引（汇总后填）

- 🔴 Critical:
- 🟠 High:
- 🟡 Medium:
- 🟢 Low:
