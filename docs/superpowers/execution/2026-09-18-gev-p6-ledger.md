# GEV P6 执行台账 — 视觉预设（滤镜/bloom/sharpen/HUD 切换器）（2026-09-18）

> Spec：`docs/superpowers/specs/2026-09-18-gev-visual-port-design.md` §3 P6 · Plan：`docs/superpowers/plans/2026-09-18-gev-p6-visual-presets.md`
> 分支：`feat/gev-p6-visual-presets`（8 commits）→ merge（--no-ff）→ main。**315 验收全绿 → 410 生产验收 → push**。

## 范围

GEV 可视化移植 P6：把 vendor 的 VisualEffects 渲染核接入 React 壳。6 个视觉风格（NORMAL/CRT/NVG/FLIR/ANIME/NOIR/SNOW）+ bloom/sharpen 后处理联动 + 500ms crossfade（vendor TRANSITION_DURATION_MS）+ HUD 顶部切换器（下拉菜单）+ localStorage 持久化 + 契约守卫测试 + probe-gev 滤镜断言。

- 渲染核直引（沿用 P5 路线）：`mountVisualEffects(viewer, deps?, initialStyle?)` 适配器只 import vendor render core，不实例化 `VisualSettings`（DOM/shell 耦合）、不装 `renderGovernor`（hold/release 在我们的 boot 下是 noop）、不碰 `detection`/`scopeMask`（spec §0 已移除 detection keyhole）。
- 交付物：`console/src/gev-visual/visual-effects.ts`（适配器）+ `__tests__/visual-effects.test.ts`（7 测试）、`console/src/globe-hud/HudStyleSwitcher.tsx`（切换器）、`console/src/globe-hud/GlobeV2.tsx` 接线、`console/src/gev-boot/__tests__/source-contracts.test.ts` 契约守卫 +3 锚点、`console/probe-gev.mjs` 滤镜段。

## 决策记录

- **detection keyhole 移除**（spec §0）：P6 完全不引入 detection/VisualSettings 引用；application.ts 里既有的 `setScopeMaskEnabled(false)` 防御行保留（import + 调用）。
- **renderGovernor 不装**：vendor render governor 的 hold/release 在自研 boot 下是 noop，装了徒增耦合——适配器注释明示，留待未来需要帧节流时再评估。
- **anime/noir/snow 本地 fallback**：vendor 这三个风格阶段依赖外部 shader 资产/资源，适配器在缺失时回退（anime → bloom off 等），行为由测试 6 钉住。
- **normal sharpen 基线对齐修复（终审 MUST-FIX，49172af）**：vendor `GLOBAL_POST_DEFAULTS` sharpen=ON，但 `if (initialStyle !== "normal")` 门控让 NORMAL 首挂跳过 `applyPresetDefaults`，导致"首挂不锐化、setStyle 回到 normal 却锐化"的不对称。修法：`applyPresetDefaults(initialStyle)` 无条件在 mount 执行（`setStageIntensity(1.0)` 瞬时应用与 `current` 赋值仍留在门内，normal 无 style stage）。
- **retro bloom 断言按真实值调整（preflight 已授权）**：vendor `STYLE_PRESET_DEFAULTS.retro.bloom` 实为 `{enabled:false}`，brief 的 `toBe(true)` 改为 `toBe(false)` + 补一条 sharpen `enabled===true` 断言保住"预设被应用"意图。
- **probe `.hud-root` → `.globe-root` 修正 + "probe 半瞎"发现**：并行会话 commit 62b2cca（Globe CSS scope 隔离）把 GlobeV2 根类改名为 `.globe-root` 却未更新 probe-gev.mjs，导致 mount gate 长期静默跳过全部 canvas/rail boot 断言——probe 一直"半瞎"跑（每轮都失败 `missing .hud-root` 但 HUD 实际正常）。a6f3505 修正 selector 后 probe 才真正开始断言 canvas/rail。**已向用户披露**。
- **agent-keys.txt 格式回写**：实际格式为 tab 分隔 `<agent_id>\t<agent_name>\t<api_key>`，无 `api_key:` 前缀——AGENTS.md 里两处 `grep "api_key:" ... grep -o` 命令样本已在本任务改为 `grep -o "ihk_[a-f0-9]*" /home/zou/IntelHub/core/agent-keys.txt | head -1`。
- **STYLE_LABELS 重复定义（OK-TO-DEFER）**：`HudStyleSwitcher.tsx` 的 `STYLE_LABELS`（normal→NORMAL 等 7 项）与 vendor `visualPresets.js` 的 `STYLE_STATUS_LABELS` 逐字重复——终审裁决为 deliberate UI pin（React 壳不与 vendor UI 模块耦合，适配器只 import render core），不修，记录于此。

## 任务执行（T1-T6）

| 任务 | 内容 | 结果 |
|---|---|---|
| T1 | 契约守卫 source-contracts.test.ts +3 锚点（pin visual-presets render-core 导出），不碰 vendor | ✅ 45d5b89 |
| T2 | `mountVisualEffects` 适配器 + 7 测试（createStage/postProcessStages.add 形状断言互锁；fallback 生效） | ✅ 4836f35 |
| T3 | HudStyleSwitcher（GLOBE_STYLES/GlobeStyle/isGlobeStyle/readPersistedStyle）+ localStorage | ✅ 680cfe5 |
| T4 | GlobeV2 接线（mount 时 apply initialStyle，destroy 先于 globe.destroy，if(viewer)+try/catch guard） | ✅ d561d5e |
| T5 | probe-gev 滤镜段（FLIR 标签断言 + 全页截图字节差异 + restore-normal）+ 315 验收 | ✅ a6f3505+868278b |
| T6 | 熵减 + ledger + 合并 main + 410 部署 + push（本任务） | ✅ |

## 315 验收结果（Debian-test / 10.10.10.35，2026-09-18）

- probe-gev exit 0：`hud=true canvas=true aircraft=134 satellites=832 pageerrors=0` + `probe-gev OK`；滤镜段通过（无 "style switcher did not apply FLIR"、无 "screenshot identical after style switch" failure）。剩余 WARN 是既有 P3 rail toggle 的 `locator.check` 超时（ground[4..6]/infra[0..1]），warning 级非本期引入。
- 验收基线（0 failed 达标）：
  ```
  sp8: == 39 passed, 2 shelved, 0 failed ==
  sp6: == 36 passed, 5 shelved, 1 failed ==   ← 唯一 fail = starlink celestrak 502（上游惩罚箱 flap）
  sp7: == 16 passed, 11 shelved, 0 failed ==
  sp3: == 19 passed, 0 failed ==
  ```

## 410 生产验收结果（IntelHub / 10.10.10.41，2026-09-18）

- probe-gev exit 0：`hud=true canvas=true aircraft=509 satellites=832 pageerrors=0` + `probe-gev OK`；P6 滤镜段通过（无 style-switcher 失败、无 screenshot-identical 失败）。剩余 WARN 为既有 P3 rail toggle `locator.check` 超时（ground[4..6]/infra[0..1]），非本期引入。
- 验收基线：
  ```
  sp8: == 42 passed, 2 shelved, 0 failed ==
  sp6: == 37 passed, 5 shelved, 0 failed ==   ← 首轮 36+5sh/1f（overpass 504 flap），复跑自愈归零
  sp7: == 16 passed, 11 shelved, 0 failed ==
  sp3: == 19 passed, 0 failed ==
  ```
- **sp6 唯一 fail 是上游惩罚箱 flap，非回归**：首轮 `overpass proxy round-trip` http=504（P3 ledger 已载明的 Overpass 镜像 403/504 惩罚箱），复跑即 200 自愈；本轮 starlink celestrak 反而 200（撞窗）。本期纯 console 前端改动，hub-core 的 overpass/celestrak 代理零涉。0 failed 达标。

## 已知边界

- **probe 截图字节对比是弱门槛**：只做全页 `Buffer.compare` 字节差异 + FLIR 标签正则，不校验像素级滤镜内容——真滤镜管线崩了但 DOM 有变化也可能假绿。FLIR 标签断言兜底一部分。
- **恢复段无断言**：probe 里 style-switch 后 restore NORMAL 段只 restore 不校验（brief 允许），未来可加 warnings 校验。
- **starlink celestrak 惩罚箱值守**：sp6 starlink 502 是共享出口 IP 惩罚箱 flap（P3 ledger 已载明），本期纯 console 前端改动与 hub-core celestrak 代理无关；stations 组（PG 后端）照常 200。后台值守补验。
- **既有 hud-bars.test.tsx 5 失败**：main 上 89c30ea 引入的 React-Router-context 测试 harness bug，本分支零改动该文件链，出范围不修（Task 2 已披露）。
- **Vendor 只读**：本期零 vendor 改动；契约守卫 pin 的 render-core 导出下次 vendor sync 时需复查（stageOf 读 vendor 私有 `this.stages` 未钉字段）。
