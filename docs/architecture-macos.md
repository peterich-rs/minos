# macOS 应用 (apps/macos) 架构文档

> 本文档详细描述 macOS 状态栏应用的架构。

## 概述

Minos macOS 应用是一个状态栏（menu bar）应用，通过 SwiftUI + UniFFI 绑定与 Rust daemon 交互。它作为 Host 设备，管理本地 AI agent 并允许远程控制。

**源码路径**: `apps/macos/`

## 项目结构

```
Minos/
  MinosApp.swift                  # @main 入口
  Application/                    # 应用级状态 & 编排
    AppState.swift                # 中央 @Observable 状态
    AppState+Actions.swift        # 用户操作
    AppState+Agent.swift          # Agent 运行时
    AgentStateObserverAdapter.swift
    DaemonDriving.swift           # Daemon 协议抽象
    PeerObserver.swift
    RelayLinkObserver.swift
  Domain/                         # 域扩展
    MinosError+Display.swift      # 错误本地化（中/英）
    PeerState+Display.swift       # 配对状态展示
    RelayLinkState+Display.swift  # Relay 状态展示
  Infrastructure/                 # 平台服务
    DaemonBootstrap.swift         # 生产引导
    DaemonHandle+DaemonDriving.swift
    DiagnosticsReveal.swift       # Finder 中打开日志
    QRCodeRenderer.swift          # CoreImage QR 渲染
  Presentation/                   # SwiftUI 视图
    MenuBarView.swift             # 主弹出视图
    AgentSegmentView.swift        # Agent 运行时子视图
    PairingQRView.swift           # QR 配对流程
    StatusIcon.swift              # 菜单栏图标
  Generated/                      # UniFFI 自动生成（13 文件）
  Resources/                      # 资源文件
```

## 构建系统

### XcodeGen (`project.yml`)

1. **Pre-build**: 调用 `just` → `cargo xtask gen-uniffi`（生成 Swift/头文件）+ `cargo xtask build-macos`（编译 Rust 静态库）
2. **链接**: 链接 `libminos_ffi_uniffi` 静态库 + `SystemConfiguration` 框架
3. **Module map**: `MinosCoreFFI.modulemap` 让 Swift 导入 UniFFI 类型
4. **Post-build**: 通过 `just _patch-macos-info-plist` 注入环境变量到 `Info.plist`

**配置**: macOS 14.0 部署目标, Swift 5.10, 严格并发, 无代码签名

### UniFFI 生成文件

| 文件 | 来源 Crate |
|------|-----------|
| `minos_agent_runtime.swift` | Agent 线程生命周期 |
| `minos_daemon.swift` | DaemonHandle, relay config, subscriptions |
| `minos_domain.swift` | PeerState, RelayLinkState, MinosError, DeviceId |
| `minos_pairing.swift` | 配对流类型 |
| `minos_protocol.swift` | 线协议类型 |
| `MinosCore.swift` | 顶层伞形模块 |

## 应用层

### `AppState` (`Application/AppState.swift`)

`@Observable` 中央状态对象。管理:

- **Phase**: `booting` / `running` / `bootFailed`
- **双轴状态**: relay link + peer
- **配对 UX**: QR 生成/刷新
- **Agent 运行时**: currentSession, thread snapshots
- **错误显示**: 3 秒自动消失

### `DaemonDriving` 协议

Rust daemon 的 Swift 侧抽象。定义:
- 双轴状态查询（relay link, peer）
- 配对往返（QR 生成）
- 生命周期（stop）
- Agent 运行时方法
- Push-model observer 订阅

测试注入 `MockDaemon`。

### Bootstrap (`Infrastructure/DaemonBootstrap.swift`)

1. 从 Info.plist 读取 backend URL
2. 加载/迁移 `local-state.json`
3. 通过 UniFFI 生成 `DaemonHandle`
4. 连接三个 observer（relay link, peer, agent）
5. 调用 `appState.finishBoot()`
6. 日志清理、本地状态迁移

## 表示层

### `MenuBarView`

主弹出视图（360px 宽），按 `Phase` 梯度渲染:
- `booting`: 旋转器
- `bootFailed`: 错误详情 + 重试
- `running`: 头部 + 已配对设备列表 + QR + Agent 段 + 操作

### `StatusIcon`

菜单栏 SF Symbol 图标。矩阵:
- Relay Connected + Peer Online → 绿色闪电
- Relay Disconnected → 红色闪电斜线
- 其他组合有对应图标和颜色

### `PairingQRView`

完整 QR 配对流程:
- QR 图像 + 5 分钟倒计时
- "重新生成" 按钮
- 返回导航

### `AgentSegmentView`

Agent 运行时子视图:
- 当前 agent 状态
- 会话信息（session ID, workspace 路径）
- Debug 控制（启动/停止 Codex agent）

## App 生命周期

`MinosApp.swift` (`@main`):
- `LSUIElement: true`（无 Dock 图标，纯 menu bar）
- 创建 `AppState`，连接 `AppTerminationController`
- 初始化时调用 `DaemonBootstrap.bootstrap()` 启动 Rust daemon
- 终止时 `applicationShouldTerminate` → `appState.shutdownForTermination()`

## 测试

| 文件 | 焦点 |
|------|------|
| `MockDaemon.swift` | Mock `DaemonDriving` 协议 |
| `AppStateTests.swift` | 状态管理 |
| `AppStateBootTests.swift` | 启动阶段转换 |
| `AgentStateTests.swift` | Agent 运行时状态机 |
| `StartupLogCleanerTests.swift` | 日志目录清理 |
| `QRCodeRendererTests.swift` | QR 码生成 |

## 与系统的连接

macOS 应用是 Minos 系统中的 **Host 设备**:
- 通过 UniFFI 内嵌 Rust daemon，管理到后端 relay 的 WebSocket 连接
- 展示配对 QR 码供移动设备配对
- 可在 Mac 上启动和管理 AI agent 运行时
- Web 应用连接同一后端，可远程控制此 Mac host
