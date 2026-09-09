OSINT Intelligence Hub[工程实施指令]
构建下一代 Multi-Agent / MCP 原生、Docker-first 云边协同 OSINT Intelligence Hub

0. 项目最终定义
在 Proxmox VE (PVE) 上构建一台基于 Debian 13 Trixie 的纯 CPU OSINT Intelligence Hub 节点。
本系统不是：
* 某一个 AI Agent 的专属后端
* Aleph 的专属情报数据库
* 一组简单拼装的 Docker 容器
* 多个第三方 Web UI 的 iframe 集合
本系统必须被定义为：
一个 Agent-agnostic、MCP-native、Docker-first 的共享情报基础设施与统一情报工作台。
系统的核心职责：
Search
Crawl
Observe
Collect
Normalize
Deduplicate
Store
Index
Graph
Retrieve
Correlate
Expose
Visualize
Alert
Audit
Manage
外部 Agent 的核心职责：
Reasoning
Planning
Hypothesis
Investigation
Synthesis
Decision Support
本地 Intelligence Hub 必须提供：
Sensors
Evidence
Memory
Graph
Search
Tools
Tasks
Events
Alerts
Audit
Unified UI
MCP
Lifecycle Management
系统不得依赖某一个 Agent 才能正常运行。

1. 总体架构原则
必须采用以下架构：
Host Foundation
+
Docker-first Third-party Services
+
Native Intelligence Hub Core
+
Local Sensor Fabric
+
Local Evidence / Memory Fabric
+
Unified Intelligence Console
+
Agent-neutral MCP Gateway
+
External Agent / Cloud Cognition Plane
最终逻辑架构：
                     ┌─────────────────────────┐
                     │      EXTERNAL AGENTS    │
                     │                         │
                     │ Codex                   │
                     │ Claude Code             │
                     │ Aleph                   │
                     │ Other MCP Agents        │
                     │ Custom Agents           │
                     └────────────┬────────────┘
                                  │
                           MCP / HTTPS
                                  │
                     ┌────────────▼────────────┐
                     │    INTELLIGENCE HUB     │
                     │                         │
                     │ Unified Console         │
                     │ MCP Gateway             │
                     │ Evidence API            │
                     │ Investigation Engine    │
                     │ Event Bus               │
                     │ Policy / Audit           │
                     │ Component Manager       │
                     └────────────┬────────────┘
                                  │
           ┌──────────────────────┼────────────────────────┐
           │                      │                        │
           ▼                      ▼                        ▼
      Sensor Fabric         Evidence Fabric          Memory Fabric
           │                      │                        │
       Search/Crawl          PostgreSQL                Neo4j
       OSINT Sensors         Object/Raw Data           Qdrant
                                                       Redis
           │
        Internet
           │
      Egress Gateway

2. 核心信任模型
必须严格区分：
External Agent
=
Untrusted Consumer

Cloud LLM
=
Untrusted Reasoning Engine

MCP Gateway
=
Trusted Capability Boundary

Intelligence Hub
=
Trusted Data / Tool Plane

Databases
=
Persistence Layer
所有 Agent 和云端模型产生的输入一律视为：
UNTRUSTED INPUT
不得因为 Agent 通过 MCP 接入而获得：
shell
sudo
host filesystem
Docker socket
arbitrary SQL
arbitrary Cypher
arbitrary network access

3. Docker 总体策略
3.1 核心原则
采用：
Docker-first, not Docker-only。
Docker 的目的不是单纯降低安装难度，而是：
dependency isolation
version isolation
upgrade isolation
rollback
reproducibility
resource control
network segmentation
third-party lifecycle isolation
禁止为了所谓“原生性能”而把所有第三方组件直接安装到 Debian Host。

4. Deployment Tier 模型
所有组件按照以下等级部署。
Tier 0 — Host Native
仅允许以下组件直接运行于 Debian Host：
Debian 13
systemd
Docker Engine
Docker Compose
SSH
nftables / firewall
QEMU Guest Agent
time synchronization
host telemetry
backup tooling
Intelligence Hub Core

Tier 1 — Dockerized Core Infrastructure
以下服务默认必须 Docker 化：
PostgreSQL
Redis
Neo4j
Qdrant
原因：
stable dependency boundary
persistent volumes
upgrade isolation
backup / restore
rollback
reproducibility

Tier 2 — Dockerized Sensors / Applications
以下服务默认 Docker 化：
SearXNG
Crawl4AI
SpiderFoot
Huginn
Crucix
Grafana
所有组件必须拥有：
image version
config
volume
network
resource limit
healthcheck
restart policy

Tier 3 — Native Exception
只有满足以下任一条件时，才允许第三方项目采用 Native Deployment：
1. 官方 Docker 支持明显不成熟
2. Docker 版本存在关键功能缺失
3. 原生版本对性能有重大且可验证的优势
4. Docker 化会导致关键 Host Integration 能力缺失
5. 官方长期维护方向明确偏向 Native
并必须同时满足：
isolated virtualenv / runtime
systemd service
独立目录
明确版本
备份策略
升级策略
回滚策略
禁止：
pip install
npm install
apt install
直接污染系统全局运行环境。

5. 为什么选择 Docker-first
系统设计必须明确：
Container
≠ VM
Docker container 共享 Debian Host kernel，不承担完整 Guest OS、Guest kernel 和虚拟硬件的额外成本。
系统资源的主要消耗来源应优先视为：
Chromium / Playwright
Neo4j
Qdrant
PostgreSQL
Cache
Actual workload
而不是把 Docker runtime 本身视为主要瓶颈。
优化顺序：
1. service memory limits
2. browser concurrency
3. database cache
4. worker concurrency
5. network throughput
6. storage I/O
7. container runtime overhead
禁止因为 Docker 存在少量运行时开销而放弃完整的依赖隔离能力。

6. Component Lifecycle Manager
Intelligence Hub 必须内置：
Component Lifecycle Manager
用于管理所有第三方组件。
每个组件建立 manifest：
name:
type:
deployment:
image:
version:
source:
channel:
volume:
networks:
resources:
healthcheck:
backup:
upgrade:
rollback:
native_ui:
Lifecycle 必须支持：
Discover Update
Check Compatibility
Pull New Image
Backup
Create Temporary Instance
Health Check
Integration Test
Upgrade
Post-upgrade Validation
Rollback

7. Update Policy
禁止直接：
docker compose pull
docker compose up -d
然后认为升级完成。
标准升级流程：
Upstream Update
        ↓
Detect
        ↓
Review Changelog
        ↓
Compatibility Check
        ↓
Backup
        ↓
Pull Image
        ↓
Disposable Test Instance
        ↓
Health Check
        ↓
Integration Test
        ↓
Promote
        ↓
Monitor
失败：
Rollback

8. Version Pinning
生产环境禁止盲目：
latest
必须尽可能固定：
major
minor
patch
image digest
至少保证：
production version
last known good version
两者始终可追踪。

9. PVE 虚拟机
9.1 OS
Debian 13 Trixie Cloud-Init
9.2 推荐规格
CPU: 6 vCPU
RAM: 24 GB
Disk: 100 GB
GPU: None
NIC: VirtIO
Storage: ZFS / LVM-thin
SSD Emulation: Enabled
Discard/TRIM: Enabled
最低配置：
4 vCPU
16 GB RAM
最低模式允许关闭：
SpiderFoot
Huginn
Grafana
等非关键服务。

10. Cloud-Init
自动完成：
static IP
hostname
DNS
SSH Ed25519
qemu-guest-agent
time synchronization
Docker Engine
Docker Compose plugin
firewall
directory initialization
Docker 安装优先采用官方 Docker repository。
优先使用：
docker-ce
docker-ce-cli
containerd.io
docker-buildx-plugin
docker-compose-plugin
而不是把 Debian 内置 Docker 包作为长期生产基线。

11. Host Security
必须：
SSH key-only
PasswordAuthentication no
PermitRootLogin no
开启：
persistent journald
security updates
time synchronization
Docker 对外暴露端口必须结合：
DOCKER-USER
nftables / iptables
进行访问控制。

12. Docker Network Architecture
至少建立：
mgmt-net
data-net
sensor-net
egress-net

13. mgmt-net
用于：
Intelligence Hub
Crucix
Grafana
management
trusted Agent access

14. data-net
必须：
internal: true
包含：
PostgreSQL
Redis
Neo4j
Qdrant
默认禁止 Internet egress。

15. sensor-net
包含：
SearXNG
Crawl4AI
SpiderFoot
Huginn
用于传感器内部通信。

16. egress-net
用于外部 Internet 访问。
默认只有必要 Sensor / Collector 可以加入：
SearXNG
Crawl4AI
Crucix collectors
外部连接通过：
OPNsense / OpenWrt
承担：
routing
DNS policy
logging
rate limiting
network reliability
outbound control
代理被定义为：
可审计的外部网络出口控制层。
不得把其系统设计目标定义为绕过第三方访问控制。

17. Port Exposure Strategy
禁止把所有组件都暴露给 LAN。
不要默认：
7474
7687
6333
6334
3001
3117
全部发布为 Host Port。
推荐：
Browser
   ↓
Intelligence Hub
   ↓
internal Docker network
   ↓
Neo4j / Qdrant / PostgreSQL / Sensors
原则：
用户尽量只需要访问一个入口：Intelligence Hub。
例如：
https://intel.local/
底层组件的 Native UI 仅作为高级诊断入口。

18. Docker Compose Structure
目录：
/opt/intelligence-hub/
建议：
compose/
├── compose.base.yml
├── compose.data.yml
├── compose.sensor.yml
├── compose.ui.yml
└── .env

manifests/
config/
scripts/
backups/

19. Core Services
Data
postgres
redis
neo4j
qdrant
Sensors
searxng
crawl4ai
spiderfoot
huginn
Applications
crucix
grafana
Observability
node-exporter
cAdvisor
Native Core
intelligence-hub-core

20. Intelligence Hub Core
Intelligence Hub Core 是本系统唯一必须拥有 Native-first 能力的核心服务。
优先使用：
Rust
构建单一 binary。
可采用：
systemd
直接运行。
它负责：
MCP Gateway
Hub API
Investigation State
Tool Registry
Policy Engine
Agent Identity
Event Routing
Audit
Cost Governance
Component Lifecycle
如后续容器化 Hub Core，也必须保持：
single binary
stateless where possible
persistent state externalized

21. Intelligence Hub UI 是一级核心产品
不得将 UI 定义为：
Dashboard
必须定义为：
Unified Intelligence Console
+
Investigation Workspace
+
Evidence Browser
+
Agent Workspace
+
Operations Console
+
System Control Center

22. UI 核心目标
用户不需要为了完成一次情报调查而频繁跳转：
Crucix
Neo4j Browser
Qdrant UI
Grafana
SearXNG
SpiderFoot
Huginn
Crawl4AI
Intelligence Hub 必须统一显示：
System Status
Sensor Status
Active Investigations
Evidence
Entities
Relationships
Timeline
Semantic Retrieval
Agent Analysis
Alerts
Costs
Audit

23. Native UI 保留原则
Intelligence Hub 不替代原项目 Native UI。
每个系统继续保留：
Crucix       → Native UI
Neo4j        → Neo4j Browser
Grafana      → Grafana
SearXNG      → SearXNG UI
SpiderFoot   → SpiderFoot UI
Huginn       → Huginn UI
Qdrant       → Native/API administration
Intelligence Hub 提供：
Open Native UI
从统一页面跳转到原生管理页面。

24. 禁止使用大量 iframe 作为整合方案
不得简单实现：
Crucix iframe
+
Neo4j iframe
+
Grafana iframe
+
...
作为主要 UI 架构。
应建立：
Intelligence Hub API
由 Hub Backend 聚合：
Postgres
Neo4j
Qdrant
Redis
Sensors
Crucix
System telemetry
Agent events
Frontend 只与：
Hub API
MCP/Event APIs
通信。
iframe 只能用于：
rare fallback
native administration

25. Intelligence Hub Overview
默认首页必须显示：
System Health
Active Investigations
Recent Alerts
Sensor Activity
Agent Activity
Evidence Ingestion
Graph Changes
Semantic Memory
Cloud API Usage
必须提供：
Global Radar
Live Event Stream
System Status

26. Global Intelligence Radar
整合 Crucix 等地理情报信息。
支持：
events
entities
alerts
investigation targets
sensor activity
支持：
time filter
entity filter
severity filter
source filter
investigation filter
路径：
Map Event
 ↓
Evidence
 ↓
Entities
 ↓
Graph
 ↓
Agent Finding

27. Investigation Workspace
Investigation 必须成为系统一级对象。
用户创建：
Investigation
并围绕一个：
target
question
hypothesis
开展工作。
每个 Investigation 包含：
Overview
Hypothesis
Evidence
Timeline
Entities
Graph
Related Documents
Agent Findings
Alerts
Audit

28. Agent Analysis 是一级内容
Agent 的分析结果不得仅显示在聊天窗口。
必须结构化为：
Finding
Claim
Hypothesis
Assessment
Confidence
Supporting Evidence
Contradicting Evidence
Related Entities
Recommended Next Actions
UI 示例：
AGENT FINDING

Organization A may be linked to Entity B

Confidence: 0.81

Supporting Evidence: 7
Contradicting Evidence: 2

[View Evidence]
[View Graph]
[Ask Agent]
[Investigate Further]

29. Evidence-backed Agent Analysis
所有 Agent Finding 必须绑定 Evidence。
必须支持：
Finding
 ↓
Supporting Evidence
 ↓
Original Document
 ↓
Source
 ↓
Retrieval Time
同时支持反向：
Evidence
 ↓
Claims
 ↓
Agent Findings
不得把：
Agent inference
直接呈现为：
verified fact
UI 必须明显区分：
FACT
OBSERVATION
AGENT CLAIM
HYPOTHESIS
ALERT
SYSTEM STATE

30. Multi-Agent Architecture
系统不得绑定 Aleph。
支持：
Codex
Claude Code
Aleph
Custom Agents
Other MCP-compatible Agents
所有 Agent 都通过：
MCP
访问 Intelligence Hub。
逻辑：
Codex
   │
Claude Code
   │
Aleph
   │
Other Agents
   │
   └──────── MCP ────────► Intelligence Hub

31. Aleph 的正式定位
Aleph 是：
One of the External Agents
而不是：
Intelligence Hub 的唯一 AI Core。
Aleph 可以负责：
Planning
Reasoning
Investigation
Orchestration
但：
Aleph offline
时 Intelligence Hub 必须仍然可以：
Search
Crawl
Store Evidence
Maintain Graph
Maintain Memory
Serve MCP
Display UI
Generate Alerts
Manage Components

32. Agent-neutral Tool Design
Tool 必须表达：
Capability
例如：
search_web
crawl_url
fetch_document
query_entity
query_relationship
find_path
semantic_search
get_evidence
create_investigation
get_task_status
get_system_health
禁止：
aleph_research
aleph_memory
aleph_graph
等 Agent-specific Tool。

33. MCP Gateway
Intelligence Hub 必须提供标准 MCP Server：
/mcp
远程 MCP 优先：
Streamable HTTP
MCP 用于：
tool discovery
tool invocation
resource access
structured agent integration
MCP 不是：
remote shell
也不是：
arbitrary command execution

34. Agent Identity
每个请求必须记录：
agent_id
agent_version
session_id
task_id
request_id
trace_id
系统必须可以回答：
哪个 Agent 发起了这次搜索？
哪个 Agent 访问了这个 Evidence？
哪个 Agent 创建了这个 Finding？
哪个 Agent 修改了这个 Relationship？
哪个 Agent 消耗了多少资源？

35. Shared Intelligence Memory
所有 Agent 可以共享：
Evidence
Documents
Entities
Relationships
Semantic Memory
Investigations
Findings
Alerts
但必须保存：
created_by
updated_by
agent_id
task_id
timestamp
最终形成：
Shared Evidence Memory
+
Shared Intelligence Graph

36. PostgreSQL
定义为：
Canonical Metadata / Evidence Store
保存：
tasks
investigations
documents
sources
claims
entities
observations
provenance
agent_runs
tool_calls
cost_records
audit_records

37. Redis
定义为：
Transient Coordination Layer
用于：
queue
locks
task state
worker coordination
rate limiting
short-lived cache
不得把 Redis 作为永久事实数据库。

38. Neo4j
定义为：
Relationship Memory
保存：
entities
relationships
events
sources
temporal relationships
禁止 Agent 直接执行任意 Cypher。
必须：
Agent
 ↓
Typed Graph Intent
 ↓
Schema Validation
 ↓
Authorization
 ↓
Parameterized Operation
 ↓
Neo4j

39. Qdrant
定义为：
Semantic Memory
Qdrant 不负责生成 Embedding。

40. Selective Embedding
全文 Embedding 不得成为默认行为。
标准流程：
Raw Evidence
 ↓
Canonicalization
 ↓
Content Hash
 ↓
Exact / Near Duplicate Detection
 ↓
Metadata / Keyword Index
 ↓
Embedding Decision
       │
       ├── NO → Store metadata / keyword index
       │
       └── YES
             ↓
         Cloud Embedding
             ↓
           Qdrant
只有满足以下条件之一时才 Embedding：
需要语义检索
高价值情报
跨文档语义关联
Agent 明确要求
高优先级来源
长期知识候选
以下默认不 Embedding：
重复新闻
导航页
模板页
低信息量页面
boilerplate
临时页面
重复抓取版本

41. Hybrid Retrieval
统一搜索必须同时支持：
Keyword
Metadata
Entity
Graph
Vector
Hybrid
查询流程：
User / Agent Query
        │
        ├── Keyword
        ├── Metadata
        ├── Entity
        ├── Graph
        └── Semantic
              │
              ▼
         Candidate Set
              │
              ▼
            Rerank
              │
              ▼
         Evidence Set
Semantic Retrieval 只有在产生实际价值时才触发 Embedding 成本。

42. Embedding Cache
Embedding cache key：
content_hash
+
chunk_algorithm_version
+
embedding_model
如果三者没有变化：
NO RE-EMBEDDING

43. Evidence Provenance
所有 Evidence 至少记录：
document_id
source_url
retrieved_at
published_at
content_hash
source_type
raw_capture
parent_task
所有 Claim 必须追溯：
Claim
 ↓
Evidence
 ↓
Document
 ↓
Source
 ↓
Timestamp

44. Evidence Ledger
任何 Agent Finding 必须记录：
claim
evidence_ids
source_ids
document_ids
content_hashes
agent_id
model
task_id
timestamp
prompt_hash
Intelligence Hub UI 必须支持一键查看完整证据链。

45. Sensor Fabric
包含：
SearXNG
Crawl4AI
SpiderFoot
Huginn
Crucix collectors

46. SearXNG
职责：
federated search
source discovery
query expansion
不承担：
deep reasoning

47. Crawl4AI
职责：
HTML
DOM
article text
metadata
links
screenshots when required
必须限制：
memory
CPU
concurrency
timeout
retry
page count
content size
默认：
max concurrency = 4
memory ≈ 3 GB
根据 telemetry 动态调整。

48. SpiderFoot
用于：
structured OSINT enrichment
输出必须先进入：
Normalization / Ingest Layer
禁止直接向 Neo4j 写入未验证结果。

49. Huginn
负责：
watchers
scheduled jobs
event workflows
禁止默认允许高权限系统命令执行。

50. Normalization / Ingest Layer
必须包含独立逻辑服务：
osint-ingest
负责：
schema normalization
URL canonicalization
timestamp normalization
content hashing
deduplication
provenance
encoding normalization
source attribution
统一 Evidence Event：
{
  "event_id": "...",
  "source": "...",
  "url": "...",
  "retrieved_at": "...",
  "published_at": "...",
  "content_hash": "...",
  "content": "...",
  "metadata": {},
  "provenance": {}
}
这是所有 AI 分析之前的统一入口。

51. Crucix
Crucix 定位：
Macro Intelligence / Global Signal Layer
用于：
global events
news
conflict
aviation
maritime
satellite
economic indicators
alerts
Crucix 自己的 Native UI 保留。
Intelligence Hub 聚合其中：
关键事件
关键 Alert
关键地图信息
而不是复制整个 Crucix。

52. Grafana
Grafana 继续作为专业 observability 页面。
Intelligence Hub 聚合：
CPU
RAM
Disk
Network
Container health
Crawler throughput
Queue depth
API latency
Cloud API calls
Token usage
Cost
Failure rate
高级监控仍然可以跳转 Grafana。

53. Unified Sensor Control Center
Intelligence Hub 统一显示：
SearXNG
Crawl4AI
SpiderFoot
Huginn
Crucix collectors
每个 Sensor：
status
requests/min
queue
success rate
error rate
latency
last activity
允许：
Start
Stop
Restart
Pause
Resume
View Logs
Open Native UI

54. Unified Alert Center
统一收集：
OSINT alerts
Agent alerts
Sensor alerts
Infrastructure alerts
Budget alerts
Security alerts
每条 Alert：
severity
timestamp
source
task
investigation
entity
evidence
recommended action
支持：
Acknowledge
Mute
Investigate
Assign to Agent
Add to Investigation

55. Agent Workspace
用户必须可以看到：
Agent
Session
Task
Tools
Context
Evidence
Findings
Cost
显示：
Codex started investigation
 ↓
Search executed
 ↓
14 sources discovered
 ↓
Claude Code performed semantic search
 ↓
Aleph correlated entities
 ↓
New finding generated
形成：
Agent Activity Timeline

56. Agent Finding Visualization
必须支持：
Finding
Confidence
Evidence count
Contradicting evidence
Source diversity
Last updated
不要只显示：
Confidence: 82%
同时显示：
Supporting Evidence: 7
Contradicting Evidence: 2
Independent Sources: 4
Last Updated: ...

57. Confidence Model
不得简单把模型输出的 confidence 当成事实可信度。
建议拆成：
source_confidence
claim_confidence
model_confidence
综合评估可以参考：
source reliability
× evidence count
× cross-source agreement
× temporal consistency
× extraction quality
UI 必须允许查看置信度形成原因。

58. Gas / Cost Governor
成本控制必须是：
Intelligence Hub Infrastructure Capability
而不是 Aleph 专属能力。
每个 Agent 单独拥有：
tool budget
crawl budget
LLM budget
embedding budget
concurrency limit
rate limit
系统拥有：
global budget

59. Gas Model
成本必须考虑：
tool calls
HTTP requests
crawl pages
LLM input tokens
LLM output tokens
embedding tokens
retry count
wall time
concurrency weight
而不是单纯：
confidence × depth

60. Budget State
GREEN
正常执行。
YELLOW
降低探索深度和并发。
RED
停止扩展新的搜索分支，只处理已有证据。
KILL
终止任务并保存当前结果。
任何 Agent runaway 都不得影响其他 Agent。

61. High-risk Action Policy
分三级：
Level 1
store_document
store_observation
允许自动执行。
Level 2
create_entity
create_claim
create_relationship
需要 schema validation。
Level 3
delete
bulk modification
configuration change
sensor control
network modification
默认：
DENY
必须显式授权。

62. Investigation Task Model
每次调查建立：
task_id
investigation_id
Investigations 与 Agent 解耦。
例如：
Task A
created_by = Codex

Task B
created_by = Claude Code

Task C
created_by = Aleph
三个任务可以访问共享 Evidence 和 Memory。

63. Event Bus
所有实时事件进入统一 Event Bus：
Sensor
Agent
Database
Worker
System
      │
      ▼
  Event Bus
      │
      ├── Intelligence Hub UI
      ├── Alert Engine
      ├── Audit
      └── Agent Event Stream
事件类型：
TASK_CREATED
TASK_STARTED
TASK_PROGRESS
TASK_COMPLETED
DOCUMENT_INGESTED
ENTITY_DISCOVERED
RELATION_CREATED
FINDING_CREATED
ALERT_RAISED
SENSOR_ERROR
BUDGET_WARNING
BUDGET_EXCEEDED
COMPONENT_UPDATED
COMPONENT_UPDATE_FAILED

64. Communication Architecture
Agent ↔ Hub
MCP Streamable HTTP
Hub ↔ Browser
SSE
WebSocket where interactive bidirectional event delivery is needed
Hub ↔ Internal Workers
Redis / internal event bus
不要让 MCP 成为所有内部服务之间的消息总线。

65. Shared Memory Principle
必须实现：
One Intelligence Hub, Many Agents, Shared Memory.
多个 Agent 可以：
读取
关联
补充
验证
引用
同一个 Evidence。
但是所有操作都必须保留 provenance。

66. Audit Center
所有关键行为必须记录：
user
agent
sensor
tool
database
finding
alert
component
必须可查询：
who
what
when
why
source
result

67. Unified Search
Intelligence Hub 必须提供全局统一搜索入口。
输入：
Entity X
返回统一结果：
Exact Matches
Documents
Evidence
Entities
Relationships
Similar Documents
Timeline
Agent Findings
Alerts
Investigations
用户不需要决定：
我现在应该查 Neo4j 还是 Qdrant？
系统根据查询自动选择：
Keyword
Metadata
Graph
Semantic
Hybrid

68. System Health Center
统一展示：
PVE
Debian
Docker
Intelligence Hub
PostgreSQL
Redis
Neo4j
Qdrant
SearXNG
Crawl4AI
SpiderFoot
Huginn
Crucix
Grafana
MCP Gateway
每个组件显示：
status
version
uptime
resource usage
health
last error
update status

69. Component Manager UI
Intelligence Hub 必须能显示：
Component
Installed Version
Latest Known Version
Status
Health
Update Available
Last Updated
操作：
View Changelog
Backup
Test Upgrade
Upgrade
Rollback
Open Native UI
View Logs

70. Backup Strategy
必须区分：
Configuration
Database
Evidence
Graph
Vector Memory
Secrets
Component Manifest
至少支持：
Postgres backup
Neo4j backup
Qdrant snapshot
configuration backup
environment manifest backup
升级前自动创建可恢复点。

71. Resource Limits
推荐初始：
Crawl4AI:
  memory: 3G
  cpu: 2

Neo4j:
  memory: 2G–4G

Qdrant:
  memory: 2G–4G

Postgres:
  memory: 1G–2G

Redis:
  memory: 256M–512M

Grafana:
  memory: 512M

SearXNG:
  memory: 512M–1G

Crucix:
  memory: 512M–1G
所有组件都必须基于实际 telemetry 再调整。

72. Failure Model
Aleph offline
系统继续运行。
Codex offline
系统继续运行。
Claude Code offline
系统继续运行。
Cloud LLM unavailable
继续采集和保存 Evidence。
Embedding API unavailable
文档保留：
embedding_status = PENDING
Qdrant unavailable
Evidence 不得丢失。
Neo4j unavailable
Graph update 必须进入 queue。
Crawl4AI unavailable
Task 重试。
Intelligence Hub UI unavailable
MCP 和各 Native UI 仍保持可用。
Docker service failure
自动 restart，并产生 System Alert。

73. Security Failure Model
必须确保：
LLM cannot execute shell
LLM cannot execute sudo
LLM cannot access Docker socket
LLM cannot access arbitrary filesystem
LLM cannot execute arbitrary SQL
LLM cannot execute arbitrary Cypher
任何模型生成的：
tool arguments
Cypher
SQL
URLs
JSON
filesystem paths
都必须先验证。

74. 最终用户工作流
Workflow A — 快速调查
User
 ↓
Intelligence Hub
 ↓
Create Investigation
 ↓
Question / Target
 ↓
Select Agent
 ↓
Agent calls MCP
 ↓
Search / Crawl / Graph / Memory
 ↓
Evidence
 ↓
Agent Findings
 ↓
Investigation Result

Workflow B — Alert Investigation
Alert
 ↓
Open
 ↓
Create Investigation
 ↓
Related Evidence
 ↓
Graph
 ↓
Agent Analysis
 ↓
Timeline
 ↓
Conclusion

Workflow C — Entity Investigation
Entity
 ↓
Graph
 ↓
Relationships
 ↓
Sources
 ↓
Documents
 ↓
Semantic Memory
 ↓
Agent Analysis

Workflow D — Multi-Agent Investigation
Investigation
      │
      ├── Codex
      │
      ├── Claude Code
      │
      └── Aleph
             │
             ▼
      Shared Intelligence Hub
             │
       ┌─────┼─────┐
       ▼     ▼     ▼
    Evidence Graph Memory
       │     │     │
       └─────┼─────┘
             ▼
       Unified Findings

75. UI 设计原则
必须：
Information Dense
Signal First
Evidence First
Context First
Actionable
避免：
excessive decoration
dashboard vanity metrics
unnecessary cards
用户应该看到：
What happened?
What do we know?
What changed?
What is connected?
What does the Agent infer?
What evidence supports it?
What should happen next?
而不是：
有多少 Docker 容器

76. Final Information Hierarchy
系统 UI 的抽象层级必须是：
Investigation
    ↓
Finding
    ↓
Evidence
    ↓
Entity / Relationship
    ↓
Source
而不是：
Container
    ↓
Database
    ↓
Sensor
Docker / Container 只属于：
System / Operations
不能成为用户进行情报调查时的主要抽象。

77. 最终系统逻辑
                       EXTERNAL AGENTS
       ┌────────────┬────────────┬────────────┐
       │            │            │            │
     Codex     Claude Code      Aleph      Other
       │            │            │            │
       └────────────┴──────┬─────┴────────────┘
                           │
                          MCP
                           │
             ┌─────────────▼──────────────┐
             │      INTELLIGENCE HUB      │
             │                            │
             │ Unified Console            │
             │ Investigation Workspace     │
             │ MCP Gateway                │
             │ Evidence API               │
             │ Event Bus                  │
             │ Policy Engine              │
             │ Audit                      │
             │ Component Manager           │
             └─────────────┬──────────────┘
                           │
             ┌─────────────┼─────────────┐
             │             │             │
          Sensors       Evidence       Memory
             │             │             │
        Search/Crawl    PostgreSQL      Neo4j
        OSINT tools     Raw Evidence    Qdrant
                                         Redis
             │
             ▼
      OPNsense/OpenWrt
             │
          INTERNET

78. 最终产品定义
本系统最终不是：
Aleph + Docker
也不是：
Crucix + Neo4j + Qdrant + Grafana
而是：
一个可以被多个 AI Agent 共同访问的共享情报基础设施，以及一个把搜索、传感器、Evidence、Graph、Semantic Memory、Agent Findings、实时告警和系统状态统一到一个工作空间中的 Intelligence Hub。
最终必须贯彻：
ONE INTELLIGENCE HUB
MANY AGENTS
SHARED EVIDENCE
SHARED MEMORY
SHARED GRAPH
UNIFIED UI
STANDARD MCP
LOCAL DATA
REMOTE COGNITION
DOCKER-FIRST
NATIVE-EXCEPTION
REPRODUCIBLE DEPLOYMENT
VERSIONED COMPONENTS
UPGRADE / ROLLBACK
核心原则：
Agents are replaceable.
Evidence is persistent.
Memory is shared.
Sensors are modular.
Cognition is external.
MCP is the interoperability boundary.
Docker provides isolation.
Native deployment is an explicit exception.
Intelligence Hub is the unified operational interface.
The Intelligence Hub must survive the replacement of any single Agent.
最终用户体验必须达到：
用户不需要理解底层使用的是 Neo4j、Qdrant、PostgreSQL、Crucix、SpiderFoot 还是 Crawl4AI；用户只需要围绕一个情报目标工作，Intelligence Hub 负责把相关 Evidence、Entities、Relationships、Timeline、Semantic Memory、Agent Findings、Alerts 和系统状态汇聚到同一个工作空间。
最终工程目标：
不是构建一个“装满 OSINT 软件的 VM”，而是构建一个可维护、可升级、可回滚、可审计、可被多个 Agent 共享调用的本地 Intelligence Substrate。
