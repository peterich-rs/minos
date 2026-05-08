# OpenWire Login Trace Bug Report

日期: 2026-05-06

范围: Android `minos-mobile` 登录请求 `POST /v1/auth/login`

## 现象

- 同一个 `call_id=1` 下，`route_plan` / `connect_race_start` 发生了两次，且两次都只解析出一个 IPv4 地址 `198.18.0.17:443`。
- 第一次建链已经出现 `connect complete` 和 `tls start`，但没有看到对应的 `tls complete` 或 `tls failed`。
- 第二次建链后，最终错误为 `connect: no fast-fallback route could be connected`，`establishment_stage=RouteExhausted`。

## 新 trace 复现结果

- 已用补丁后的 `minos-mobile` trace 在 Android 真机上重新复现。
- 第一次建链显示为 `dial_attempt=1`，并按顺序出现 `dns start`、`dns end`、`route plan`、`connect complete`、`tls start`。
- 在第一次 `tls start` 之后，新的 trace 明确出现了 `openwire retry scheduled`，且 `reason=connect`，随后才开始 `dial_attempt=2` 的第二次 DNS / connect / TLS。
- 第一次建链期间依然没有出现 `tls_failed`、`tls complete`、`connect_race_lost`。
- 这说明第二次连接已经不是“看起来像重试”，而是被源码里的 follow-up retry 真实触发了一次，并且 retry policy 看到的是一个 connect 类错误，而不是 TLS 类错误。

## 已确认的源码事实

- `race_id` 由 `FastFallbackDialer::dial_route_plan()` 每次调用时递增生成。相同 `call_id` 下从 `race_id=1` 变成 `race_id=2`，说明同一个逻辑调用里至少重新进入了两次 connect planning。
- `openwire` 默认重试策略会对可重试的 DNS / TCP / TLS 建链错误自动重试一次。默认值是 `retry_on_connection_failure = true` 且 `max_retries = 1`。
- follow-up policy 在决定重试时会调用 `ctx.listener().retry(&ctx, retries, reason)`，然后重建请求并再次进入网络链路。
- 对于正常返回的 Rustls TLS 握手错误，`RustlsTlsConnector::connect()` 会在 `tls_start` 之后显式发出 `tls_failed`；握手成功则会发出 `tls_end`。
- `openwire` 自带集成测试已经覆盖了 TLS 失败后继续 fallback 的场景，预期事件里应出现 `connect_race_lost ... reason=tls_failed`。
- 当前 workspace 使用的是 vendored `third_party/openwire`，且 `openwire` 默认 feature 仍包含 `tls-rustls` 与 `platform-verifier`；`minos-mobile` 没有显式替换 TLS connector，所以 Android Rust 侧确实会走 `rustls-platform-verifier` 做证书校验。
- 但这条链路的 TLS 引擎仍然是 `tokio-rustls` / `rustls`，不是 Android 原生 TLS socket 实现；这里“走平台”只体现在证书校验器，不体现在 TLS 实现本身。
- `rustls-platform-verifier` 在 Android 上除了 cargo feature 之外，还要求两件额外集成：把 `rustls-platform-verifier-android` 的 AAR 打进 APK，以及在发起网络请求前调用 `rustls_platform_verifier::android::init_with_env(...)` 完成运行时初始化。
- 本仓库在本次修复前缺少上述两件 Android 集成，因此虽然源码层面声明了 `platform-verifier`，但运行时并没有形成完整可用的 Android verifier 路径。

## 当前结论

- “为什么只有一个 IP 却连了两次” 现在已经可以定性：不是 DNS 返回了两个地址，也不是 fast-fallback 在单 route 上自发重复连接，而是第一次建链失败后，`openwire` follow-up retry 又重跑了一次完整建链。
- 新日志里的 `reason=connect` 说明 retry policy 接收到的不是 TLS 类错误，而是 connect 类建链错误。
- 第一次建链虽然已经进入 `tls_start`，但并没有走到 `RustlsTlsConnector::connect()` 的常规失败上报路径，否则应看到 `tls_failed`。
- 结合 `FastFallbackDialer::dial_route_plan()` 的尾部分支，可以推断第一次建链更像是 route task 在 `tls_start` 之后没有把 `Finished { result }` 回传给接收端，最终让 dialer 合成了 `route_exhausted("no fast-fallback route could be connected")` 这类 connect 错误，再被 retry policy 当成可重试 connect failure 处理。
- 这使问题的重心从“为什么 retry”转成了“为什么 route task 会在 TLS 已启动后，没有产出 `tls_failed` / `connect_race_lost` / `Finished(result)`”。
- 对“是不是因为没走平台 TLS”这个问题，当前可以更准确地回答为：Android Rust 侧确实配置了 platform verifier，但 TLS 栈本身仍是 Rustls，不是 Android 原生 TLS；更关键的真实缺口是 Android verifier 的运行时初始化和 AAR 打包之前都没有接上。
- 结合 `rustls-platform-verifier` Android 代码里的 `global().expect(...)`、原始现象中的 `tls_start` 后静默消失，以及新的库层 route-task drop 诊断，当前最强根因候选是：第一次握手在进入 verifier 路径后因为 Android verifier 未初始化或其桥接未就绪而异常退出，随后被 fast-fallback 合成为 connect 类错误并触发 retry。

## 这次已落地的改动

- `minos-mobile` 的 `openwire_trace` 已补充 `retry` / `redirect` 事件日志。
- `minos-mobile` 的 `openwire_trace` 已为每次 DNS 开始分配本地 `dial_attempt` 序号，并把 `dial_attempt`、`retry_count`、`redirect_count` 带入 DNS / route / connect / TLS / response / error 日志。
- workspace 依赖已从 git checkout 切换为 vendored `third_party/openwire` path 依赖，后续可以直接在仓库内修改和验证 `openwire` 源码。
- `crates/minos-ffi-frb` 已新增 Android JNI 导出，用于在 App 启动时调用 `rustls_platform_verifier::android::init_with_env(...)`。
- `apps/mobile/android/app/src/main/kotlin/minos/ai/android/MainActivity.kt` 已在 `onCreate()` 中加载 `minos_ffi_frb` 并执行 verifier 初始化；真机冷启动日志已确认出现 `MinosMainActivity: initialized rustls-platform-verifier`。
- `apps/mobile/android/app/build.gradle.kts` 已补上 `rustls-platform-verifier-android` 对应的本地 Maven/AAR 依赖接入，并已通过 `./gradlew :app:assembleDebug` 验证整个 Android 打包链路可用。
- vendored `third_party/openwire` 已新增两类库层诊断：`openwire-rustls` 会记录当前使用的是 `platform-verifier` 还是 `native-root-store`；`fast_fallback` 会记录 route task drop / abort / receiver-closed 等生命周期异常。

## 仍然存在的歧义

- 第一次 `tls_start` 之后究竟是 TLS future 被取消、panic、短路退出，还是某个上层 future 提前结束，目前 app 侧日志还无法分辨。
- `dial_route_plan()` 会在接收端拿不到任何 `Finished` 结果时合成 `route_exhausted`，但当前 app 侧 trace 看不到 route task 为什么没有回报结果。
- 这已经超出 app 侧 listener 能观测到的边界；若要继续收敛，需要在 vendored `third_party/openwire` 里给 route task 生命周期和 TLS future 取消路径补更细事件。

## 建议的下一步验证

- 现在 Android verifier 初始化和 AAR 打包已经补上，下一步最值得做的是在真机上再走一次登录，确认原先的 `tls_start` 后静默消失是否已经消失。
- 复现时优先关注三类新证据：`MinosMainActivity` 的初始化日志、`openwire::tls` 的 `verifier_backend=platform-verifier` 日志、以及 `fast_fallback` 的 route-task drop / abort 日志。
- 如果补完 Android verifier 集成后问题直接消失，说明前一次异常大概率就是 verifier 路径未初始化导致；如果问题仍在，则新的 vendored `openwire` 诊断足以继续把范围收紧到 route task / TLS future 生命周期。