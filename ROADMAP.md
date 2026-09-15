# IntelHub Roadmap

> 本文件记录经过评估但**当前不做**（wontfix-by-design）的功能方向、决策理由、和对应 issue 链接。
> 任何想重启评估的人，先读这份再开新讨论。

---

## 已知不作（Wontfix-by-design）

### 1. 全球飞机/船舶实时通行系统（ADS-B / AIS）

- **决策日期**：2026-09-15
- **状态**：wontfix-by-design
- **GitHub issue**：[#1](https://github.com/rootazero/IntelHub/issues/1)
- **触发背景**：用户提出 "没有开源免费的全球飞机船舶通行实时系统 API"，经调研确认无合规免费方案

#### 核心结论

**没有任何"开源免费 + 全球覆盖 + 实时 + 可商用 + 无限制"的现成数据源。** "免费层"几乎都附带"非商用 / 研究用途 / 必须馈送数据" 等条款，对外公开的 OSINT 产品一律不适用。

#### 候选数据源对比

**飞机（ADS-B）**

| 数据源 | 许可证 | 限速 | 成本 | 实时/全球 | 可商用 | 关键条款 |
|---|---|---|---|---|---|---|
| OpenSky Network | GPL v3 客户端；数据需书面协议 | 匿名 400 req/天；OAuth 4000 req/天 | 免费 | ✅/✅ | ❌ | "Operational use"（集成到任何自动化系统即便内部）必须书面协议，非营利也走审批 |
| ADS-B Exchange Community API | 私有 ToS，非商用 | 未公开 | 免费 | ✅/✅ | ❌ | 明确锁定 hobby/research/prototype；商用需付费订阅 |
| FlightAware AeroAPI Personal | 商业 | 10 result sets/min；$5/月免费额度 | $0-5/月（**仍需信用卡**） | ✅/✅ | ✅ | 最直接的合规免费层但额度极小 |
| ADSB.lol | **ODbL 真开源** | re-api 限 feeder IP | 免费但**必须自架 ADS-B 接收器**馈送 | ✅/✅（依赖社区密度） | ✅ | 协议宽松；接入需硬件 |
| ADS-B Exchange 付费 API | 商业 | 订阅制 | 未公开 | ✅/✅ | ✅ | 唯一实时 + 历史 + 商用许可三位一体 |

**船舶（AIS）**

| 数据源 | 许可证 | 限速 | 成本 | 实时/全球 | 可商用 | 关键条款 |
|---|---|---|---|---|---|---|
| Global Fishing Watch API | CC BY-NC 4.0 | 50,000 req/天；1.5M/月/用户 | 免费 | ✅/⚠️ 仅渔船 + 部分商船 | ❌ | 明确 non-commercial only |
| AIS Hub | 私有合作协议 | 每分钟 1 次；**必须自架 AIS 接收器**馈送 NMEA | 免费（贡献换访问） | ✅/⚠️（80 国、1500+ 站） | ⚠️ 灰色 | 隐性 share-for-share；自建硬件加入即可 |
| MarineCadastre / NOAA | **美国公共领域** | 2GB/order；季度更新；3 年滚动 | 免费 | ❌/❌ 仅美国海域 + **历史非实时** | ✅ | 批量 CSV 下载，研究 US 港口流量极好 |
| EMSA SafeSeaNet | 政府间协议 | 仅会员国 | 不开放公众 | ⚠️/❌ 仅欧盟海域 | ❌ | 仅给欧盟成员国海事局 + 挪威 + 冰岛 |
| MarineTraffic API | 商业 | 默认 1 次/2 分钟（简单）/1 次/小时（扩展）；2025-01 起取消 addon/credits 销售 | 必须订阅 Online Plan + 单独买 API 服务 | ✅/✅ | ✅ | 限速极严，做实时雷达层会被 429 打死 |
| AISStream.io（Spire）/ VesselFinder / FleetMon / MyShipTracking / Shipfinder | 商业 | 商业 | 商业 | ✅/✅ | ✅ | 合规选项，均需付费 |

#### 决策理由

1. **合规阻塞**：OpenSky / GFW / ADS-B Exchange Community API 的非商用条款直接堵死 IntelHub 这类对外公开 OSINT 产品
2. **限速阻塞**：MarineTraffic 默认 1 次/2 分钟无法支撑 Radar 实时图层，会立即 429
3. **物理阻塞**：ADSB.lol / AISHub 需要自架 SDR 接收器（RTL-SDR + 天线 ~$100-200），IntelHub 是无 USB/SDR 接入能力的 VM
4. **付费路径成本**：FlightAware Personal + MarineTraffic Basic 组合约 $30-80/月 + 信用卡；超出免费层后年成本 $1000+

#### 未来重启评估的触发条件

任一条件满足时可重新评估：

- ① 用户申请到商业 ADS-B / AIS API 商业密钥（如 ADS-B Exchange 付费层、Spire AISStream）并愿意承担月费
- ② IntelHub 增加物理部署节点（能挂 SDR 硬件），可走 ADSB.lol + AISHub 馈送换访问路径
- ③ 出现新的开源协议（ODbL/MIT/Apache）下"全球/实时/可商用"的数据源（截至 2026-09 未见）
- ④ 业务定位明确转为"个人研究/学术原型"（可走 OpenSky research 协议 + GFW），不再做对外公开 OSINT 产品

#### 调研笔记来源

- OpenSky REST API 文档：https://openskynetwork.github.io/opensky-api/rest.html
- OpenSky 服务条款：https://opensky-network.org/about/terms-of-use
- ADS-B Exchange 数据产品：https://www.adsbexchange.com/data-products/
- ADS-B Exchange v2 字段：https://www.adsbexchange.com/version-2-api/
- JETNET 收购公告：https://www.jetnet.com/resources/press-releases/jetnet-acquires-ads-b-exchange
- FlightAware AeroAPI 定价：https://www.flightaware.com/commercial/aeroapi/
- ADSB.lol 文档：https://www.adsb.lol/docs/
- ADSB.lol API：https://api.adsb.lol/docs
- GFW 许可证与限速：https://globalfishingwatch.org/our-apis/documentation/docs/license-rate-limits
- GFW 认证：https://globalfishingwatch.org/our-apis/documentation/docs/authentication.md
- AISHub 加入：https://www.aishub.net/join-us
- AISHub API：https://www.aishub.net/api
- MarineCadastre：https://hub.marinecadastre.gov/pages/vesseltraffic
- EMSA SafeSeaNet：https://www.emsa.europa.eu/ssn-main.html
- MarineTraffic 限速：https://support.marinetraffic.com/en/articles/9552800-api-most-common-response-error-codes

---

### 2. OpenCorporates 公司注册库

- **决策日期**：2026-09-15
- **状态**：**条件性值得**（需走 public-benefit grant）
- **GitHub issue**：（未建——调查结论不是 wontfix，是需用户决策启动申请流程）
- **触发背景**：用户从 pyking/security_w1k1/wiki_OsintData.md 选出的候选源；评估 IntelHub 接入价值

#### 核心结论

**仅**通过 OpenCorporates 的 public-benefit grant 申请路径可行；付费 tier 最低 £2,250/年 ≈ $3,040（500 calls/month）对 OSS 项目不划算。IntelHub 公开 GitHub + 反腐 OSINT + 公开研究 三个属性正好命中 grant 资格条款（"investigative journalists, NGOs, universities and anti-crime-and-corruption research groups"）。

免费匿名 tier 早已关闭（2024+ 政策变更），200/月配额做不出可用功能。

#### 价格档位（GBP 年付；SavvyIQ 2026 换算 USD）

| 档位 | 年费 | 月费 | calls/month | calls/day |
|---|---|---|---|---|
| Free (open-data only) | £0 | £0 | 200 | 50 |
| Essentials | £2,250 | £225 | 500 | 200 |
| Starter | £6,600 | £660 | 2,500 | 500 |
| Basic | £12,000 | £1,200 | 5,000 | 1,000 |
| Enterprise | bespoke | — | custom | custom |

每 call 成本：Essentials ~$0.51 → Basic ~$0.27。Bulk + Enterprise 须联系 sales@opencorporates.com。

无独立 academic tier；只有 public-benefit grant 明文覆盖 journalism/NGO/学术/反腐。

#### API 能力

- `GET /companies/search` — `q` (free text, all-words-in-any-order 松散匹配)、`jurisdiction_code`（如 `us_de`/`gb`/`hk`/`sg`）、可选 `company_type`、`current_status`
- 公司对象字段：`company_number` + `jurisdiction_code`（复合 PK）、`name`、`company_type`、`current_status`、`incorporation_date`、`dissolution_date`、`registered_address`、`alternative_names`、`industry_codes`、`source`（含 provenance URL）、`officers`（独立 endpoint）、`previous_names`
- 鉴权：URL `?api_token=...` 或 `Authorization` header；每账号 `GET /account_status` 自查配额

#### 数据覆盖（按 Coverage HeatMap 实测）

| 司法管辖区 | 状态 | 备注 |
|---|---|---|
| Delaware (`us_de`) | monthly 更新，5.9M 公司，current_status 100% | 顶级质量 |
| Hong Kong (`hk`) | **offline** | 2.9M 历史 archive |
| Singapore (`sg`) | **offline** | 1.97M 公司，仅 14%（273K）索引 |
| BVI / Cayman / RoW 离岸 | 多为 archive snapshot | RoW 总 44.7M |
| UK Companies House / Florida | 高质量、随主 tier 包含 | **未发现 premium-jurisdiction 额外付费** |

#### ToS / 许可证

- Self-serve 路线：share-alike attribution；禁止大规模再分发底层数据，仅可 surface 在产品内；要求 provenance 链接可见
- Enterprise 路线：commercial license，internal/external distribution 可；attribution 仍必备
- 付费 tier 仅对**企业/金融机构/政府**（**非** NGO/journalist/academic）
- Public-benefit grant 必须保持非商业用途；若 IntelHub 后续接商业客户，需切到 paid tier
- 网站大规模 scraping 已被禁，必须走 API

#### 与 IntelHub 现有 monitors 对比

**零重叠**——互补层：

| 维度 | 来源 | 给什么 |
|---|---|---|
| 制裁 | ofac | 被制裁实体名单 |
| 联邦合同 | usaspending | 美国政府跟谁签合同 |
| 金融情报 | finintel | 上市公司财报 |
| **公司注册** | **❌（OpenCorporates 补）** | **法人主体、注册地、董事、子公司、离岸结构** |

OpenCorporates 是 OSINT 关系图谱的 entity 注册数据源，应挂入 Neo4j 而非进 Radar 信号流。

#### 决策路径

**推荐**：申请 public-benefit grant。

- 申请地址：https://opencorporates.atlassian.net/servicedesk/customer/portal/4/group/16/create/36
- IntelHub 资格论据：公开 GitHub + 反腐 OSINT + 公开研究 + 已有 NVD/OFAC/GDELT 等非商业 feed
- 预计 grant 周期：2-4 周（不可预期；不能当默认路径兜底）
- 风险：grant 拒绝 → 需 fallback 到 Essentials (£2,250/年 ≈ $3,040)

#### 未来启动条件

任一满足时可推进：

1. 用户决定申请 public-benefit grant（推荐路径）
2. 用户接受 Essentials 年费 £2,250 / $3,040 并提供 token
3. 出现新的免费替代（如某个司法管辖区的开放 registry API）

#### 调研笔记来源

- Pricing: https://opencorporates.com/pricing/ · https://opencorporates.com/plug-in-our-data/
- API docs: https://api.opencorporates.com/documentation/API-Reference
- Data Dictionary: https://knowledge.opencorporates.com/knowledge-base/data-dictionary-companies/
- Coverage HeatMap: https://knowledge.opencorporates.com/knowledge-base/coverage-heatmap/
- Jurisdiction status: https://knowledge.opencorporates.com/knowledge-base/overview/
- ToS: https://opencorporates.com/terms-of-use-2/ · https://opencorporates.com/legal-information/self-service-api-terms-of-service/ · https://opencorporates.com/legal-information/enterprise-api-terms-of-service/
- Public-benefit 申请: https://opencorporates.atlassian.net/servicedesk/customer/portal/4/group/16/create/36
- Bellingcat 入门: https://www.bellingcat.com/resources/2023/08/24/following-the-money-a-beginners-guide-to-using-the-opencorporates-api/
- 第三方评测 (2026): https://zephira.ai/opencorporates-pricing-explained-2026-plans-api-limits-licensing-and-what-it-means-in-production/ · https://savvyiq.ai/compare/opencorporates

---

### 3. OSINT Framework 三金 collector（OpenSanctions / OSV / SEC EDGAR）

- **决策日期**：2026-09-15
- **状态**：**全部已落地**（feat/osint-trio，main 部署，推）
- **GitHub issue**：（未建——直接走 feature 分支交付）
- **触发背景**：用户从 https://osintframework.com （lockfale/OSINT-Framework 仓库 1.1MB JSON tree，33 顶层类 / 1169 叶子节点）评估后选定三个互补性最强的真开源 feed。

#### 核心结论

OSINT Framework 本身是**元目录**（link tree），不是数据 feed，整体集成价值低。但 tree 里的具体 data source 链接里有三个跟 IntelHub 现有 28 个 monitor **零重叠**、**真开源 + 完整 API** 的高价值源——三金打包交付。

#### 三金实现总结

| Collector | 许可证 | API 限速 / key | IntelHub 写入 | 现有重复 |
|---|---|---|---|---|
| **OpenSanctions** | ODbL | 需 key（secrets.env），免费 | geo_events (kind=sanction, anchor=Treasury DC) | ofac（仅 SDN） |
| **OSV.dev** | CC-BY 4.0 | 无 key，无限速 | geo_events (kind=cyber, anchor=OSV HQ Mountain View CA) | nvd/cisakev（不同范围） |
| **SEC EDGAR** | US 公共领域 | 无 key，需 User-Agent 含 email | geo_events (kind=filing, anchor=SEC HQ Washington DC) | finintel/fd.rs（不覆盖 SEC filing 本身） |

#### OpenSanctions — 制裁/PEP 全聚合

- **是什么**：聚合 30+ 制裁/PEP/watchlist（OFAC SDN + EU CFSP + UN + UK HMT + INTERPOL + 各国 PEP）的 ODbL 数据库
- **API**：GET /datasets/default → entity_count + last_change
- **IntelHub 实现**：`monitor/sources/opensanctions.rs`，24h 间隔，emit 1 signal/天 based on dataset last_change dedup
- **限制**：free tier 需 API key（注册 https://www.opensanctions.org/api/ ）—— 无 key 时 shelved-by-design（与 firms/reliefweb 同样模式）
- **集成后状态**：✅ code shipped, ⏸️ 等待用户注册 + 配 HUB_OPENSANCTIONS_API_KEY

#### OSV.dev — 开源生态 CVE

- **是什么**：Google 维护的开源生态专向漏洞 DB（PyPI/npm/Go/crates.io/Maven/RubyGems 等 10+ ecosystem）
- **API**：POST /v1/query（无 key，免费，无限速）
- **IntelHub 实现**：`monitor/sources/osv.rs`，6h 间隔，watchlist（默认 12 包，可 HUB_OSV_WATCH override），client-side `modified` 时间戳过滤
- **与 NVD 区别**：NVD 是通用 CVE，OSV 是包生态专向 + 含 ecosystem-specific 版本解析
- **集成后状态**：✅ code shipped + live（415 首轮 fetched=1，sp5 全绿）

#### SEC EDGAR — 美国上市公司 filing

- **是什么**：SEC 官方全量 filing feed（10-K/10-Q/8-K/Form 4/DEF 14A 等）
- **API**：EFTS full-text search（GET /LATEST/search-index，无 key，需 User-Agent）
- **IntelHub 实现**：`monitor/sources/sec_edgar.rs`，6h 间隔，24h lookback，默认 form=8-K（material events，可 HUB_SEC_FORM override）
- **缺口背景**：`finintel.rs` 是 finnhub + stocktwits，`fd.rs` 是 financialdatasets.ai，**都不覆盖 SEC filing 本身**——10-K/10-Q/8-K/13F/Form 4 这块原是真空
- **集成后状态**：✅ code shipped + live（未配置 HUB_SEC_USER_AGENT_EMAIL 时 shelved；配置后 8-K filings 会进 geo_events）

#### 三金设计选择

| 决策 | 理由 |
|---|---|
| OpenSanctions **tempo signal**（per-day），不做 entity extraction | 对齐 ofac.rs 模式——per-entity 落 graph 平面是后续 MCP 工具阶段，不是 Radar 地图 |
| OSV **client-side `modified` 过滤** | OSV 服务端无 severity filter；`modified` 字段是增量检测的唯一 stable signal |
| SEC EDGAR **默认 form=8-K**（material events） | 8-K 是最高新闻价值子集；10-K/10-Q 季报节奏不快；Form 4 insider 高量高噪 |
| 三金共用不同 anchor（同 DC 区但视觉可区分） | OSV HQ Mountain View CA / SEC HQ DC / Treasury DC + 已有 OFAC DC |
| 都用 `&'static str` kind 名 | 与现有 Signal 约定兼容，不破坏 kind taxonomy |

#### 重启 / 配置 checklist

- **OpenSanctions**：用户去 https://www.opensanctions.org/api/ 注册 free tier → `echo 'HUB_OPENSANCTIONS_API_KEY=xxx' >> /home/zou/IntelHub/core/secrets.env` → restart hub-core。无需 rebuild。
- **SEC EDGAR**：用户填 `HUB_SEC_USER_AGENT_EMAIL=your-email@domain.com` 到 secrets.env → restart。无需 rebuild。
- **OSV**：默认 watchlist 已部署，无需配置。可通过 `HUB_OSV_WATCH=PyPI:requests,npm:lodash` override。

#### 不选的（OSINT Framework 评估中显式拒绝）

- **OSINT Framework tree 整体集成** — 元目录，不是 feed，console 嵌入价值低
- **Hoaxy / PolitiFact / Snopes** — 无生产级 API
- **ExploitDB / Packet Storm** — 漏洞利用代码，超出 IntelHub scope
- **NHTSA Vehicle API** — niche 单国家车辆数据，与 OSINT 调查不交叉
- **CourtListener / PACER** — niche 司法记录

#### 调研笔记来源

- OSINT Framework tree: https://github.com/lockfale/OSINT-Framework （public/arf.json，1112KB / 33 类 / 1169 叶子）
- OpenSanctions API: https://www.opensanctions.org/api/ · https://www.opensanctions.org/datasets/default/
- OSV API: https://api.osv.dev/v1/query · https://google.github.io/osv.dev/
- SEC EDGAR EFTS: https://efts.sec.gov/LATEST/search-index · https://www.sec.gov/edgar/sec-api-documentation
- Cargo clean force rebuild: `docker run --rm -v intelhub-hub-target:/target alpine sh -c 'rm -rf /target/release/.fingerprint /target/release/deps /target/release/build'`（避坑：rsync 后 cargo incremental cache 可能 stale）

---

## 文档维护

- 新增 wontfix 项：在下方加新章节，保持格式一致
- 重启评估：先在 issue 讨论，更新本文档状态字段；不要直接删历史