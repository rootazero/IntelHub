# SDD ledger — plan: docs/superpowers/plans/2026-09-17-globe-p1.md

## Pre-flight scan (2026-09-17)

Shared files / interfaces table:
- T2→T3 celestrak.rs：T2 产 parse_tle_catalog/tle_epoch_to_utc/celestrak_groups，T3 消费——一致。
- T4→T5→T6 adsb.rs：T4 产 AIRCRAFT_KEY/AdsbPoint/parse_ac_array/classify_notable/merge_aircraft/snapshot_envelope，T5/T6 消费——一致。
- T5 opensky.rs `pub(crate) HOTSPOTS`：仅被 adsb.rs 消费，opensky 行为不变。OK。
- T3/T5 monitor/mod.rs registry：相邻插入、顺序执行、无重叠。OK。
- T2/T4 sources/mod.rs：不同 mod 行。OK。
- T6 api.rs：Query/Deserialize/json/hub_err/redis 已确认现存于文件；AIRCRAFT_KEY 路径 pub。OK。
- T8→T9 basemap.ts：applyBasemap/globeBase 签名匹配。OK。
- T9→T10→T11 Globe.tsx：T10 定义 GlobePick 供 T11 用；T10 HUD 按计划替换 T9 的 div。OK。
- T10/T11→T13 probe：data-probe 属性（aircraft-count/sat-count）定义与消费一致。OK。
- T12→T8：build-console.sh 注入 VITE_CESIUM_ION_KEY/VITE_GOOGLE_MAPS_KEY 与 basemap.ts 读取一致。OK。
- T14→T3/T5/T6：sp6 断言对象与产出一致（health cells、redis key、satellites 表、端点）。OK。
- 自一致性：T2/T4 测试与实现匹配（37000ft=11277.6m 容差 0.5 ✓）；TS `satisfies` 语法 OK（TS 5.8）。
- Global constraints：T15 用 Debian-test 别名 + AGENTS.md exclude 清单 ✓；T3/T5 VM 上游探测 ✓。

Findings & rulings:
- Ruling: satellite.js v6 若未内置类型，允许加 `@types/satellite.js` devDep 并放宽类型标注 —— 防 tsc 阻塞 —— 若错，仅多一个 devDep。
- Ruling: vite-plugin-cesium@^1.2.23 为老包，若与 vite 6 不兼容，回退 = 手写 CESIUM_BASE_URL define + 静态资产拷贝 —— 防构建阻塞 —— 若错，多一次构建迭代。
- Ruling: T2 default_groups_match_spec 测试改环境变量属 plan-mandated，接受（当前无并发污染） —— 若错，未来需串行化该测试。
- Ruling: T15 在 315 验收全绿后暂停，merge→410→push 前必须问用户（side-effect 停止条件） —— 成本：一次人工确认。

Task 1: minor (deferred): 无 DOWN 注释（0017 有，计划明确只增不改，reviewer 认为更合规）；部分老迁移用 IF NOT EXISTS 风格差异（brief verbatim 优先）
Task 1: ⚠️ resolved by controller: DDL 首执行为 315 验收环境，属计划内验证点，非缺口
Task 1: complete (commits 0e66125..788c91e, review clean)

Task 2: minor (deferred): ①parser 两个 continue 分支无测试 ②celestrak_groups trim/去空无测试 ③tle_epoch_to_utc 无 DOY 边界（对抗输入可 panic，plan-mandated 代码，终评分流）④Alpha-5 NORAD ID 被静默丢弃（罕见）⑤FORMAT=3LE 名称行有 "0 " 前缀——本计划用 FORMAT=tle 无此前缀，T3 已提示
Task 2: complete (commits 788c91e..577cc61, review clean)
Ruling: 插入 T2.5 chore —— 修复 main 预存的 in-lib 测试目标编译破损（epa.rs E0308×2、fred.rs cpi_yoy→yoy、graph_v2 RPIT lifetime@rustc1.96、trace_propagation include_str 0009→0010），否则计划全部 cargo test 步骤被阻塞 —— 最小解锁改动，熵减原则支持 —— 若错，仅多一个 chore commit，可 revert

Task 2.5: minor (deferred): ①无 cargo test 编译回归守卫（建议后续 CI chore）②8 个预存运行期断言失败（courtlistener/etherscan/osv/urlscan/usaspending/wikidata/series，数据漂移，main 上即如此，建议单独 chore）
Task 2.5: complete (commits 577cc61..11fdb16, review clean)

Task 3: Ruling: 接受 ON CONFLICT (norad_id) DO UPDATE 偏离 —— spec§2.1 的 DELETE+INSERT/计划 plain INSERT 在跨组 NORAD 重叠（ISS∈stations+visual）下必触发 23505，implementer 在 315 真实 PG 上验证了失败与修复 —— spec/plan 文档滞后，终评前同步修订 —— 成本：ISS 分类为 last-group-wins(visual)，语义可接受

Task 3: minor (deferred): ①DB 错误中断后续组（plan-verbatim，自愈一轮）②DELETE+last-wins 单 tick 瞬态丢星（schema 固有）③分类 order-dependent（ISS→visual 非 stations，UI 过滤需注意）④epoch 兜底 Utc::now 会掩盖腐坏数据（建议加 warn!）⑤norad_id as i32 截断转型（parser 已限 5 位）
Task 3: complete (commits 11fdb16..0fb4b95, review clean)

Task 4: minor (deferred): ①merge 跨源 seen 平局取先（可忽略）②alt_baro 缺失≡ground 混淆（plan-mandated）③snapshot_envelope 非纯（now()，无害）④mod.rs 多 2 行注释（inert）
Task 4: complete (commits 0fb4b95..77ce2c8, review clean)
Ruling: 允许 T5 在 adsb.rs 内做两处小加固 —— ①merge_aircraft 输出按 hex 排序（消除 envelope 顺序随机，防 sp6/REST 断言 flaky）②补 2 个 classify 测试（7500/7600→priority 分支 + mil external_id 格式）—— reviewer 建议、同文件、低风险 —— 若错，仅多几行测试与一次排序

Task 5: Ruling: 采纳 implementer 选项 (a) 重设计 adsb fetch —— 实测 adsb.lol 可持续配额 ~2-5 req/min（容量 4-8，回填 1 token/20-30s），计划 44 req/min 超配 10×，且 scheduler 退避永不触发（regions_ok>0 即 Ok）+ TTL60s 会致快照抖动消失 + IP 声誉风险 —— 新设计：每 tick(15s) 单请求轮转 [mil, squawk7700, squawk7500, squawk7600, point×10]（全周期 14 tick=3.5min ≈4 req/min）；单请求失败即 Err（退避恢复有效）；快照改累积式（Mutex<HashMap<hex,(AdsbPoint,seen_abs)>>，每 tick 重写全量 envelope，TTL 60→300s，>10min 未见剔除）；squawk 通道升级全球覆盖（优于 spec 的热点内检测，spec 意图达成且更好）；spec§2.2/§3.1 + plan T5 文档随后同步 —— 成本：单区域位置新鲜度 15s→3.5min，前端外推需 cap

Task 5: fix round 1/5 (rate-limit redesign per controller ruling; commits 5845e9e..d66a936)
Task 5: minor (deferred): ①Mutex poison 无恢复（建议 unwrap_or_else(into_inner) 一行）②快照无 MAX cap（稳态 <2k 架无风险）③失败不动测试偏弱（结构性保证，可接受）④CYCLE_SECS 硬编码未与 QUEUE_LEN×15 联动⑤merge_aircraft 无产出调用方（pub 保留）
Task 5: complete (commits 77ce2c8..d66a936, review clean)
Ruling: envelope 契约变更需带进下游任务 —— T10 前端：AircraftSnapshot 移除 regions_ok、新增 cycle_secs/last_tick/age_s，HUD 改显 last_tick/age，dead-reckoning 外推 cap 240s；T14 sp6：regions_ok 细节串改为 last_tick —— 成本：T10/T14 dispatch 需带此修订

Task 6: minor (deferred): ①?category= 空串→0 行（P1 无调用方触发，终评决定是否加 .filter 加固）②api.rs 注释 ">60s" 滞后（实际 TTL 300s，建议终评 fix wave 改一行）③降级 body 无 count/ts——T10 dispatch 需提示 stale 是唯一可靠判别字段
Task 6: complete (commits 063d3ad..ef00060, review clean)

Task 7: Ruling: T9 dispatch 必须带 4 项 —— ①vite.config.ts 改 cesium({ rebuildCesium: true })（默认模式外链 6.1MB render-blocking 脚本进每个路由，破坏 React.lazy 分包目标）②Viewer 构造传 baseLayer: false（无 Ion token 时默认 Ion 底图会 401）③CSS 用 cesium/Build/Cesium/Widgets/widgets.css（与计划一致，注意 rebuild 模式下是否与插件注入重复，验证一次）④serve_static 对 /cesium/* 的 MIME/fallback 行为留作 T15 VM spot-check —— 成本：T9 多一次构建验证
Task 7: minor (deferred): maplibre-gl npm audit critical（GHSA-jrc7-96c5-q579，修复需 6.10.0 breaking 升级，独立 chore）

Task 7: minor (deferred): ①cesium 实际解析 1.145.0（^1.124 内，@cesium/engine 重构版，T9 注意 API 面）②engines.node>=22（镜像 node:22 满足，README 可补本地下限）③tsc -b 不检查 vite.config.ts（证据口径，无实际影响）
Task 7: complete (commits 511d625..f34a8b3, review clean)

Task 8: minor (deferred): ①removeAll() 在 await 前有瞬态无影像窗口（不持久，brief-verbatim，T9 若首屏白球可改 fetch-then-swap）②google 分支不动 terrainProvider（T9 构造 Viewer 时不得用 Ion 地形，与 ruling ②一致）③失败路径双 removeAll（幂等无害）
Task 8: complete (commits f34a8b3..e1178ba, review clean)

Task 9: minor (deferred): ①ErrorBoundary 文案 Globe-only 与通用路径不匹配（后续参数化或重命名）②Retry 对确定性失败无上限（重试循环）③StrictMode 双挂载未排除（cleanup 正确，无害）
Task 9: complete (commits e1178ba..48ab207, review clean)

Task 10: minor (deferred): ①双 remove 冗余（no-op 无害）②15s 重建闪烁（联调若可见可改 in-place 更新）③count 与 aircraft.length 可能不一致（server 契约层面）
Task 10: complete (commits 48ab207..a7cfed9, review clean)

Task 11: minor (deferred): ①propagate 异常未捕获（理论风险，可套 try/catch）②satCollection ref 只写不读（dead ref）③500 点/秒重建 GC（已知取舍）④首拍 1s 延迟（brief 原样）
Task 11: complete (commits a7cfed9..2e0e009, review clean)

Task 12: minor (deferred): ①ABORT 文案指向 console-build.env 与新 key 实际来源 secrets.env 不完全吻合（预存文案）②manifest 无读者（纯 additive 安全）③新 key 缺失静默（manifest 记 false，设计一致）
Task 12: complete (commits 2e0e009..729e863, review clean)
Note: T15 验收项补充 —— VM 构建后确认 .build-manifest.json 含 cesium_ion/google_maps 字段且 verify ABORT 未触发；清理未跟踪的 console/probe-globe-smoke.mjs（T9-T11 smoke 脚手架，防误提交）

Task 13: parked — Important: 无 try/finally 浏览器关闭（waitForSelector 超时会跳过 browser.close）—— Ruling: plan-mandated 冻结脚本文本，probe 为 T15 手动工具，Playwright 进程退出 hook 兜底收割，修复收益边际 —— 成本：极端情况下残留一个 chromium 进程到脚本退出
Task 13: minor (deferred): ①失败路径双 textContent 各等 30s（有界）②parseInt 宽松（当前输出为纯整数）③9s 固定 sleep（契约允许 0/stale）
Task 13: complete (commits 729e863..c520b2f, review clean, 1 parked)
Note: T15 dispatch 前置 —— npx playwright install chromium；probe 失败路径最长 ~69s

Task 14: minor (deferred): ①valid-JSON-non-dict 逃逸防御（缺 isinstance 检查，部署瞬态陈旧 blob 可能触发 traceback——终评决定是否加固一行）②check4 名称低估断言面 ③check2 "fresh" 实为 TTL 活性 ④风格漂移（st_a/body_a vs code,name 约定；未编号 §12）
Task 14: complete (commits c520b2f..5a999d4, review clean)
Note: sp6 实际基线为 23（AGENTS.md 写 18 系预存漂移），T15 验收预期 = 预存绿基线 + 4 且 0 failed，T15 后更新 AGENTS.md/记忆

Task 15 (steps 1-3): minor (deferred): ①sp8 TSV 模式 PG parity 检查静默跳过（应 shelve 一行）②TSV/legacy 分流靠 "===" 子串（极低误判率）③sp7 secret() 只读 secrets.env ④sp3 urlparse 无 scheme 会崩（文档用法均带 scheme）⑤315 hub.env HUB_MCP_ALLOWED_HOSTS 改动在 VM 未入库（环境配置）
Task 15 (steps 1-3): complete (315 部署+验收全绿: sp6 25+2sh/0f, sp8 39+2sh/0f, sp7 16+11sh/0f, sp3 19/0, probe OK aircraft=316 sats=440; fix commit 5a999d4..294e030 review clean)
Task 15 (steps 4-6): PENDING USER GATE — merge/410/push 前必须用户确认

Final review: With fixes → fix wave commit 3b946fd (F1-F11) → re-review all addressed, no new breakage
Final 315 re-verify (fix wave deployed): sp6 25 passed + 2 shelved + 0 failed (globe 4/4 PASS), probe-globe OK (canvas=true aircraft=292 satellites=440 pageerrors=0, 收窄后的过滤正则无误伤)
Ruling: 终评 Important#2 不改代码 —— 410 实测 FRED/EIA/COMTRADE/FINNHUB 同样空值（预存状态，非本分支引入），shelved 门控如实反映，merge gate 报告用户
All tasks complete. Awaiting user gate for T15 steps 4-6 (merge → 410 → push).
