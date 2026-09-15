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

## 文档维护

- 新增 wontfix 项：在下方加新章节，保持格式一致
- 重启评估：先在 issue 讨论，更新本文档状态字段；不要直接删历史