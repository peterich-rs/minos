# Grok ACP → Minos UI 投影清单

> 对照 grok-build `acp_conversion.rs` + pager `acp/tracker.rs`。  
> Codex app-server 的 tool item 多为「人可读」正文；Grok ACP 是 **双通道**：
> `content`（常给模型/协议）+ `raw_output`（typed `ToolOutput`，pager 用来渲染）。

Minos 投影入口：`minos-ui-protocol::grok::extract_tool_result_text`  
策略：**先结构化 `raw_output`，再 ACP `content`，禁止整坨 JSON dump**；出口统一 strip ANSI。

## 工具 × 线格式 × 投影

| ToolOutput | ACP `content`（噪音/形态） | `raw_output` 关键字段 | Minos 投影结果 | 状态 |
|------------|---------------------------|---------------------|----------------|------|
| **ReadFile::FileContent** | `N→line` 稀疏行号（首行+每10行） | `raw_output` 纯文本 + `offset`/`total_lines` | 纯文本 densify 为 `{offset+1+i}→line`（gutter 用） | ✅ |
| **ReadFile** 错误 | 错误文案 | 同左 | 错误文案 | ✅ |
| **ReadFile** Image/PDF | Image blocks | 结构化 | 空正文（见下方 backlog） | 📋 高优 backlog |
| **SearchReplace::EditsApplied** | `type:diff` old/new + `_meta.details` | `EditsApplied{…}` | **unified patch** | ✅ |
| **SearchReplace** 错误 | 错误文案 | 错误变体 | 错误文案 | ✅ |
| **ApplyPatch::Success** | 每文件一个 Diff | `Success.files[]` | 多文件 unified patch | ✅ |
| **Bash** | **原始字节 + ANSI** | `output_for_prompt`（已 strip） | 优先 `output_for_prompt`；再 strip ANSI | ✅ |
| **GrepSearch** | 仅 `"found N matches"` | `file_matches` / `stdout` | `path:line:content` 或 strip `<workspace_result>` | ✅ |
| **ListDir** | **无 content** | `Content.content` 列表字符串 | 目录列表正文 | ✅ |
| **WebSearch** | 无 | `content` + `citations` | 正文 + 引用列表 | ✅ |
| **WebFetch** | `to_prompt_format()` 文本 | 同结构化 | Content / 错误文案 | ✅ |
| **Todo** | 无（另发 Plan） | todos | 成功无 body；错误文案；timeline 抑制 plumbing | ✅ 抑制 |
| **MCP** | 无 | `output: OkayOutput/Error` | 文本；JSON pretty | ✅ |
| **BackgroundTaskStarted** | summary | summary / task_id | summary；title `[bg] cmd` | ✅ |
| **TaskOutput / KillTask** | 无 | structured | title/摘要；plumbing 多半抑制 | ⚠️ 抑制路径为主 |
| **Skill** | `tool_result` | 同左 | tool_result 文本 | ✅ |
| **SearchTool / Text / Dynamic** | text | content/text/value | 文本 / pretty JSON | ✅ |
| **CodexGrepFiles** | 无 | Matches.content | 路径列表 | ✅ |
| **ImageGen / Video / ImageEdit** | `to_prompt_format()` 散文 | path / uploaded_url | `saved:` / `uploaded:` | ✅ |
| **SubagentCompleted** | `to_model_text()` | structured | 模型文案（含 resume 提示） | 📋 backlog（有价值，后做） |
| **Plan enter/exit / AskUser** | message | message | 文案；审批走 ext reverse-request | ✅ 审批路径 |
| **UpdateGoal / Monitor / Scheduler*** | 无 | structured | 常无 body；抑制 or Raw | ⚠️ 低优先级 |

## 展示层（TUI / Desktop）配合

| 形态 | 处理 |
|------|------|
| unified patch | `isDiffLike` + DiffView / `render_tool_diff` |
| densify `N→` read body | ReadView / `render_tool_read_body` 解析为 gutter |
| ANSI | 投影层 strip；TUI/Desktop 再 strip 一次（历史帧） |
| ListDir / Grep 正文 | 普通 preformatted tool body |

## 后续 backlog（已记录，非本次实现）

### 高优：Image / PDF 媒体工具

- **现状**：ACP `content` 可带 Image blocks；`raw_output` 有 `ImageContent` / `PdfPageImages`；投影目前返回空正文，避免 dump JSON。
- **目标**：Desktop/TUI transcript 展示缩略图 / PDF 页预览（路径或 data URL），Summary 可记 “1 image read”。
- **触达路径**：`ReadFile` 读图、Grok `image_gen` / `image_edit` / video 工具（path + session_folder）。
- **用户价值高**：截图、设计稿、报错图是常见输入；不做就“工具完成了但什么都看不见”。

### 中优：SubagentCompleted

- **现状**：`to_model_text()` 偏模型（含 resume handle 长文案）。
- **目标**：短摘要（子代理 id + 状态 + 一行结论），详情可展开；与 Minos `SubagentSpawned` / status 事件对齐。
- **暂缓原因**：不阻塞主会话委托路径；先保证 MCP teamwork 可用。

### 其它

3. **TaskOutput 完整 log**：体量大，继续走抑制 + 按需展开。  
4. **Codex 对齐**：Codex 本身已是干净 item 文本；本清单只服务 Grok 双通道。

## 回归锚点（单测）

- `search_replace_*` / `apply_patch_*` → patch，不 dump EditsApplied  
- `read_file_prefers_raw_output_over_arrow_content`  
- `list_dir_projects_listing_from_raw_output`  
- `grep_projects_file_matches_not_stub_count`  
- `bash_prefers_output_for_prompt` + ANSI strip  
- `never_dumps_typed_tool_output_json_objects`  
