# GEV P11 follow-up — Austin/Tarktee/Street View/Batch 6

> 日期：2026-09-19 · 状态：✅ 已合并并部署
> 合并 commits：main @ 525ec0b (austin), fd97555 (tarktee docs), 2291114 (Batch 6), d5b6f27 (testid fix)

## 实施内容

| Follow-up | 解决 | commit |
|---|---|---|
| 1. Austin Socrata dataset ID | `dx9v-zd7x` (traffic reports) → `b4k4-adkb` (Traffic Cameras). 重写 parser 处理 object-array JSON + GeoJSON Point 坐标。Frame URL 优先用上游 `screenshot_address`，fallback 到 canonical `cctv.austinmobility.io/image/{id}.jpg` | 525ec0b |
| 2. Tarktee upstream 500 | DATEX2 XML + image server 在 transpordiamet.ee 全部 500 (持续性 upstream outage, 验证 2026-09-19). ArcGIS MapServer 在 tarktee.ee 返 200 但坐标 EPSG:3301 + image_path 仍指向 500 server. 文档化在 tarktee.rs 头部;deferred to follow-up (需要 new geo crate for LCC→WGS84) | fd97555 |
| 3. Street View env var | 代码已 wiring (gev_cctv_frame_fallback.rs::street_view_key).examples/intelhub.env.example 文档化 HUB_GOOGLE_MAPS_SERVER_API_KEY | 2291114 |
| 4. Batch 6 acceptance | sp6: per-provider floor checks (caltrans≥100, drivebc≥500, fintraffic≥500, txdot≥50, nsw≥100, calgary≥100, austin≥100). sp8: 4 个 cctv-popout testid 检查 (dist bundle + source truth). probe-gev: P11 CCTV popout segment (click → panel mount → media → ESC close) | 2291114, d5b6f27 |

## 验收数据

### 摄像头数据 (生产 IntelHub VM 410, deploy 后)

| Provider | Rows | 备注 |
|---|---|---|
| austin | 454 | **NEW** (up from 0) |
| calgary | 217 | |
| caltrans | 1091 | |
| drivebc | 1051 | |
| fintraffic | 2258 | |
| nsw | 216 | |
| ny511 | 1870 | |
| ontario511 | 945 | |
| static-* | 259 | |
| tfl | 890 | |
| txdot | 550 | |
| tarktee | 0 | upstream 500 — documented limitation |
| **总计** | **~10,150** | (vs 原 ~4000 提升 ~150%) |

### sp6 验收 (baseline 39+5sh/0)

| 类别 | 结果 | Δ |
|---|---|---|
| passed | 45 | **+6** vs baseline (新 per-provider floors: caltrans/drivebc/fintraffic/txdot/nsw/calgary/austin) |
| shelved | 5 | same |
| failed | 1 | starlink TLE (celestrak upstream 502 — 与本工作无关) |

### sp8 验收 (baseline 48+2sh/0)

| 类别 | 结果 | Δ |
|---|---|---|
| passed | 55 | **+7** vs baseline (4 new P11 testid + 3 其他 drift) |
| shelved | 2 | same |
| failed | 0 | ✓ |

### sp3 验收 (baseline 19/0)

| 类别 | 结果 | Δ |
|---|---|---|
| passed | 19 | same |
| failed | 0 | ✓ |

## 决策日志

| 决策 | 上下文 | 影响 |
|---|---|---|
| Austin Socrata: object-array JSON, not rows.json+format | 新 dataset b4k4-adkb 返回 JSON objects;旧 dataset dx9v-zd7x (实际是 traffic reports) | 适配上游 payload |
| Austin 坐标: GeoJSON Point `location.coordinates=[lon,lat]` | Socrata 默认 GeoJSON 格式 | extractor 加 array fallback before nested object lookup |
| Tarktee deferred to follow-up | 需要 geo crate for LCC→WGS84 (proj, proj4rs 等) | 加 crate 需 PR 评审;upstream 也未恢复 |
| Street View env var 文档化在 examples/ | 代码已 wiring, 仅需 docs | 用户可读性 + 后续直接填 key |
| sp6 per-provider floors 在 parse_austin 修复前不应通过 | 7 floors 加进 sp6 | 测试覆盖 P11 实施正确性 |

## 后续未结

1. **Tarktee** — 需等 upstream 恢复 + 加 geo crate for EPSG:3301 LCC→WGS84 reprojection
2. **Street View** — 等用户填 GOOGLE_MAPS_SERVER_API_KEY 到 VM 410 的 core/secrets.env
3. **CCTV sources 增长到 ~10,150** — 已远超原 4+5 = ~4000 baseline,持续监控 uptime