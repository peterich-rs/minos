# D04 · Mobile Cloud Path

| Field | Value |
|-------|--------|
| Domain ID | D04 |
| Status | Refined (2026-07-31) |
| L0 | Mobile role in [long-term spec](../2026-07-30-cloud-identity-clients-long-term.md) |
| Tasks | `T-mob-*` in [tasks/TASKS.md](tasks/TASKS.md) |
| Depends on | **D01** exchange; **D02** host link（Mobile 不再需要 QR） |
| Blocks | Remote UX completeness with D02 + D05 |

---

## 1. Goal

Mobile 保持 **native remote client**：

- 同一 **account**（via D01 exchange）
- 查看 **Linked hosts** 和 **projected** sessions（D02 + D05）
- **不**采用 Desktop multi-pane / shadcn density
- Golden path 优先；隐藏或移除半成品功能
- **移除 QR pairing 代码**（`pairing/` feature directory + FRB bindings）

---

## 2. Decisions (locked)

| # | Decision |
|---|----------|
| 1 | Flutter UI 保留；只共享 protocol/semantics（不共享 React 代码） |
| 2 | Auth via Supabase → Minos exchange（password transitional） |
| 3 | **QR 完全移除**（不再是 happy path 也不是辅助路径） |
| 4 | Scope control：login → hosts → session stream → send → critical approvals |
| 5 | `minos-pairing` crate 依赖移除（FRB bindings 更新） |

---

## 3. Golden path (Desktop ↔ Mobile 联通)

```text
1. 打开 Mobile
2. 登录（Google 或 email via Supabase → exchange → Minos session）
3. 查看 linked host(s)——Desktop 已经 Link 了 Mac
4. 打开一个 agent session（或 conversation projection）
5. 观察流式 assistant 输出
6. 发送后续消息
7. (Stretch) 响应 approval（如果 MVP 需要）
```

**关键**：用户**不需要**在手机上做任何配对操作。只要 Desktop 端 Link 了 Mac，Mobile 登录同一 account 就能看到。

---

## 4. Alignment with CloudPort semantics

Dart repositories 应该 mirror CloudPort capabilities（与 Web 一致）：

| Capability | Mobile (Dart) | Web (CloudPort) |
|------------|---------------|-----------------|
| Exchange / login | `supabase_flutter` → exchange | `@supabase/supabase-js` → exchange |
| Refresh / logout | Minos refresh + Supabase signOut | 同 |
| List hosts | `GET /v1/hosts` | 同 |
| List/read sessions | `GET /v1/agent-sessions/*` | 同 |
| WS client events | `/ws/client` via `MobileClient` | `/ws/client` via `RelaySocket` |
| Host commands | `sendHostCommand` via WS forward | 同 |

不共享 React 代码。

---

## 5. QR 代码移除清单（Mobile）

| 文件/模块 | 处理 |
|-----------|------|
| `apps/mobile/lib/features/pairing/` | **删除** |
| `minos-pairing` 的 FRB bindings | **删除**（重新生成 FRB bindings） |
| `crates/minos-mobile` 中对 `minos-pairing` 的依赖 | **移除** |
| QR camera scanner 权限 | **移除**（`pubspec.yaml`） |
| Mobile UI 中的 "Pair new Mac" 入口 | 替换为 "Hosts"（`GET /v1/hosts`） |

---

## 6. Debt policy

- 非 golden path 功能：**隐藏在 flag 后或从导航移除**，直到稳定
- 优先修复 cloud contract bug，而不是添加新的 social/tasks chrome
- `social/`、`projects/`、`shell/` feature directories：暂时保留代码但从主导航隐藏，等 golden path 稳定后再决定

---

## 7. Exit criteria

- [ ] Exchange login 在设备上对 prod/staging hub 工作
- [ ] Host list 反映 D02 Linked hosts（`GET /v1/hosts`）
- [ ] Stream + send 对一个真实 session 工作
- [ ] Dual-session logout 不残留 Minos session
- [ ] QR pairing 代码全部移除（feature + FRB bindings）
- [ ] `dart analyze` 绿

---

## 8. Task slice

`T-mob-01` … `T-mob-08` + `T-cleanup-05`（mobile pairing feature 移除）in [tasks/TASKS.md](tasks/TASKS.md).
